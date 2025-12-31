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
    pub sacn: Option<Sacn>,
    pub wled: Option<WLed>,
    pub toggles: Option<Vec<Toggle>>,
}

/// Common filter fields for boost sources
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct BoostFiltersConfig {
    pub load_since: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub podcasts: Option<Vec<String>>,
    pub episode_guids: Option<Vec<String>>,
    pub event_guids: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct BoostBoard {
    #[serde(default)]
    pub relay_addrs: Vec<String>,
    pub pubkey: String,
    #[serde(flatten)]
    pub filters: BoostFiltersConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct NWC {
    pub uri: String,
    #[serde(flatten)]
    pub filters: BoostFiltersConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Zaps {
    pub relay_addrs: Vec<String>,
    pub naddr: String,
    pub load_since: Option<String>,  // Load zaps since this timestamp (e.g., "2025-01-11 00:00:00")
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OSC {
    pub address: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ArtNet {
    pub broadcast_address: String,
    pub local_address: Option<String>,
    pub universe: Option<u16>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Sacn {
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

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum OscArgValue {
    Int(i64),
    Float(f64),
    String(String),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ToggleOsc {
    pub path: String,
    pub arg_value: OscArgValue,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ToggleArtNet {
    pub universe: Option<u16>,
    pub channel: u16,
    pub value: u8,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ToggleSacn {
    pub universe: Option<u16>,
    pub channel: u16,
    pub value: u8,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ToggleWled {
    pub preset: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Toggle {
    #[serde(default)]
    pub threshold: i64,
    pub output: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub use_total: bool,  // If true, trigger based on cumulative sat total instead of individual boost amount
    #[serde(default = "default_true")]
    pub trigger_multiple: bool,  // If true, trigger for every multiple of the threshold (e.g., 250k triggers at 250k, 500k, 750k, etc.)
    pub endswith_range: Option<(u8, u8)>,  // If set, only trigger when the last digit of sats is within this range (inclusive), e.g., (0, 3) for 0-3

    // Protocol-specific configuration
    pub osc: Option<ToggleOsc>,
    pub artnet: Option<ToggleArtNet>,
    pub sacn: Option<ToggleSacn>,
    pub wled: Option<ToggleWled>,
}

fn default_true() -> bool {
    true
}

pub fn load_config() -> Result<Config> {
    let filename = "./config.toml";

    let contents = fs::read_to_string(filename)
        .context(format!("Failed to read config file: {}", filename))?;
    let cfg: Config = toml::from_str(&contents)
        .context("Failed to parse config file as TOML")?;

    Ok(cfg)
}
