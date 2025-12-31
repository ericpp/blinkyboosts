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

fn parse_timestamp(s: &str) -> Result<Timestamp> {
    s.parse::<u64>()
        .map(Timestamp::from_secs)
        .context("Failed to parse timestamp as unix seconds")
}

async fn setup_effects(config: config::Config) -> Result<()> {
    let Some(cfg) = config.wled else {
        return Ok(());
    };

    if !cfg.setup {
        return Ok(());
    }

    let mut wled = wled::WLed::new();
    wled.load(&cfg.host).await
        .context("Unable to load from WLED")?;

    if let Some(presets) = &cfg.presets {
        for (idx, preset) in presets.iter().enumerate() {
            wled.set_preset(idx, &cfg, preset).await
                .context("Failed to set WLED preset")?;
        }
    }

    if let Some(playlists) = &cfg.playlists {
        for (idx, playlist) in playlists.iter().enumerate() {
            wled.set_playlist(idx, &cfg, playlist).await
                .context("Failed to set WLED playlist")?;
        }
    }

    Ok(())
}

async fn trigger_wled_effects(cfg: config::WLed, sats: i64) -> Result<()> {
    let number_playlist = format!("BOOST-{}", sats);
    let endnum = sats.to_string().chars().last().unwrap();
    let endnum_playlist = format!("BOOST-{}", endnum);

    let mut wled = wled::WLed::new();
    wled.load(&cfg.host).await
        .context("Failed to load WLED configuration")?;

    // Try to find playlist in order of specificity
    let playlist = wled.get_preset(&number_playlist)
        .or_else(|| wled.get_preset(&endnum_playlist))
        .or_else(|| wled.get_preset(&cfg.boost_playlist));

    if let Some(playlist) = playlist {
        println!("Triggering WLED playlist {}", playlist.name);
        wled.run_preset(playlist).await
            .context("Failed to run WLED preset")?;
    } else {
        eprintln!(
            "Unable to find WLED playlist matching {}, {}, or {}",
            number_playlist, endnum_playlist, cfg.boost_playlist
        );
    }

    Ok(())
}

async fn trigger_toggles(
    config: &config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    let Some(toggles) = &config.toggles else {
        return Ok(Vec::new());
    };

    let mut any_triggered = false;
    let mut triggered_effects = Vec::new();

    // Get the last digit of sats for endswith_range checks
    let last_digit = (sats % 10).abs() as u8;

    // First pass: process non-default toggles (threshold-based or endswith_range-only)
    for toggle in toggles.iter().filter(|t| !t.is_default) {
        // Check if this toggle should trigger based on threshold or endswith_range
        let mut should_trigger = false;

        // If there's a threshold, check it
        if toggle.threshold > 0 {
            should_trigger = check_total_threshold(tracker.as_ref(), toggle.threshold, toggle.trigger_multiple, sats).await;
        } else if toggle.endswith_range.is_some() {
            // No threshold, but has endswith_range - always consider for triggering
            should_trigger = true;
        }

        if should_trigger {
            // Check endswith_range if specified
            if let Some((start, end)) = toggle.endswith_range {
                if last_digit < start || last_digit > end {
                    println!("Toggle skipped: {} sats ends with {}, not in range {}-{}", sats, last_digit, start, end);
                    continue;
                }
            }

            any_triggered = true;

            if let Err(e) = trigger_single_toggle(config, toggle).await {
                eprintln!("Failed to trigger toggle: {:#}", e);
            } else {
                triggered_effects.push(format_toggle_description(toggle));
            }
        }
    }

    // Second pass: if no threshold toggles were triggered, process default toggles
    if !any_triggered {
        for toggle in toggles.iter().filter(|t| t.is_default) {
            // Check endswith_range if specified
            if let Some((start, end)) = toggle.endswith_range {
                if last_digit < start || last_digit > end {
                    println!("Default toggle skipped: {} sats ends with {}, not in range {}-{}", sats, last_digit, start, end);
                    continue;
                }
            }

            println!("Default toggle triggered for {} sats - {} output", sats, toggle.output);

            if let Err(e) = trigger_single_toggle(config, toggle).await {
                eprintln!("Failed to trigger default toggle: {:#}", e);
            } else {
                triggered_effects.push(format_toggle_description(toggle));
            }
        }
    }

    Ok(triggered_effects)
}

