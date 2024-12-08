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

async fn setup_effects(config: config::Config) -> Result<(), Box<dyn Error>> {

    if config.wled.is_none() {
        return Ok(());
    }

    let cfg = config.wled.unwrap();

    if !cfg.setup {
        return Ok(()); // setup not requested
    }

    let mut wled = wled::WLed::new();

    if let Err(err) = wled.load(&cfg.host).await {
        eprintln!("Unable to load from WLED: {:#?}", err);
        return Err(err);
    }

    if let Some(presets) = &cfg.presets {
        // let map: HashMap<String, wled::Preset> = current_presets.into_iter().map(|ps| (ps.name.clone(), ps)).collect();
        for preset in presets {
            wled.set_preset(&cfg, &preset).await?;
        }
    }

    if let Some(playlists) = &cfg.playlists {
        for playlist in playlists {
            wled.set_playlist(&playlist).await?;
        }
    }

    Ok(())
}

async fn trigger_wled_effects(cfg: config::WLed, sats: i64) -> Result<(), Box<dyn Error>> {
    let number_playlist = format!("BOOST-{}", sats);

    let endnum = sats.to_string().chars().last().unwrap();
    let endnum_playlist = format!("BOOST-{}", endnum);

    let mut wled = wled::WLed::new();

    wled.load(&cfg.host).await?;

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
        wled.run_preset(playlist).await?;
    }
    else {
        eprintln!("Unable to find WLED playlist matching {}, {}, or {}", number_playlist, endnum_playlist, cfg.boost_playlist.clone());
    }

    Ok(())
}

async fn trigger_effects(config: config::Config, sats: i64) -> Result<(), Box<dyn Error>> {

    println!("Triggering effects for {} sats", sats);

    if let Some(cfg) = config.wled {
        trigger_wled_effects(cfg, sats).await?;
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
    let nwc = nwc::NWC::new(&cfg.uri).await.expect("Failed to create NWC");

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

    if let Err(err) = setup_effects(config.clone()).await {
        eprintln!("Error setting up effects: {}", err);
    }

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