use serde_derive::Deserialize;
use std::error::Error;
use std::fs;
use toml;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub nwc: Option<NWC>,
    pub boostboard: Option<BoostBoard>,
    pub zaps: Option<Zaps>,
    pub osc: Option<OSC>,
    pub wled: Option<WLed>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BoostBoard {
    pub relay_addr: String,
    pub pubkey: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NWC {
    pub uri: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Zaps {
    pub relay_addrs: Vec<String>,
    pub naddr: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct OSC {
    pub address: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WLed {
    pub host: String,
    pub boost_playlist: String,
    pub brightness: u64,
    pub leds: Option<u64>,
    pub segments: Option<u64>,
    pub presets: Option<Vec<WLedPreset>>,
    pub playlists: Option<Vec<WLedPlaylist>>,
    pub setup: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WLedPreset {
    pub id: u64,
    pub name: String,
    pub effect: Option<u64>,
    pub speed: Option<u64>,
    pub intensity: Option<u64>,
    pub colors: Vec<Vec<u64>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WLedPlaylist {
    pub id: u64,
    pub name: String,
    pub presets: Vec<u64>,
    pub durations: Vec<u64>,
    pub transitions: Vec<u64>,
    pub repeat: u64,
    pub end: u64,
}

pub fn load_config() -> Result<Config, Box<dyn Error>> {
    let filename = "./config.toml";

    let contents = fs::read_to_string(filename)?;
    let cfg: Config = toml::from_str(&contents)?;

    Ok(cfg)
}