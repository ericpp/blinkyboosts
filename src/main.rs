use nostr_sdk::Timestamp;

use std::error::Error;

use tokio;

mod boostboard;
mod boosts;
mod config;
mod nwc;
mod osc;
mod wled;
mod zaps;


pub async fn trigger_effects(config: config::Config, sats: i64) -> Result<(), Box<dyn Error>> {

    if let Some(cfg) = config.wled {
        let playlist = wled::get_playlist(&cfg.host, cfg.playlist.clone()).await?;

        if playlist.is_some() {
            let playlist = playlist.unwrap();
            println!("Triggering WLED playlist {}", playlist.name);
            wled::run_playlist(cfg.host, playlist).await?;
        }
        else {
            eprintln!("Unable to find WLED playlist matching: {}", cfg.playlist.clone());
        }
    }

    if let Some(cfg) = config.osc {
        println!("Triggering OSC with value {}", sats);

        let osc = osc::Osc::new(cfg.address.clone());
        osc.trigger_for_sats(sats)?;
    }

    Ok(())
}

async fn listen_for_zaps(config: config::Config) {
    let cfg = config.zaps.clone().unwrap();
    let zap = zaps::Zaps::new(&cfg.relay_addrs, &cfg.naddr)
        .await
        .expect("Error connecting to zaps");

    let now = Some(Timestamp::now());

    println!("Waiting for Zaps...");

    zap.subscribe_zaps(now, |zap: zaps::Zap| {
        let myconfig = config.clone();

        async move {
            println!("Zap: {:#?}", zap);

            let sats = zap.value_msat_total / 1000;

            if let Err(e) = trigger_effects(myconfig.clone(), sats).await {
                eprintln!("Unable to trigger effects: {}", e);
            }
        }

    }).await.expect("Error handling events");
}

async fn listen_for_boostboard(config: config::Config) {
    let cfg = config.boostboard.clone().unwrap();
    let board = boostboard::BoostBoard::new(&cfg.relay_addr, &cfg.pubkey)
        .await
        .expect("Error connecting to boostboard");

    let now = Some(Timestamp::now());
    let subscription_id = board.subscribe(now).await.expect("Error subscribing to board");

    println!("Waiting for Boostboard boosts...");

    board.handle_boosts(subscription_id, |boost: boosts::Boostagram| {
        let myconfig = config.clone();

        async move {
            if boost.action != "boost" {
                return;
            }

            println!("Boost: {:#?}", boost);

            let sats = boost.value_msat_total / 1000;

            if let Err(e) = trigger_effects(myconfig.clone(), sats).await {
                eprintln!("Unable to trigger effects: {}", e);
            }
        }

    }).await.expect("Error handling events");
}

async fn listen_for_nwc(config: config::Config) {
    let cfg = config.nwc.clone().unwrap();
    let nwc = nwc::NWC::new(&cfg.uri).expect("Failed to create NWC");

    println!("Waiting for NWC boosts...");

    let last_created_at = Timestamp::now(); // Timestamp::from_secs(1722104476); //Timestamp::now();

    nwc.subscribe_boosts(last_created_at, |boost: boosts::Boostagram| {
        let myconfig = config.clone();

        async move {
            if boost.action != "boost" {
                return;
            }

            println!("NWC Boost: {:#?}", boost);

            let sats = boost.value_msat_total / 1000;

            if let Err(e) = trigger_effects(myconfig.clone(), sats).await {
                eprintln!("Unable to trigger effects: {}", e);
            }
        }

    }).await.expect("Error handling events");
}


#[tokio::main]
async fn main() {
    println!("Starting...");

    let config = config::load_config().expect("Unable to load config");
    let mut tasks = Vec::new();

    if config.zaps.is_some() {
        tasks.push(tokio::spawn(listen_for_zaps(config.clone())));
    }

    if config.boostboard.is_some() {
        tasks.push(tokio::spawn(listen_for_boostboard(config.clone())));
    }

    if config.nwc.is_some() {
        tasks.push(tokio::spawn(listen_for_nwc(config.clone())));
    }

    for task in tasks {
        task.await.unwrap();
    }
}