use nostr_sdk::Timestamp;
use std::error::Error;
use tokio;
use anyhow::{Context, Result};

mod boostboard;
mod boosts;
mod config;
mod nwc;
mod osc;
mod artnet;
mod wled;
mod zaps;
mod gui;

use gui::{ComponentStatus, GuiMessage};

// Define a custom error type that is Send + Sync
#[derive(Debug)]
pub struct AppError(String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Application error: {}", self.0)
    }
}

impl Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError(format!("{:#}", err))
    }
}

async fn setup_effects(config: config::Config) -> Result<()> {
    if config.wled.is_none() {
        return Ok(());
    }

    let cfg = config.wled.unwrap();

    if !cfg.setup {
        return Ok(()); // setup not requested
    }

    let mut wled = wled::WLed::new();

    wled.load(&cfg.host).await
        .context("Unable to load from WLED")?;

    if let Some(presets) = &cfg.presets {
        for (idx, preset) in presets.into_iter().enumerate() {
            wled.set_preset(idx, &cfg, &preset).await
                .context("Failed to set WLED preset")?;
        }
    }

    if let Some(playlists) = &cfg.playlists {
        for (idx, playlist) in playlists.into_iter().enumerate() {
            wled.set_playlist(idx, &cfg, &playlist).await
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

    // find playlist matching boost amount
    let mut playlist = wled.get_preset(&number_playlist);

    if playlist.is_none() {
        // find playlist matching end number
        playlist = wled.get_preset(&endnum_playlist);
    }

    if playlist.is_none() {
        // find general boost playlist
        playlist = wled.get_preset(&cfg.boost_playlist);
    }

    if let Some(playlist) = playlist {
        println!("Triggering WLED playlist {}", playlist.name);
        wled.run_preset(playlist).await
            .context("Failed to run WLED preset")?;
    }
    else {
        eprintln!("Unable to find WLED playlist matching {}, {}, or {}", number_playlist, endnum_playlist, cfg.boost_playlist.clone());
    }

    Ok(())
}

async fn trigger_effects(config: config::Config, sats: i64) -> Result<Vec<String>> {
    println!("Triggering effects for {} sats", sats);
    let mut triggered = Vec::new();

    if let Some(cfg) = config.wled {
        trigger_wled_effects(cfg, sats).await
            .context("Failed to trigger WLED effects")?;
        triggered.push("WLED".to_string());
    }

    if let Some(cfg) = config.osc {
        println!("Triggering OSC with value {}", sats);

        let osc = osc::Osc::new(cfg.address.clone());
        osc.trigger_for_sats(sats)
            .context("Failed to trigger OSC")?;
        triggered.push("OSC".to_string());
    }

    if let Some(cfg) = config.artnet {
        println!("Triggering Art-Net with value {}", sats);

        let artnet = artnet::ArtNet::new(cfg.address.clone(), cfg.universe);
        artnet?.trigger_for_sats(sats)
            .context("Failed to trigger Art-Net")?;
        triggered.push("Art-Net".to_string());
    }

    Ok(triggered)
}

async fn listen_for_zaps(config: config::Config, tx: tokio::sync::mpsc::Sender<GuiMessage>) {
    let cfg = config.zaps.clone().unwrap();
    
    // Update status to Running
    let _ = tx.send(GuiMessage::UpdateStatus("Zaps".to_string(), ComponentStatus::Running)).await;
    
    let zap = match zaps::Zaps::new(&cfg.relay_addrs, &cfg.naddr).await {
        Ok(z) => z,
        Err(e) => {
            let error_msg = format!("Connection error: {:#}", e);
            eprintln!("Error connecting to zaps: {}", error_msg);
            let _ = tx.send(GuiMessage::UpdateStatus(
                "Zaps".to_string(), 
                ComponentStatus::Error(error_msg)
            )).await;
            return;
        }
    };

    let now = Some(Timestamp::now());

    println!("Waiting for Zaps...");

    match zap.subscribe_zaps(now, |zap: zaps::Zap| {
        let myconfig = config.clone();
        let tx = tx.clone();

        async move {
            println!("Zap: {:#?}", zap);

            let sats = zap.value_msat_total / 1000;
            
            // Trigger effects and get list of triggered effects
            let effects = match trigger_effects(myconfig.clone(), sats).await {
                Ok(effects) => effects,
                Err(e) => {
                    eprintln!("Unable to trigger effects: {:#}", e);
                    Vec::new()
                }
            };
            
            // Send boost received message to GUI with effects
            let _ = tx.send(GuiMessage::BoostReceived("Zaps".to_string(), sats, effects)).await;
        }
    }).await {
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

async fn listen_for_boostboard(config: config::Config, tx: tokio::sync::mpsc::Sender<GuiMessage>) {
    let cfg = config.boostboard.clone().unwrap();
    
    // Update status to Running
    let _ = tx.send(GuiMessage::UpdateStatus("Boostboard".to_string(), ComponentStatus::Running)).await;
    
    let board = match boostboard::BoostBoard::new(&cfg.relay_addr, &cfg.pubkey).await {
        Ok(b) => b,
        Err(e) => {
            let error_msg = format!("Connection error: {:#}", e);
            eprintln!("Error connecting to boostboard: {}", error_msg);
            let _ = tx.send(GuiMessage::UpdateStatus(
                "Boostboard".to_string(), 
                ComponentStatus::Error(error_msg)
            )).await;
            return;
        }
    };

    let now = Some(Timestamp::now());
    let subscription_id = match board.subscribe(now).await {
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

    match board.handle_boosts(subscription_id, |boost: boosts::Boostagram| {
        let myconfig = config.clone();
        let tx = tx.clone();

        async move {
            if boost.action != "boost" {
                return;
            }

            println!("Boost: {:#?}", boost);

            let sats = boost.value_msat_total / 1000;
            
            // Trigger effects and get list of triggered effects
            let effects = match trigger_effects(myconfig.clone(), sats).await {
                Ok(effects) => effects,
                Err(e) => {
                    eprintln!("Unable to trigger effects: {:#}", e);
                    Vec::new()
                }
            };
            
            // Send boost received message to GUI with effects
            let _ = tx.send(GuiMessage::BoostReceived("Boostboard".to_string(), sats, effects)).await;
        }
    }).await {
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

async fn listen_for_nwc(config: config::Config, tx: tokio::sync::mpsc::Sender<GuiMessage>) {
    let cfg = config.nwc.clone().unwrap();
    
    // Update status to Running
    let _ = tx.send(GuiMessage::UpdateStatus("NWC".to_string(), ComponentStatus::Running)).await;
    
    let nwc = match nwc::NWC::new(&cfg.uri).await {
        Ok(n) => n,
        Err(e) => {
            let error_msg = format!("Connection error: {:#}", e);
            eprintln!("Failed to create NWC: {}", error_msg);
            let _ = tx.send(GuiMessage::UpdateStatus(
                "NWC".to_string(), 
                ComponentStatus::Error(error_msg)
            )).await;
            return;
        }
    };

    println!("Waiting for NWC boosts...");

    let last_created_at = Timestamp::now(); // Timestamp::from_secs(1722104476); //Timestamp::now();

    match nwc.subscribe_boosts(last_created_at, |boost: boosts::Boostagram| {
        let myconfig = config.clone();
        let tx = tx.clone();

        async move {
            if boost.action != "boost" {
                return;
            }

            println!("NWC Boost: {:#?}", boost);

            let sats = boost.value_msat_total / 1000;
            
            // Trigger effects and get list of triggered effects
            let effects = match trigger_effects(myconfig.clone(), sats).await {
                Ok(effects) => effects,
                Err(e) => {
                    eprintln!("Unable to trigger effects: {:#}", e);
                    Vec::new()
                }
            };
            
            // Send boost received message to GUI with effects
            let _ = tx.send(GuiMessage::BoostReceived("NWC".to_string(), sats, effects)).await;
        }
    }).await {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting BlinkyBoosts...");

    // Load configuration
    let config = config::load_config()?;

    // Create a tokio runtime
    let rt = tokio::runtime::Runtime::new()?;

    // Create a channel for communication between async tasks and the GUI
    let (tx, mut rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);

    // Spawn async tasks on the tokio runtime
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

    // Spawn the zaps listener
    let config_clone = config.clone();
    let tx_clone = tx.clone();
    rt.spawn(async move {
        listen_for_zaps(config_clone, tx_clone).await;
    });

    // Spawn the boostboard listener
    let config_clone = config.clone();
    let tx_clone = tx.clone();
    rt.spawn(async move {
        listen_for_boostboard(config_clone, tx_clone).await;
    });

    // Spawn the NWC listener
    let config_clone = config.clone();
    let tx_clone = tx.clone();
    rt.spawn(async move {
        listen_for_nwc(config_clone, tx_clone).await;
    });

    // Create a handler for test triggers and message forwarding
    // This task reads from the main channel, handles TestTrigger messages,
    // and forwards other messages to the GUI
    let config_for_tests = config.clone();
    let (gui_tx, gui_rx) = tokio::sync::mpsc::channel::<GuiMessage>(100);

    rt.spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                GuiMessage::TestTrigger(sats) => {
                    println!("Test trigger received for {} sats", sats);
                    let effects = match trigger_effects(config_for_tests.clone(), sats).await {
                        Ok(effects) => effects,
                        Err(e) => {
                            eprintln!("Error triggering test effects: {:#}", e);
                            Vec::new()
                        }
                    };
                    // Send boost received message to GUI with effects
                    let _ = gui_tx.send(GuiMessage::BoostReceived("Test".to_string(), sats, effects)).await;
                },
                other => {
                    // Forward other messages to GUI
                    let _ = gui_tx.send(other).await;
                }
            }
        }
    });

    // Run the GUI on the main thread
    // Pass the main tx so GUI can send test triggers to the handler
    gui::run_gui(tx.clone(), gui_rx)?;

    Ok(())
}