use nostr_sdk::Timestamp;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use anyhow::{Context, Result};

mod boostboard;
mod boosts;
mod config;
mod nwc;
mod osc;
mod artnet;
mod sacn;
mod wled;
mod zaps;
mod gui;
mod sat_tracker;
mod revert;
mod actions;

use gui::{ComponentStatus, GuiMessage};

/// The live config. A reload swaps the contents, and every boost reads the current value.
type SharedConfig = Arc<std::sync::RwLock<config::Config>>;

fn snapshot(config: &SharedConfig) -> config::Config {
    config.read().unwrap().clone()
}

// ============================================================================
// Utility Functions
// ============================================================================

fn parse_timestamp(s: &str) -> Result<Timestamp> {
    s.parse::<u64>()
        .map(Timestamp::from_secs)
        .context("Failed to parse timestamp as unix seconds")
}

fn parse_load_since(load_since_str: Option<&String>, default: Timestamp) -> Timestamp {
    load_since_str
        .and_then(|s| parse_timestamp(s).ok().inspect(|_| println!("Loading since: {}", s)))
        .unwrap_or(default)
}

// ============================================================================
// Effects Setup & Triggering
// ============================================================================

async fn setup_effects(config: config::Config) -> Result<()> {
    for (name, target) in &config.targets {
        let config::Target::Wled(cfg) = &target.kind else { continue };
        if !target.enabled || !cfg.setup { continue };

        println!("Setting up WLED target '{}'", name);
        let mut wled = wled::WLed::new();
        wled.load(&cfg.host).await
            .with_context(|| format!("Unable to load WLED target '{}'", name))?;

        if let Some(presets) = &cfg.presets {
            for (idx, preset) in presets.iter().enumerate() {
                wled.set_preset(idx, cfg, preset).await?;
            }
        }

        if let Some(playlists) = &cfg.playlists {
            for (idx, playlist) in playlists.iter().enumerate() {
                wled.set_playlist(idx, cfg, playlist).await?;
            }
        }
    }

    Ok(())
}

fn format_toggle_description(toggle: &config::Toggle) -> String {
    let mut desc = actions::describe(&toggle.actions);
    if let Some(secs) = toggle.revert_after {
        if !toggle.revert.is_empty() {
            desc.push_str(&format!(" (reverts in {}s)", secs));
        }
    }
    desc
}

async fn trigger_single_toggle(config: &config::Config, toggle: &config::Toggle) -> Result<()> {
    actions::run(config, &toggle.actions).await
}

async fn fire_toggle(config: &config::Config, toggle: &config::Toggle, triggered_effects: &mut Vec<String>, err_context: &str) {
    if let Err(e) = trigger_single_toggle(config, toggle).await {
        eprintln!("{}: {:#}", err_context, e);
    } else {
        triggered_effects.push(format_toggle_description(toggle));
        // A new look replaces the previous one, so drop any revert still waiting.
        if toggle.revert_after.is_some() && !toggle.revert.is_empty() {
            revert::schedule(config.clone(), toggle.clone()).await;
        } else {
            revert::cancel_pending().await;
        }
    }
}

