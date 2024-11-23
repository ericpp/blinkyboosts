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
    pub playlist: String,
}

pub fn load_config() -> Result<Config, Box<dyn Error>> {
    let filename = "./config.toml";

    let contents = fs::read_to_string(filename)?;
    let cfg: Config = toml::from_str(&contents)?;

    Ok(cfg)
}