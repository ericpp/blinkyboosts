use serde_derive::{Deserialize, Serialize};
use std::fs;
use anyhow::{Context, Result};
use toml;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub nwc: Option<NWC>,
    pub boostboard: Option<BoostBoard>,
    pub zaps: Option<Zaps>,
    pub osc: Option<OSC>,
    pub artnet: Option<ArtNet>,
    pub wled: Option<WLed>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BoostBoard {
    pub relay_addr: String,
    pub pubkey: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct NWC {
    pub uri: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Zaps {
    pub relay_addrs: Vec<String>,
    pub naddr: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OSC {
    pub address: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ArtNet {
    pub broadcast_address: String,
    pub universe: Option<u16>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLed {
    pub host: String,
    pub boost_playlist: String,
    pub brightness: u64,
    pub segments: Option<Vec<WLedSegment>>,
    pub presets: Option<Vec<WLedPreset>>,
    pub playlists: Option<Vec<WLedPlaylist>>,
    pub setup: bool,
    pub force: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedSegment {
    pub name: String,
    pub start: u64,
    pub stop: u64,
    pub reverse: Option<bool>,
    pub grouping: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedPreset {
    pub name: String,
    pub speed: Option<u64>,
    pub intensity: Option<u64>,
    pub colors: Vec<Vec<u64>>,
    pub colors2: Option<Vec<Vec<u64>>>,
    pub colors3: Option<Vec<Vec<u64>>>,
    pub effects: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedPlaylist {
    pub name: String,
    pub presets: Vec<String>,
    pub durations: Vec<u64>,
    pub transitions: Vec<u64>,
    pub repeat: u64,
    pub end: String,
}

pub fn load_config() -> Result<Config> {
    let filename = "./config.toml";

    let contents = fs::read_to_string(filename)
        .context(format!("Failed to read config file: {}", filename))?;
    let cfg: Config = toml::from_str(&contents)
        .context("Failed to parse config file as TOML")?;

    Ok(cfg)
}

pub fn save_config(config: &Config) -> Result<()> {
    let filename = "./config.toml";
    let toml_string = toml::to_string(config)
        .context("Failed to serialize config to TOML")?;
    fs::write(filename, toml_string)
        .context(format!("Failed to write config to file: {}", filename))?;
    Ok(())
}