async fn trigger_toggles(
    config: &config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    let Some(toggles) = &config.toggles else { return Ok(Vec::new()) };

    let last_two_digits = (sats % 100).abs() as u8;
    let mut triggered_effects = Vec::new();

    // Check threshold-based toggles
    let threshold_toggles: Vec<_> = toggles.iter()
        .filter(|t| !t.is_default && t.threshold > 0)
        .collect();

    let threshold_triggered = if !threshold_toggles.is_empty() {
        if let Some(tracker_ref) = tracker.as_ref() {
            let all_thresholds: Vec<i64> = threshold_toggles.iter().map(|t| t.threshold).collect();
            let max_threshold = *all_thresholds.iter().max().unwrap();

            let mut tracker_guard = tracker_ref.lock().await;
            let thresholds_to_trigger = tracker_guard.get_thresholds_to_trigger(sats, &all_thresholds, max_threshold);
            drop(tracker_guard);

            if let Some(&max_crossed) = thresholds_to_trigger.iter().max() {
                if thresholds_to_trigger.len() > 1 {
                    println!("Multiple thresholds crossed ({:?}), applying only maximum: {} sats", thresholds_to_trigger, max_crossed);
                } else {
                    println!("Triggering threshold: {} sats", max_crossed);
                }

                if let Some(toggle) = threshold_toggles.iter().find(|t| t.threshold == max_crossed) {
                    let should_trigger = toggle.endswith_range
                        .map_or(true, |(start, end)| {
                            let in_range = last_two_digits >= start && last_two_digits <= end;
                            if !in_range {
                                println!("Toggle skipped: {} sats threshold ends with {}, not in range {}-{}", max_crossed, last_two_digits, start, end);
                            }
                            in_range
                        });

                    if should_trigger {
                        fire_toggle(config, toggle, &mut triggered_effects, &format!("Failed to trigger toggle at {} sats", max_crossed)).await;
                    }
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // A single boost at least this large, when no running-total line was crossed.
    let mut amount_triggered = false;
    if !threshold_triggered {
        let amount_toggles: Vec<_> = toggles.iter()
            .filter(|t| !t.is_default && t.amount > 0 && sats >= t.amount)
            .collect();

        if let Some(toggle) = amount_toggles.iter().max_by_key(|t| t.amount) {
            amount_triggered = true;
            if amount_toggles.len() > 1 {
                println!("Multiple amounts matched, applying only maximum: {} sats", toggle.amount);
            } else {
                println!("Triggering amount: {} sats >= {}", sats, toggle.amount);
            }

            let should_trigger = toggle.endswith_range
                .map_or(true, |(start, end)| {
                    let in_range = last_two_digits >= start && last_two_digits <= end;
                    if !in_range {
                        println!("Toggle skipped: {} sats ends with {}, not in range {}-{}", sats, last_two_digits, start, end);
                    }
                    in_range
                });

            if should_trigger {
                fire_toggle(config, toggle, &mut triggered_effects, "Failed to trigger amount toggle").await;
            }
        }
    }

    // The boost's digits end with this value, with no amount or threshold required.
    let mut ends_with_triggered = false;
    if !threshold_triggered && !amount_triggered {
        let ending_toggles: Vec<_> = toggles.iter()
            .filter(|t| !t.is_default && t.threshold == 0 && t.amount == 0)
            .filter(|t| t.ends_with.is_some_and(|suffix| sats_end_with(sats, suffix)))
            .collect();

        if let Some(toggle) = ending_toggles.iter().max_by_key(|t| {
            let suffix = t.ends_with.unwrap_or(0);
            (suffix_digits(suffix), suffix)
        }) {
            ends_with_triggered = true;
            let suffix = toggle.ends_with.unwrap_or(0);
            if ending_toggles.len() > 1 {
                println!("Multiple endings matched, applying only the longest: {}", suffix);
            } else {
                println!("Triggering ending: {} sats ends with {}", sats, suffix);
            }
            fire_toggle(config, toggle, &mut triggered_effects, "Failed to trigger ending toggle").await;
        }
    }

    // Trigger default toggles if nothing else matched
    if !threshold_triggered && !amount_triggered && !ends_with_triggered {
        for toggle in toggles.iter().filter(|t| t.is_default) {
            let should_trigger = toggle.endswith_range
                .map_or(true, |(start, end)| {
                    let in_range = last_two_digits >= start && last_two_digits <= end;
                    if !in_range {
                        println!("Default toggle skipped: {} sats ends with {}, not in range {}-{}", sats, last_two_digits, start, end);
                    }
                    in_range
                });

            if should_trigger {
                println!("Default toggle triggered for {} sats - {}", sats, actions::describe(&toggle.actions));
                if let Err(e) = trigger_single_toggle(config, toggle).await {
                    eprintln!("Failed to trigger default toggle: {:#}", e);
                } else {
                    triggered_effects.push(format_toggle_description(toggle));
                }
            }
        }
    }

    Ok(triggered_effects)
}

/// True when the decimal digits of `sats` end with `suffix`.
/// `1` matches 1, 11, and 101. `21` matches 21 and 121, and not 1.
fn sats_end_with(sats: i64, suffix: i64) -> bool {
    if suffix < 0 {
        return false;
    }
    let sats = sats.unsigned_abs();
    let suffix = suffix as u64;
    let mut modulus = 10u64;
    let mut rest = suffix;
    while rest >= 10 {
        modulus = modulus.saturating_mul(10);
        rest /= 10;
    }
    sats % modulus == suffix
}

fn suffix_digits(suffix: i64) -> u32 {
    let mut n = suffix.unsigned_abs();
    let mut digits = 1u32;
    while n >= 10 {
        digits += 1;
        n /= 10;
    }
    digits
}

async fn trigger_effects(
    config: config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    println!("Triggering effects for {} sats", sats);
    trigger_toggles(&config, sats, tracker).await
        .inspect_err(|e| eprintln!("Failed to trigger toggles: {:#}", e))
        .or(Ok(Vec::new()))
}

// ============================================================================
// Boost Processing
// ============================================================================

async fn process_boost(
    source: &str,
    sats: i64,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>,
    config: &SharedConfig,
    trigger_effects_flag: bool
) {
    let total = tracker.lock().await.add(source, sats);
    println!("{} received: {} sats, total now: {} sats", source, sats, total);

    let _ = tx.send(GuiMessage::UpdateSatTotal(total)).await;

    let effects = if trigger_effects_flag {
        trigger_effects(snapshot(config), sats, Some(tracker.clone())).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let _ = tx.send(GuiMessage::BoostReceived(source.to_string(), sats, effects)).await;
}

async fn sync_threshold_triggers(config: &config::Config, tracker: &Arc<Mutex<sat_tracker::SatTracker>>) {
    if let Some(toggles) = &config.toggles {
        let thresholds: Vec<i64> = toggles.iter()
            .filter(|t| !t.is_default && t.threshold > 0)
            .map(|t| t.threshold)
            .collect();

        if let Some(&max_threshold) = thresholds.iter().max() {
            tracker.lock().await.sync_trigger_state(max_threshold);
        }
    }
}

// ============================================================================
// Listeners
// ============================================================================

async fn initialize_listener(component_name: &str, tx: &tokio::sync::mpsc::Sender<GuiMessage>) {
    let _ = tx.send(GuiMessage::UpdateStatus(component_name.to_string(), ComponentStatus::Running)).await;
}

async fn handle_connection_error(component: &str, error: anyhow::Error, tx: &tokio::sync::mpsc::Sender<GuiMessage>) {
    let error_msg = format!("Connection error: {:#}", error);
    eprintln!("Error connecting to {}: {}", component, error_msg);
    let _ = tx.send(GuiMessage::UpdateStatus(component.to_string(), ComponentStatus::Error(error_msg))).await;
}

async fn listen_for_zaps(
    cfg: config::Zaps,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener("Zaps", &tx).await;

    let zap = match zaps::Zaps::new(&cfg.relay_addrs, &cfg.naddr).await {
        Ok(z) => z,
        Err(e) => return handle_connection_error("Zaps", e, &tx).await,
    };

    let load_since = match cfg.load_since {
        Some(load_since_str) => match parse_timestamp(&load_since_str) {
            Ok(ts) => Some(ts),
            Err(_) => None,
        },
        None => None,
    };

    println!("Waiting for Zaps...");

    tokio::select! {
        result = zap.subscribe_zaps(load_since, |zap: zaps::Zap| {
            let (config, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            async move {
                println!("Zap: {:#?}", zap);
                process_boost("Zaps", zap.value_msat_total / 1000, &tx, &tracker, &config, !zap.is_old).await;
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling zap events: {}", error_msg);
                let _ = tx.send(GuiMessage::UpdateStatus("Zaps".to_string(), ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("Zaps listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus("Zaps".to_string(), ComponentStatus::Disabled)).await;
        }
    }
}

async fn listen_for_boostboard(
    cfg: config::BoostBoard,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener("Boostboard", &tx).await;

    if cfg.relay_addrs.is_empty() {
        eprintln!("Error: No relay addresses specified for boostboard");
        let _ = tx.send(GuiMessage::UpdateStatus("Boostboard".to_string(), ComponentStatus::Error("No relay addresses specified".to_string()))).await;
        return;
    }

    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    println!("Boostboard Filters: {:#?}", &filters);

    let board = match boostboard::BoostBoard::new(&cfg.relay_addrs, &cfg.pubkey, filters.clone()).await {
        Ok(b) => b,
        Err(e) => return handle_connection_error("Boostboard", e, &tx).await,
    };

    let load_since = Some(parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now()));

    // Load stored boosts
    println!("Loading stored boosts from API...");
    let stored_boosts = boostboard::StoredBoosts::new(filters);
    let _ = stored_boosts.load(|boost: boosts::Boostagram| {
        let (tx, tracker, config) = (tx.clone(), tracker.clone(), config.clone());
        async move {
            if boost.action == "boost" {
                process_boost("Boostboard", boost.sats, &tx, &tracker, &config, false).await;
            }
        }
    }).await;

    sync_threshold_triggers(&snapshot(&config), &tracker).await;

    let subscription_id = match board.subscribe(load_since).await {
        Ok(id) => id,
        Err(e) => {
            let error_msg = format!("Subscription error: {:#}", e);
            eprintln!("Error subscribing to board: {}", error_msg);
            let _ = tx.send(GuiMessage::UpdateStatus("Boostboard".to_string(), ComponentStatus::Error(error_msg))).await;
            return;
        }
    };

    println!("Waiting for Boostboard boosts...");
    let subscription_start_time = Timestamp::now();
    let tx_clone = tx.clone();

    tokio::select! {
        result = board.handle_boosts(subscription_id, move |boost: boosts::Boostagram, event_ts: Timestamp| {
            let (config, tx, tracker) = (config.clone(), tx_clone.clone(), tracker.clone());
            async move {
                if boost.action == "boost" {
                    println!("Boost: {:#?}", boost);
                    let trigger = event_ts >= subscription_start_time;
                    process_boost("Boostboard", boost.sats, &tx, &tracker, &config, trigger).await;
                }
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling boostboard events: {}", error_msg);
                let _ = tx.send(GuiMessage::UpdateStatus("Boostboard".to_string(), ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("Boostboard listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus("Boostboard".to_string(), ComponentStatus::Disabled)).await;
        }
    }
}

async fn listen_for_nwc(
    cfg: config::NWC,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener("NWC", &tx).await;

    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    println!("NWC Filters: {:#?}", &filters);

    let nwc = match nwc::NWC::new(&cfg.uri, filters).await {
        Ok(n) => n,
        Err(e) => return handle_connection_error("NWC", e, &tx).await,
    };

    let load_since = parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now());

    println!("Loading previous boosts from NWC...");
    let latest_boost_timestamp = nwc.load_previous_boosts(Some(load_since), |boost: boosts::Boostagram| {
        let (tx, tracker, config) = (tx.clone(), tracker.clone(), config.clone());
        async move {
            process_boost("NWC", boost.sats, &tx, &tracker, &config, false).await;
        }
    }).await.unwrap_or(None);

    sync_threshold_triggers(&snapshot(&config), &tracker).await;

    let subscription_start = latest_boost_timestamp.map(|ts| ts + 1).unwrap_or(load_since);
    println!("Waiting for NWC boosts...");

    tokio::select! {
        result = nwc.subscribe_boosts(subscription_start, |boost: boosts::Boostagram| {
            let (config, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            async move {
                if boost.action == "boost" {
                    println!("NWC Boost: {:#?}", boost);
                    process_boost("NWC", boost.sats, &tx, &tracker, &config, true).await;
                }
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling NWC events: {}", error_msg);
                let _ = tx.send(GuiMessage::UpdateStatus("NWC".to_string(), ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("NWC listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus("NWC".to_string(), ComponentStatus::Disabled)).await;
        }
    }
}

// ============================================================================
// Listener Management
// ============================================================================

async fn start_listener(
    name: &str,
    handles: &Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>>,
    config: &SharedConfig,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>
) {
    stop_listener(name, handles).await;

    let current = snapshot(config);
    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    let handle = match name {
        "Zaps" if current.zaps.is_some() => {
            let (cfg, shared, tx, tracker) = (current.zaps.unwrap(), config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_zaps(cfg, shared, tx, tracker, cancel_clone).await })
        },
        "Boostboard" if current.boostboard.is_some() => {
            let (cfg, shared, tx, tracker) = (current.boostboard.unwrap(), config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_boostboard(cfg, shared, tx, tracker, cancel_clone).await })
        },
        "NWC" if current.nwc.is_some() => {
            let (cfg, shared, tx, tracker) = (current.nwc.unwrap(), config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_nwc(cfg, shared, tx, tracker, cancel_clone).await })
        },
        _ => {
            eprintln!("Cannot start {}: not configured or unknown", name);
            return;
        }
    };

    handles.lock().await.insert(name.to_string(), (handle, cancel_token));
}

/// What a reload should do with one input listener.
#[derive(Debug, PartialEq)]
enum ListenerChange {
    Unchanged,
    Restart,
    Stop,
}

fn listener_change<T: PartialEq>(old: &Option<T>, new: &Option<T>) -> ListenerChange {
    if old == new {
        ListenerChange::Unchanged
    } else if new.is_some() {
        ListenerChange::Restart
    } else {
        ListenerChange::Stop
    }
}

async fn reload_config(
    config: &SharedConfig,
    handles: &Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>>,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>,
) {
    println!("Reloading config...");
    let new = match config::load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Config reload failed: {:#}", e);
            let _ = tx.send(GuiMessage::ConfigReloadFailed(format!("{:#}", e))).await;
            return;
        }
    };

    let old = snapshot(config);
    {
        let mut guard = config.write().unwrap();
        *guard = new.clone();
    }

    for (name, change) in [
        ("NWC", listener_change(&old.nwc, &new.nwc)),
        ("Boostboard", listener_change(&old.boostboard, &new.boostboard)),
        ("Zaps", listener_change(&old.zaps, &new.zaps)),
    ] {
        match change {
            ListenerChange::Unchanged => {}
            ListenerChange::Restart => {
                println!("Restarting {} listener after config reload", name);
                start_listener(name, handles, config, tx, tracker).await;
            }
            ListenerChange::Stop => {
                println!("Stopping {} listener after config reload", name);
                stop_listener(name, handles).await;
                let _ = tx.send(GuiMessage::UpdateStatus(name.to_string(), ComponentStatus::Disabled)).await;
            }
        }
    }

    let setup_cfg = new.clone();
    let setup_tx = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = setup_effects(setup_cfg).await {
            eprintln!("Error setting up effects: {:#}", e);
            let _ = setup_tx.send(GuiMessage::UpdateStatus(
                "Effects".to_string(),
                ComponentStatus::Error(format!("{:#}", e)),
            )).await;
        }
    });

    println!("Config reloaded");
    let _ = tx.send(GuiMessage::ConfigReloaded(Box::new(new))).await;
}

async fn stop_listener(
    name: &str,
    handles: &Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>>
) {
    if let Some((handle, cancel_token)) = handles.lock().await.remove(name) {
        println!("Cancelling {} listener...", name);
        cancel_token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        println!("{} listener stopped", name);
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting BlinkyBoosts...");

    let config = config::load_config()?;
    let shared: SharedConfig = Arc::new(std::sync::RwLock::new(config));
    let rt = tokio::runtime::Runtime::new()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);
    let sat_tracker = Arc::new(Mutex::new(sat_tracker::SatTracker::new()));

    // Setup effects
    rt.spawn({
        let config = snapshot(&shared);
        let tx = tx.clone();
        async move {
            if let Err(e) = setup_effects(config).await {
                eprintln!("Error setting up effects: {:#}", e);
                let _ = tx.send(GuiMessage::UpdateStatus("Effects".to_string(), ComponentStatus::Error(format!("{:#}", e)))).await;
            }
        }
    });

    // Track listener tasks
    let listener_handles: Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Start initial listeners
    rt.spawn({
        let (handles, config, tx, tracker) = (listener_handles.clone(), shared.clone(), tx.clone(), sat_tracker.clone());
        async move {
            let cfg = snapshot(&config);
            if cfg.zaps.is_some() {
                start_listener("Zaps", &handles, &config, &tx, &tracker).await;
            }
            if cfg.boostboard.is_some() {
                start_listener("Boostboard", &handles, &config, &tx, &tracker).await;
            }
            if cfg.nwc.is_some() {
                start_listener("NWC", &handles, &config, &tx, &tracker).await;
            }
        }
    });

    // Message handler
    let (gui_tx, gui_rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);
    rt.spawn({
        let (config, tracker, handles) = (shared.clone(), sat_tracker.clone(), listener_handles.clone());
        async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    GuiMessage::TestTrigger(sats) => {
                        println!("Test trigger received for {} sats", sats);
                        process_boost("Test", sats, &gui_tx, &tracker, &config, true).await;
                    },
                    GuiMessage::RunAction(name) => {
                        println!("Manual action triggered: {}", name);
                        let (current, tx) = (snapshot(&config), gui_tx.clone());
                        tokio::spawn(async move {
                            // A manual look replaces the current one, so a waiting revert must not undo it.
                            revert::cancel_pending().await;
                            let names = [name.clone()];
                            let err = actions::run(&current, &names).await
                                .err()
                                .map(|e| format!("{:#}", e));
                            if let Some(e) = &err {
                                eprintln!("Manual action '{}' failed: {}", name, e);
                            }
                            let _ = tx.send(GuiMessage::ActionFinished(name, err)).await;
                        });
                    },
                    GuiMessage::StartListener(name) => {
                        println!("Starting listener: {}", name);
                        start_listener(&name, &handles, &config, &gui_tx, &tracker).await;
                    },
                    GuiMessage::StopListener(name) => {
                        println!("Stopping listener: {}", name);
                        stop_listener(&name, &handles).await;
                    },
                    GuiMessage::ReloadConfig => {
                        reload_config(&config, &handles, &gui_tx, &tracker).await;
                    },
                    other => { let _ = gui_tx.send(other).await; }
                }
            }
        }
    });

    gui::run_gui(tx, gui_rx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_restarts_only_changed_inputs() {
        let nwc = config::NWC { uri: "a".into(), filters: Default::default() };
        assert_eq!(listener_change(&Some(nwc.clone()), &Some(nwc.clone())), ListenerChange::Unchanged);
        let changed = config::NWC { uri: "b".into(), filters: Default::default() };
        assert_eq!(listener_change(&Some(nwc.clone()), &Some(changed)), ListenerChange::Restart);
        assert_eq!(listener_change(&Some(nwc), &None), ListenerChange::Stop);
        assert_eq!(listener_change::<config::NWC>(&None, &None), ListenerChange::Unchanged);
    }

    #[test]
    fn ending_match_uses_the_digits_given() {
        assert!(sats_end_with(101, 1));
        assert!(sats_end_with(11, 1));
        assert!(sats_end_with(1, 1));
        assert!(!sats_end_with(101, 2));
        assert!(sats_end_with(121, 21));
        assert!(!sats_end_with(101, 21));
        assert!(sats_end_with(100, 0));
        assert!(!sats_end_with(101, 0));
    }
}