fn format_toggle_description(toggle: &config::Toggle) -> String {
    match toggle.output.to_lowercase().as_str() {
        "osc" => {
            if let Some(osc) = &toggle.osc {
                use crate::config::OscArgValue;
                let value_str = match &osc.arg_value {
                    OscArgValue::String(s) => format!("\"{}\"", s),
                    OscArgValue::Int(i) => i.to_string(),
                    OscArgValue::Float(f) => f.to_string(),
                };
                format!("OSC {}: {}", osc.path, value_str)
            } else {
                "OSC".to_string()
            }
        },
        "artnet" => {
            if let Some(artnet) = &toggle.artnet {
                format!("Art-Net ch{}: {}", artnet.channel, artnet.value)
            } else {
                "Art-Net".to_string()
            }
        },
        "sacn" => {
            if let Some(sacn) = &toggle.sacn {
                format!("sACN ch{}: {}", sacn.channel, sacn.value)
            } else {
                "sACN".to_string()
            }
        },
        "wled" => {
            if let Some(wled) = &toggle.wled {
                format!("WLED: {}", wled.preset)
            } else {
                "WLED".to_string()
            }
        },
        _ => toggle.output.clone()
    }
}

async fn check_total_threshold(
    tracker: Option<&Arc<Mutex<sat_tracker::SatTracker>>>,
    threshold: i64,
    trigger_multiple: bool,
    boost_amount: i64
) -> bool {
    let Some(tracker_ref) = tracker else {
        return false;
    };

    let mut tracker_guard = tracker_ref.lock().await;
    let new_total = tracker_guard.get_total();
    let previous_total = new_total - boost_amount;
    let should_trigger = tracker_guard.should_trigger_threshold(previous_total, new_total, threshold, trigger_multiple);

    if should_trigger {
        tracker_guard.update_last_triggered_threshold(threshold, trigger_multiple);
        if trigger_multiple {
            let multiple = new_total / threshold;
            let threshold_value = multiple * threshold;
            println!(
                "Toggle triggered: {} total sats crossed {} threshold (multiple {}, threshold {})",
                new_total, threshold, multiple, threshold_value
            );
        } else {
            println!(
                "Toggle triggered: {} total sats >= {} threshold",
                new_total, threshold
            );
        }
    }

    should_trigger
}

async fn trigger_single_toggle(config: &config::Config, toggle: &config::Toggle) -> Result<()> {
    match toggle.output.to_lowercase().as_str() {
        "osc" => {
            let Some(osc_cfg) = &config.osc else {
                eprintln!("OSC toggle configured but OSC is not configured");
                return Ok(());
            };
            let osc = osc::Osc::new(&osc_cfg.address)?;
            osc.trigger_toggle(toggle)?;
        },
        "artnet" => {
            let Some(artnet_cfg) = &config.artnet else {
                eprintln!("Art-Net toggle configured but Art-Net is not configured");
                return Ok(());
            };
            artnet::ArtNet::trigger_toggle(
                toggle,
                artnet_cfg.universe.unwrap_or(0),
                artnet_cfg.broadcast_address.clone(),
                artnet_cfg.local_address.clone()
            )?;
        },
        "sacn" => {
            let Some(sacn_cfg) = &config.sacn else {
                eprintln!("sACN toggle configured but sACN is not configured");
                return Ok(());
            };
            sacn::Sacn::trigger_toggle(
                toggle,
                sacn_cfg.universe.unwrap_or(1),
                sacn_cfg.broadcast_address.clone()
            )?;
        },
        "wled" => {
            let Some(wled_cfg) = &config.wled else {
                eprintln!("WLED toggle configured but WLED is not configured");
                return Ok(());
            };
            wled::WLed::trigger_toggle(toggle, &wled_cfg.host).await?;
        },
        _ => {
            eprintln!("Unknown toggle output type: {}", toggle.output);
        }
    }

    Ok(())
}

// Generic listener initialization that handles common patterns
async fn initialize_listener(
    component_name: &str,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>
) {
    let _ = tx.send(GuiMessage::UpdateStatus(component_name.to_string(), ComponentStatus::Running)).await;
}

// Helper to sync trigger state after loading historical data
async fn sync_threshold_triggers(
    config: &config::Config,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>
) {
    if let Some(toggles) = &config.toggles {
        let thresholds: Vec<(i64, bool)> = toggles.iter()
            .filter(|t| !t.is_default && t.use_total)
            .map(|t| (t.threshold, t.trigger_multiple))
            .collect();

        if !thresholds.is_empty() {
            let mut tracker_guard = tracker.lock().await;
            tracker_guard.sync_trigger_state(&thresholds);
        }
    }
}

