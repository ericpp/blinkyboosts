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

use gui::{ComponentStatus, GuiMessage};

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
    let Some(cfg) = config.wled else { return Ok(()) };
    if !cfg.setup { return Ok(()) };

    let mut wled = wled::WLed::new();
    wled.load(&cfg.host).await.context("Unable to load from WLED")?;

    if let Some(presets) = &cfg.presets {
        for (idx, preset) in presets.iter().enumerate() {
            wled.set_preset(idx, &cfg, preset).await?;
        }
    }

    if let Some(playlists) = &cfg.playlists {
        for (idx, playlist) in playlists.iter().enumerate() {
            wled.set_playlist(idx, &cfg, playlist).await?;
        }
    }

    Ok(())
}

fn format_toggle_description(toggle: &config::Toggle) -> String {
    match toggle.output.to_lowercase().as_str() {
        "osc" => toggle.osc.as_ref().map_or("OSC".to_string(), |osc| {
            use crate::config::OscArgValue;
            let value_str = match &osc.arg_value {
                OscArgValue::String(s) => format!("\"{}\"", s),
                OscArgValue::Int(i) => i.to_string(),
                OscArgValue::Float(f) => f.to_string(),
            };
            format!("OSC {}: {}", osc.path, value_str)
        }),
        "artnet" => toggle.artnet.as_ref()
            .map_or("Art-Net".to_string(), |a| format!("Art-Net ch{}: {}", a.channel, a.value)),
        "sacn" => toggle.sacn.as_ref()
            .map_or("sACN".to_string(), |s| format!("sACN ch{}: {}", s.channel, s.value)),
        "wled" => toggle.wled.as_ref()
            .map_or("WLED".to_string(), |w| format!("WLED: {}", w.preset)),
        _ => toggle.output.clone()
    }
}

async fn trigger_single_toggle(config: &config::Config, toggle: &config::Toggle) -> Result<()> {
    match toggle.output.to_lowercase().as_str() {
        "osc" => {
            let osc_cfg = config.osc.as_ref().context("OSC not configured")?;
            osc::Osc::new(&osc_cfg.address)?.trigger_toggle(toggle)?;
        },
        "artnet" => {
            let cfg = config.artnet.as_ref().context("Art-Net not configured")?;
            artnet::ArtNet::trigger_toggle(
                toggle, cfg.universe.unwrap_or(0),
                cfg.broadcast_address.clone(), cfg.local_address.clone()
            )?;
        },
        "sacn" => {
            let cfg = config.sacn.as_ref().context("sACN not configured")?;
            sacn::Sacn::trigger_toggle(toggle, cfg.universe.unwrap_or(1), cfg.broadcast_address.clone())?;
        },
        "wled" => {
            let cfg = config.wled.as_ref().context("WLED not configured")?;
            wled::WLed::trigger_toggle(toggle, &cfg.host).await?;
        },
        _ => eprintln!("Unknown toggle output type: {}", toggle.output),
    }
    Ok(())
}

