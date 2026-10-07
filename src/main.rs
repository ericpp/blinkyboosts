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
mod obs;
mod zaps;
mod splitkit;
mod gui;
mod sat_tracker;
mod revert;
mod actions;
mod queue;

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
    for (name, output) in &config.outputs {
        let config::Output::Wled(cfg) = &output.kind else { continue };
        if !output.enabled || !cfg.setup { continue };

        println!("Setting up WLED output '{}'", name);
        let mut wled = wled::WLed::new();
        wled.load(&cfg.host).await
            .with_context(|| format!("Unable to load WLED output '{}'", name))?;

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

fn fire_toggle(config: &config::Config, toggle: &config::Toggle, triggered_effects: &mut Vec<String>) -> bool {
    let revert = match toggle.revert_after {
        Some(secs) if !toggle.revert.is_empty() => Some((secs, toggle.revert.clone())),
        _ => None,
    };
    // The worker runs this after anything already queued, and skips it when the
    // same actions are already running or waiting.
    let queued = queue::enqueue(queue::Job {
        config: config.clone(),
        names: toggle.actions.clone(),
        revert,
        replaces_look: true,
        skip_duplicate: true,
        done: None,
    });
    if queued {
        triggered_effects.push(format_toggle_description(toggle));
    }
    queued
}

async fn trigger_toggles(
    config: &config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    let Some(toggles) = &config.toggles else { return Ok(Vec::new()) };

    let last_two_digits = (sats % 100).abs() as u8;
    let mut triggered_effects = Vec::new();

    // Effect toggles fire only from SplitKit broadcasts, never from sats.
    let toggles: Vec<_> = toggles.iter().filter(|t| t.effect.is_none()).collect();

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
                        fire_toggle(config, toggle, &mut triggered_effects);
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
                fire_toggle(config, toggle, &mut triggered_effects);
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
            fire_toggle(config, toggle, &mut triggered_effects);
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
                if fire_toggle(config, toggle, &mut triggered_effects) {
                    println!("Default toggle triggered for {} sats - {}", sats, actions::describe(&toggle.actions));
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
    name: String,
    cfg: config::Zaps,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener(&name, &tx).await;

    let zap = match zaps::Zaps::new(&cfg.relay_addrs, &cfg.naddr).await {
        Ok(z) => z,
        Err(e) => return handle_connection_error(&name, e, &tx).await,
    };

    let load_since = match cfg.load_since {
        Some(load_since_str) => match parse_timestamp(&load_since_str) {
            Ok(ts) => Some(ts),
            Err(_) => None,
        },
        None => None,
    };

    println!("Waiting for {name}...");

    let callback_name = name.clone();
    let error_name = name.clone();
    tokio::select! {
        result = zap.subscribe_zaps(load_since, |zap: zaps::Zap| {
            let (config, tx, tracker, name) = (config.clone(), tx.clone(), tracker.clone(), callback_name.clone());
            async move {
                println!("Zap: {:#?}", zap);
                process_boost(&name, zap.value_msat_total / 1000, &tx, &tracker, &config, !zap.is_old).await;
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling {error_name} events: {error_msg}");
                let _ = tx.send(GuiMessage::UpdateStatus(error_name, ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("{name} listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(name, ComponentStatus::Disabled)).await;
        }
    }
}

async fn listen_for_boostboard(
    name: String,
    cfg: config::BoostBoard,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener(&name, &tx).await;

    if cfg.relay_addrs.is_empty() {
        eprintln!("Error: No relay addresses specified for {name}");
        let _ = tx.send(GuiMessage::UpdateStatus(name, ComponentStatus::Error("No relay addresses specified".to_string()))).await;
        return;
    }

    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    println!("{name} Filters: {:#?}", &filters);

    let board = match boostboard::BoostBoard::new(&cfg.relay_addrs, &cfg.pubkey, filters.clone()).await {
        Ok(b) => b,
        Err(e) => return handle_connection_error(&name, e, &tx).await,
    };

    let load_since = Some(parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now()));

    // Load stored boosts
    println!("Loading stored boosts from API...");
    let stored_boosts = boostboard::StoredBoosts::new(filters);
    let load_name = name.clone();
    let _ = stored_boosts.load(|boost: boosts::Boostagram| {
        let (tx, tracker, config, name) = (tx.clone(), tracker.clone(), config.clone(), load_name.clone());
        async move {
            if boost.action == "boost" {
                process_boost(&name, boost.sats, &tx, &tracker, &config, false).await;
            }
        }
    }).await;

    sync_threshold_triggers(&snapshot(&config), &tracker).await;

    let subscription_id = match board.subscribe(load_since).await {
        Ok(id) => id,
        Err(e) => {
            let error_msg = format!("Subscription error: {:#}", e);
            eprintln!("Error subscribing to {name}: {error_msg}");
            let _ = tx.send(GuiMessage::UpdateStatus(name, ComponentStatus::Error(error_msg))).await;
            return;
        }
    };

    println!("Waiting for {name} boosts...");
    let subscription_start_time = Timestamp::now();
    let tx_clone = tx.clone();
    let callback_name = name.clone();
    let error_name = name.clone();

    tokio::select! {
        result = board.handle_boosts(subscription_id, move |boost: boosts::Boostagram, event_ts: Timestamp| {
            let (config, tx, tracker, name) = (config.clone(), tx_clone.clone(), tracker.clone(), callback_name.clone());
            async move {
                if boost.action == "boost" {
                    println!("Boost: {:#?}", boost);
                    let trigger = event_ts >= subscription_start_time;
                    process_boost(&name, boost.sats, &tx, &tracker, &config, trigger).await;
                }
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling {error_name} events: {error_msg}");
                let _ = tx.send(GuiMessage::UpdateStatus(error_name, ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("{name} listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(name, ComponentStatus::Disabled)).await;
        }
    }
}

async fn listen_for_nwc(
    name: String,
    cfg: config::NWC,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    initialize_listener(&name, &tx).await;

    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    println!("{name} Filters: {:#?}", &filters);

    let nwc = match nwc::NWC::new(&cfg.uri, filters).await {
        Ok(n) => n,
        Err(e) => return handle_connection_error(&name, e, &tx).await,
    };

    let load_since = parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now());

    println!("Loading previous boosts from {name}...");
    let load_name = name.clone();
    let latest_boost_timestamp = nwc.load_previous_boosts(Some(load_since), |boost: boosts::Boostagram| {
        let (tx, tracker, config, name) = (tx.clone(), tracker.clone(), config.clone(), load_name.clone());
        async move {
            process_boost(&name, boost.sats, &tx, &tracker, &config, false).await;
        }
    }).await.unwrap_or(None);

    sync_threshold_triggers(&snapshot(&config), &tracker).await;

    let subscription_start = latest_boost_timestamp.map(|ts| ts + 1).unwrap_or(load_since);
    println!("Waiting for {name} boosts...");

    let callback_name = name.clone();
    let error_name = name.clone();
    tokio::select! {
        result = nwc.subscribe_boosts(subscription_start, |boost: boosts::Boostagram| {
            let (config, tx, tracker, name) = (config.clone(), tx.clone(), tracker.clone(), callback_name.clone());
            async move {
                if boost.action == "boost" {
                    println!("{name} Boost: {:#?}", boost);
                    process_boost(&name, boost.sats, &tx, &tracker, &config, true).await;
                }
            }
        }) => {
            if let Err(e) = result {
                let error_msg = format!("Event error: {:#}", e);
                eprintln!("Error handling {error_name} events: {error_msg}");
                let _ = tx.send(GuiMessage::UpdateStatus(error_name, ComponentStatus::Error(error_msg))).await;
            }
        }
        _ = cancel_token.cancelled() => {
            println!("{name} listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(name, ComponentStatus::Disabled)).await;
        }
    }
}

async fn listen_for_splitkit(
    name: String,
    cfg: config::SplitKit,
    config: SharedConfig,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    cancel_token: CancellationToken
) {
    initialize_listener(&name, &tx).await;

    loop {
        println!("Connecting to {name} at {}...", cfg.url);
        let callback_name = name.clone();
        let cancel_name = name.clone();
        let result = tokio::select! {
            result = splitkit::subscribe(&cfg.url, |effect: splitkit::FiredEffect| {
                let (config, tx, name) = (config.clone(), tx.clone(), callback_name.clone());
                async move {
                    println!(
                        "{name} effect '{}': {} says \"{}\"",
                        effect.label, effect.sender_name, effect.message
                    );
                    let current = snapshot(&config);
                    let mut triggered = Vec::new();
                    if let Some(toggles) = &current.toggles {
                        for toggle in toggles.iter().filter(|t| t.effect.as_deref() == Some(effect.id.as_str())) {
                            fire_toggle(&current, toggle, &mut triggered);
                        }
                    }
                    let _ = tx.send(GuiMessage::BoostReceived(
                        format!("{name}: {} ({})", effect.label, effect.sender_name),
                        0,
                        triggered,
                    )).await;
                }
            }) => result,
            _ = cancel_token.cancelled() => {
                println!("{cancel_name} listener cancelled");
                let _ = tx.send(GuiMessage::UpdateStatus(cancel_name, ComponentStatus::Disabled)).await;
                return;
            }
        };

        let (message, wait) = match result {
            Ok(()) => ("connection ended".to_string(), 5),
            Err(e) if e.downcast_ref::<splitkit::LiveDisabled>().is_some() => {
                ("live mode disabled".to_string(), 30)
            }
            Err(e) => (format!("{:#}", e), 5),
        };
        eprintln!("{name}: {message}, retrying in {wait}s");
        let _ = tx.send(GuiMessage::UpdateStatus(name.clone(), ComponentStatus::Error(message))).await;

        let cancel_name = name.clone();
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(wait)) => {}
            _ = cancel_token.cancelled() => {
                println!("{cancel_name} listener cancelled");
                let _ = tx.send(GuiMessage::UpdateStatus(cancel_name, ComponentStatus::Disabled)).await;
                return;
            }
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
    let Some(input) = current.inputs.get(name).cloned() else {
        eprintln!("Cannot start {}: not configured or unknown", name);
        return;
    };

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();
    let listener_name = name.to_string();

    let handle = match input.kind {
        config::Input::Zaps(cfg) => {
            let (shared, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_zaps(listener_name, cfg, shared, tx, tracker, cancel_clone).await })
        },
        config::Input::Boostboard(cfg) => {
            let (shared, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_boostboard(listener_name, cfg, shared, tx, tracker, cancel_clone).await })
        },
        config::Input::Nwc(cfg) => {
            let (shared, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_nwc(listener_name, cfg, shared, tx, tracker, cancel_clone).await })
        },
        config::Input::Splitkit(cfg) => {
            let (shared, tx) = (config.clone(), tx.clone());
            tokio::spawn(async move { listen_for_splitkit(listener_name, cfg, shared, tx, cancel_clone).await })
        },
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

fn listener_change<T: PartialEq>(old: Option<&T>, new: Option<&T>) -> ListenerChange {
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

    let mut names: Vec<&str> = old.inputs.keys().chain(new.inputs.keys()).map(|s| s.as_str()).collect();
    names.sort_unstable();
    names.dedup();

    for name in names {
        let change = listener_change(
            old.inputs.get(name).filter(|input| input.enabled),
            new.inputs.get(name).filter(|input| input.enabled),
        );
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
            let names: Vec<String> = cfg.inputs.iter()
                .filter(|(_, input)| input.enabled)
                .map(|(name, _)| name.clone())
                .collect();
            for name in names {
                start_listener(&name, &handles, &config, &tx, &tracker).await;
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
                            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
                            let queued = queue::enqueue(queue::Job {
                                config: current,
                                names: vec![name.clone()],
                                revert: None,
                                replaces_look: true,
                                skip_duplicate: false,
                                done: Some(done_tx),
                            });
                            let err = if queued {
                                match done_rx.await {
                                    Ok(Ok(())) => None,
                                    Ok(Err(e)) => Some(format!("{:#}", e)),
                                    Err(_) => Some("action queue stopped".to_string()),
                                }
                            } else {
                                Some("action queue stopped".to_string())
                            };
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
        assert_eq!(listener_change(Some(&nwc), Some(&nwc)), ListenerChange::Unchanged);
        let changed = config::NWC { uri: "b".into(), filters: Default::default() };
        assert_eq!(listener_change(Some(&nwc), Some(&changed)), ListenerChange::Restart);
        assert_eq!(listener_change(Some(&nwc), None), ListenerChange::Stop);
        assert_eq!(listener_change::<config::NWC>(None, None), ListenerChange::Unchanged);
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