async fn trigger_effects(
    config: config::Config,
    sats: i64,
    tracker: Option<Arc<Mutex<sat_tracker::SatTracker>>>
) -> Result<Vec<String>> {
    println!("Triggering effects for {} sats", sats);

    // Trigger toggles and collect what was triggered
    let triggered = match trigger_toggles(&config, sats, tracker.clone()).await {
        Ok(effects) => effects,
        Err(e) => {
            eprintln!("Failed to trigger toggles: {:#}", e);
            Vec::new()
        }
    };

    Ok(triggered)
}

// Shared logic for processing boosts
async fn process_boost(
    source: &str,
    sats: i64,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>,
    config: &config::Config,
    trigger_effects_flag: bool
) {
    // Add to tracker and get new total
    let total = {
        let mut tracker_guard = tracker.lock().await;
        tracker_guard.add(source, sats)
    };
    println!("{} received: {} sats, total now: {} sats", source, sats, total);

    // Update GUI with new total
    let _ = tx.send(GuiMessage::UpdateSatTotal(total)).await;

    // Trigger effects if requested
    let effects = if trigger_effects_flag {
        match trigger_effects(config.clone(), sats, Some(tracker.clone())).await {
            Ok(effects) => effects,
            Err(e) => {
                eprintln!("Unable to trigger effects: {:#}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Send boost received message to GUI with effects
    let _ = tx.send(GuiMessage::BoostReceived(source.to_string(), sats, effects)).await;
}

// Helper to handle connection errors
async fn handle_connection_error(
    component: &str,
    error: anyhow::Error,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>
) {
    let error_msg = format!("Connection error: {:#}", error);
    eprintln!("Error connecting to {}: {}", component, error_msg);
    let _ = tx.send(GuiMessage::UpdateStatus(
        component.to_string(),
        ComponentStatus::Error(error_msg)
    )).await;
}

// Helper to parse load_since timestamp
fn parse_load_since(load_since_str: Option<&String>, default: Timestamp) -> Timestamp {
    load_since_str
        .and_then(|s| {
            match parse_timestamp(s) {
                Ok(ts) => {
                    println!("Loading since: {}", s);
                    Some(ts)
                },
                Err(e) => {
                    eprintln!("Failed to parse load_since timestamp '{}': {:#}", s, e);
                    None
                }
            }
        })
        .unwrap_or(default)
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
        Err(e) => {
            handle_connection_error("Zaps", e, &tx).await;
            return;
        }
    };

    let load_since = Some(parse_load_since(cfg.load_since.as_ref(), Timestamp::now()));
    println!("Waiting for Zaps...");

    tokio::select! {
        result = zap.subscribe_zaps(load_since, |zap: zaps::Zap| {
            let config = config.clone();
            let tx = tx.clone();
            let tracker = tracker.clone();

            async move {
                println!("Zap: {:#?}", zap);
                let sats = zap.value_msat_total / 1000;
                process_boost("Zaps", sats, &tx, &tracker, &config, true).await;
            }
        }) => {
            match result {
                Ok(_) => {},
                Err(e) => {
                    let error_msg = format!("Event error: {:#}", e);
                    eprintln!("Error handling zap events: {}", error_msg);
                    let _ = tx.send(GuiMessage::UpdateStatus(
                        "Zaps".to_string(),
                        ComponentStatus::Error(error_msg)
                    )).await;
                }
            }
        }
        _ = cancel_token.cancelled() => {
            println!("Zaps listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(
                "Zaps".to_string(),
                ComponentStatus::Disabled
            )).await;
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
        let _ = tx.send(GuiMessage::UpdateStatus(
            "Boostboard".to_string(),
            ComponentStatus::Error("No relay addresses specified".to_string())
        )).await;
        return;
    }

    // Build filters
    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    let board = match boostboard::BoostBoard::new(&cfg.relay_addrs, &cfg.pubkey, filters.clone()).await {
        Ok(b) => b,
        Err(e) => {
            handle_connection_error("Boostboard", e, &tx).await;
            return;
        }
    };

    let load_since = Some(parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now()));

    // Load stored boosts
    println!("Loading stored boosts from API...");
    let stored_boosts = boostboard::StoredBoosts::new(filters);

    let tx_stored = tx.clone();
    let tracker_stored = tracker.clone();
    let config_stored = config.clone();

    let _ = stored_boosts.load(move |boost: boosts::Boostagram| {
        let tx = tx_stored.clone();
        let tracker = tracker_stored.clone();
        let config = config_stored.clone();

        async move {
            if boost.action == "boost" {
                let sats = boost.sats;
                process_boost("Boostboard", sats, &tx, &tracker, &config, false).await;
            }
        }
    }).await;

    // Sync trigger state after loading historical boosts
    sync_threshold_triggers(&config, &tracker).await;

    let subscription_id = match board.subscribe(load_since).await {
        Ok(id) => id,
        Err(e) => {
            let error_msg = format!("Subscription error: {:#}", e);
            eprintln!("Error subscribing to board: {}", error_msg);
            let _ = tx.send(GuiMessage::UpdateStatus(
                "Boostboard".to_string(),
                ComponentStatus::Error(error_msg)
            )).await;
            return;
        }
    };

    println!("Waiting for Boostboard boosts...");
    let subscription_start_time = Timestamp::now();

    let config_clone = config.clone();
    let tx_clone = tx.clone();
    let tracker_clone = tracker.clone();

    tokio::select! {
        result = board.handle_boosts(subscription_id, move |boost: boosts::Boostagram, event_ts: Timestamp| {
            let config = config_clone.clone();
            let tx = tx_clone.clone();
            let tracker = tracker_clone.clone();
            let subscription_start = subscription_start_time;

            async move {
                if boost.action == "boost" {
                    println!("Boost: {:#?}", boost);
                    let sats = boost.sats;
                    let trigger = event_ts >= subscription_start;
                    process_boost("Boostboard", sats, &tx, &tracker, &config, trigger).await;
                }
            }
        }) => {
            match result {
                Ok(_) => {},
                Err(e) => {
                    let error_msg = format!("Event error: {:#}", e);
                    eprintln!("Error handling boostboard events: {}", error_msg);
                    let _ = tx.send(GuiMessage::UpdateStatus(
                        "Boostboard".to_string(),
                        ComponentStatus::Error(error_msg)
                    )).await;
                }
            }
        }
        _ = cancel_token.cancelled() => {
            println!("Boostboard listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(
                "Boostboard".to_string(),
                ComponentStatus::Disabled
            )).await;
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

    // Build filters
    let filters = boostboard::BoostFilters {
        podcasts: cfg.filters.podcasts.clone(),
        episode_guids: cfg.filters.episode_guids.clone(),
        event_guids: cfg.filters.event_guids.clone(),
        before: cfg.filters.before.as_ref().and_then(|s| parse_timestamp(s).ok()),
        after: cfg.filters.after.as_ref().and_then(|s| parse_timestamp(s).ok()),
    };

    let nwc = match nwc::NWC::new(&cfg.uri, filters).await {
        Ok(n) => n,
        Err(e) => {
            handle_connection_error("NWC", e, &tx).await;
            return;
        }
    };

    let load_since = parse_load_since(cfg.filters.load_since.as_ref(), Timestamp::now());

    // Load stored boosts
    println!("Loading previous boosts from NWC...");
    let tx_stored = tx.clone();
    let tracker_stored = tracker.clone();
    let config_stored = config.clone();

    let latest_boost_timestamp = match nwc.load_previous_boosts(Some(load_since), move |boost: boosts::Boostagram| {
        let tx = tx_stored.clone();
        let tracker = tracker_stored.clone();
        let config = config_stored.clone();

        async move {
            let sats = boost.sats;
            process_boost("NWC", sats, &tx, &tracker, &config, false).await;
        }
    }).await {
        Ok(ts) => ts,
        Err(e) => {
            eprintln!("Error loading previous boosts from NWC: {:#}", e);
            let _ = tx.send(GuiMessage::UpdateStatus(
                "NWC".to_string(),
                ComponentStatus::Error(format!("Failed to load previous boosts: {:#}", e))
            )).await;
            None
        }
    };

    // Sync trigger state after loading historical boosts
    sync_threshold_triggers(&config, &tracker).await;

    // Use the latest boost timestamp or load_since as the starting point for subscription
    let subscription_start = latest_boost_timestamp
        .map(|ts| ts + 1) // Start from after the last loaded boost to avoid duplicates
        .unwrap_or(load_since);

    println!("Waiting for NWC boosts...");

    tokio::select! {
        result = nwc.subscribe_boosts(subscription_start, |boost: boosts::Boostagram| {
            let config = config.clone();
            let tx = tx.clone();
            let tracker = tracker.clone();

            async move {
                if boost.action == "boost" {
                    println!("NWC Boost: {:#?}", boost);
                    let sats = boost.sats;
                    process_boost("NWC", sats, &tx, &tracker, &config, true).await;
                }
            }
        }) => {
            match result {
                Ok(_) => {},
                Err(e) => {
                    let error_msg = format!("Event error: {:#}", e);
                    eprintln!("Error handling NWC events: {}", error_msg);
                    let _ = tx.send(GuiMessage::UpdateStatus(
                        "NWC".to_string(),
                        ComponentStatus::Error(error_msg)
                    )).await;
                }
            }
        }
        _ = cancel_token.cancelled() => {
            println!("NWC listener cancelled");
            let _ = tx.send(GuiMessage::UpdateStatus(
                "NWC".to_string(),
                ComponentStatus::Disabled
            )).await;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting BlinkyBoosts...");

    let config = config::load_config()?;
    let rt = tokio::runtime::Runtime::new()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);
    let sat_tracker = Arc::new(Mutex::new(sat_tracker::SatTracker::new()));

    // Setup effects
    let config_clone = config.clone();
    let tx_clone = tx.clone();
    rt.spawn(async move {
        if let Err(e) = setup_effects(config_clone).await {
            eprintln!("Error setting up effects: {:#}", e);
            let _ = tx_clone.send(GuiMessage::UpdateStatus(
                "Effects".to_string(),
                ComponentStatus::Error(format!("{:#}", e))
            )).await;
        }
    });

    // Track listener tasks and cancellation tokens
    let listener_handles: Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Start initial listeners
    let handles_clone = listener_handles.clone();
    let config_clone = config.clone();
    let tx_clone = tx.clone();
    let tracker_clone = sat_tracker.clone();
    rt.spawn(async move {
        if config_clone.zaps.is_some() {
            start_listener("Zaps", &handles_clone, &config_clone, &tx_clone, &tracker_clone).await;
        }
        if config_clone.boostboard.is_some() {
            start_listener("Boostboard", &handles_clone, &config_clone, &tx_clone, &tracker_clone).await;
        }
        if config_clone.nwc.is_some() {
            start_listener("NWC", &handles_clone, &config_clone, &tx_clone, &tracker_clone).await;
        }
    });

    // Handle test triggers, start/stop commands, and forward messages to GUI
    let config_for_tests = config.clone();
    let tracker_for_tests = sat_tracker.clone();
    let handles_for_control = listener_handles.clone();
    let (gui_tx, gui_rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);

    rt.spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                GuiMessage::TestTrigger(sats) => {
                    println!("Test trigger received for {} sats", sats);
                    process_boost("Test", sats, &gui_tx, &tracker_for_tests, &config_for_tests, true).await;
                },
                GuiMessage::StartListener(name) => {
                    println!("Starting listener: {}", name);
                    start_listener(&name, &handles_for_control, &config_for_tests, &gui_tx, &tracker_for_tests).await;
                },
                GuiMessage::StopListener(name) => {
                    println!("Stopping listener: {}", name);
                    stop_listener(&name, &handles_for_control).await;
                },
                other => {
                    let _ = gui_tx.send(other).await;
                }
            }
        }
    });

    gui::run_gui(tx.clone(), gui_rx)?;

    Ok(())
}