async fn trigger_toggles(
    config: &config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    let Some(toggles) = &config.toggles else { return Ok(Vec::new()) };

    let last_digit = (sats % 10).abs() as u8;
    let mut triggered_effects = Vec::new();

    // Check threshold-based toggles
    let threshold_toggles: Vec<_> = toggles.iter()
        .filter(|t| !t.is_default && t.use_total && t.threshold > 0)
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
                            let in_range = last_digit >= start && last_digit <= end;
                            if !in_range {
                                println!("Toggle skipped: {} sats threshold ends with {}, not in range {}-{}", max_crossed, last_digit, start, end);
                            }
                            in_range
                        });

                    if should_trigger {
                        if let Err(e) = trigger_single_toggle(config, toggle).await {
                            eprintln!("Failed to trigger toggle at {} sats: {:#}", max_crossed, e);
                        } else {
                            triggered_effects.push(format_toggle_description(toggle));
                        }
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

    // Trigger default toggles if no threshold was triggered
    if !threshold_triggered {
        for toggle in toggles.iter().filter(|t| t.is_default) {
            let should_trigger = toggle.endswith_range
                .map_or(true, |(start, end)| {
                    let in_range = last_digit >= start && last_digit <= end;
                    if !in_range {
                        println!("Default toggle skipped: {} sats ends with {}, not in range {}-{}", sats, last_digit, start, end);
                    }
                    in_range
                });

            if should_trigger {
                println!("Default toggle triggered for {} sats - {} output", sats, toggle.output);
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
    config: &config::Config,
    trigger_effects_flag: bool
) {
    let total = tracker.lock().await.add(source, sats);
    println!("{} received: {} sats, total now: {} sats", source, sats, total);

    let _ = tx.send(GuiMessage::UpdateSatTotal(total)).await;

    let effects = if trigger_effects_flag {
        trigger_effects(config.clone(), sats, Some(tracker.clone())).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let _ = tx.send(GuiMessage::BoostReceived(source.to_string(), sats, effects)).await;
}

async fn sync_threshold_triggers(config: &config::Config, tracker: &Arc<Mutex<sat_tracker::SatTracker>>) {
    if let Some(toggles) = &config.toggles {
        let thresholds: Vec<i64> = toggles.iter()
            .filter(|t| !t.is_default && t.use_total && t.threshold > 0)
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
    config: config::Config,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    let cfg = config.zaps.clone().unwrap();
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
    config: config::Config,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    let cfg = config.boostboard.clone().unwrap();
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

    sync_threshold_triggers(&config, &tracker).await;

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
    config: config::Config,
    tx: tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: Arc<Mutex<sat_tracker::SatTracker>>,
    cancel_token: CancellationToken
) {
    let cfg = config.nwc.clone().unwrap();
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

    sync_threshold_triggers(&config, &tracker).await;

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
    config: &config::Config,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>
) {
    stop_listener(name, handles).await;

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    let handle = match name {
        "Zaps" if config.zaps.is_some() => {
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_zaps(cfg, tx, tracker, cancel_clone).await })
        },
        "Boostboard" if config.boostboard.is_some() => {
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_boostboard(cfg, tx, tracker, cancel_clone).await })
        },
        "NWC" if config.nwc.is_some() => {
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move { listen_for_nwc(cfg, tx, tracker, cancel_clone).await })
        },
        _ => {
            eprintln!("Cannot start {}: not configured or unknown", name);
            return;
        }
    };

    handles.lock().await.insert(name.to_string(), (handle, cancel_token));
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
    let rt = tokio::runtime::Runtime::new()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);
    let sat_tracker = Arc::new(Mutex::new(sat_tracker::SatTracker::new()));

    // Setup effects
    rt.spawn({
        let config = config.clone();
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
        let (handles, config, tx, tracker) = (listener_handles.clone(), config.clone(), tx.clone(), sat_tracker.clone());
        async move {
            if config.zaps.is_some() {
                start_listener("Zaps", &handles, &config, &tx, &tracker).await;
            }
            if config.boostboard.is_some() {
                start_listener("Boostboard", &handles, &config, &tx, &tracker).await;
            }
            if config.nwc.is_some() {
                start_listener("NWC", &handles, &config, &tx, &tracker).await;
            }
        }
    });

    // Message handler
    let (gui_tx, gui_rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);
    rt.spawn({
        let (config, tracker, handles) = (config.clone(), sat_tracker.clone(), listener_handles.clone());
        async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    GuiMessage::TestTrigger(sats) => {
                        println!("Test trigger received for {} sats", sats);
                        process_boost("Test", sats, &gui_tx, &tracker, &config, true).await;
                    },
                    GuiMessage::StartListener(name) => {
                        println!("Starting listener: {}", name);
                        start_listener(&name, &handles, &config, &gui_tx, &tracker).await;
                    },
                    GuiMessage::StopListener(name) => {
                        println!("Stopping listener: {}", name);
                        stop_listener(&name, &handles).await;
                    },
                    other => { let _ = gui_tx.send(other).await; }
                }
            }
        }
    });

    gui::run_gui(tx, gui_rx)?;
    Ok(())
}