async fn start_listener(
    name: &str,
    handles: &Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>>,
    config: &config::Config,
    tx: &tokio::sync::mpsc::Sender<GuiMessage>,
    tracker: &Arc<Mutex<sat_tracker::SatTracker>>
) {
    // Stop existing listener if running
    stop_listener(name, handles).await;

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    let handle = match name {
        "Zaps" => {
            if config.zaps.is_none() {
                eprintln!("Cannot start Zaps: not configured");
                return;
            }
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move {
                listen_for_zaps(cfg, tx, tracker, cancel_clone).await;
            })
        },
        "Boostboard" => {
            if config.boostboard.is_none() {
                eprintln!("Cannot start Boostboard: not configured");
                return;
            }
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move {
                listen_for_boostboard(cfg, tx, tracker, cancel_clone).await;
            })
        },
        "NWC" => {
            if config.nwc.is_none() {
                eprintln!("Cannot start NWC: not configured");
                return;
            }
            let (cfg, tx, tracker) = (config.clone(), tx.clone(), tracker.clone());
            tokio::spawn(async move {
                listen_for_nwc(cfg, tx, tracker, cancel_clone).await;
            })
        },
        _ => {
            eprintln!("Unknown listener: {}", name);
            return;
        }
    };

    let mut handles_guard = handles.lock().await;
    handles_guard.insert(name.to_string(), (handle, cancel_token));
}

async fn stop_listener(
    name: &str,
    handles: &Arc<Mutex<HashMap<String, (JoinHandle<()>, CancellationToken)>>>
) {
    let mut handles_guard = handles.lock().await;
    if let Some((handle, cancel_token)) = handles_guard.remove(name) {
        println!("Cancelling {} listener...", name);
        cancel_token.cancel();
        drop(handles_guard); // Release lock before awaiting

        // Wait for task to complete (with timeout)
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        println!("{} listener stopped", name);
    }
}