use crate::config;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::Value;
use std::collections::HashMap;
use anyhow::{Context, Result};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct JsonPreset {
    pub n: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub psave: Option<u64>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub seg: Vec<JsonSegmentEnum>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub playlist: Option<JsonPlaylist>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
enum JsonSegmentEnum {
    Segment(JsonSegment),
    Empty { stop: u64 },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Default)]
struct JsonSegment {
    #[serde(default)] pub id: u64,
    #[serde(default)] pub start: u64,
    #[serde(default)] pub stop: u64,
    #[serde(default = "default_one")] pub grp: u64,
    #[serde(default)] pub spc: u64,
    #[serde(default)] pub of: u64,
    #[serde(default = "default_true")] pub on: bool,
    #[serde(default)] pub frz: bool,
    #[serde(default)] pub bri: u64,
    #[serde(default = "default_cct")] pub cct: u64,
    #[serde(default)] pub set: u64,
    #[serde(default)] pub n: String,
    #[serde(default)] pub col: Vec<Vec<u64>>,
    #[serde(default)] pub fx: u64,
    #[serde(default = "default_128")] pub sx: u64,
    #[serde(default = "default_128")] pub ix: u64,
    #[serde(default)] pub pal: u64,
    #[serde(default)] pub c1: u64,
    #[serde(default)] pub c2: u64,
    #[serde(default)] pub c3: u64,
    #[serde(default = "default_true")] pub sel: bool,
    #[serde(default)] pub rev: bool,
    #[serde(default)] pub mi: bool,
    #[serde(default)] pub o1: bool,
    #[serde(default)] pub o2: bool,
    #[serde(default)] pub o3: bool,
    #[serde(default)] pub si: u64,
    #[serde(default)] pub m12: u64,
}

fn default_one() -> u64 { 1 }
fn default_true() -> bool { true }
fn default_128() -> u64 { 128 }
fn default_cct() -> u64 { 127 }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct JsonPlaylist {
    pub ps: Vec<u64>,
    pub dur: Vec<u64>,
    pub transition: Vec<u64>,
    pub repeat: u64,
    pub end: u64,
    pub r: u64,
}

#[derive(Debug)]
pub struct WLed {
    host: String,
    presets: Vec<Preset>,
    effects: Vec<Effect>,
    raw_presets: HashMap<u64, JsonPreset>,
}

impl WLed {
    pub fn new() -> Self {
        Self {
            host: "".to_string(),
            presets: vec![],
            effects: vec![],
            raw_presets: HashMap::new(),
        }
    }

    pub async fn load(&mut self, host: &str) -> Result<()> {
        self.host = String::from(host);
        self.load_effects().await
            .context("Failed to load WLED effects")?;
        self.load_presets().await
            .context("Failed to load WLED presets")?;
        Ok(())
    }

    pub async fn load_effects(&mut self) -> Result<()> {
        self.effects = get_effects(&self.host).await
            .context("Failed to get WLED effects")?;
        Ok(())
    }

    pub async fn load_presets(&mut self) -> Result<()> {
        self.presets = get_presets(&self.host).await
            .context("Failed to get WLED presets")?;
        self.raw_presets = get_raw_presets(&self.host).await
            .context("Failed to get raw WLED presets")?;
        Ok(())
    }

    pub fn get_effect(&self, name: &str) -> Option<Effect> {
        self.effects.clone().into_iter().find(|eff| eff.name == name)
    }

    pub fn get_effect_id(&self, name: &str) -> u64 {

        let effect_id = match self.get_effect(name) {
            Some(eff) => eff.id,
            _ => 0,
        };

        effect_id
    }

    pub fn get_preset(&self, name: &str) -> Option<Preset> {
        self.presets.clone().into_iter().find(|ps| ps.name == name)
    }

    pub fn get_preset_id(&self, name: &str) -> u64 {
        self.presets.iter()
            .find(|ps| ps.name == name)
            .map(|ps| ps.id)
            .unwrap_or_else(|| self.presets.iter().map(|ps| ps.id).max().map(|m| m + 1).unwrap_or(1))
    }

    pub async fn set_preset(&mut self, index: usize, config: &config::WLed, preset: &config::WLedPreset) -> Result<bool> {
        // let preset_id = self.get_preset_id(&preset.name);
        let preset_id = (index + 1) as u64;
        let segments = config.segments.as_ref()
            .context("No segments defined in configuration")?;

        let mut segs = vec![];

        let black = vec![0, 0, 0];
        for s in 0..32 {
            if s < preset.colors.len() {
                let segment = &segments[s];
                let get_color = |colors: &Option<Vec<Vec<u64>>>| {
                    colors.as_ref().and_then(|c| c.get(s).cloned()).unwrap_or_else(|| black.clone())
                };

                segs.push(JsonSegmentEnum::Segment(JsonSegment {
                    id: s as u64,
                    start: segment.start,
                    stop: segment.stop,
                    grp: segment.grouping.unwrap_or(1),
                    bri: config.brightness,
                    n: segment.name.clone(),
                    col: vec![preset.colors[s].clone(), get_color(&preset.colors2), get_color(&preset.colors3)],
                    fx: self.get_effect_id(&preset.effects[s]),
                    sx: preset.speed.unwrap_or(128),
                    ix: preset.intensity.unwrap_or(128),
                    rev: segment.reverse.unwrap_or(false),
                    ..Default::default()
                }));
            } else {
                segs.push(JsonSegmentEnum::Empty { stop: 0 });
            }
        }

        let json_preset = JsonPreset {
            n: preset.name.clone(),
            psave: Some(1),
            seg: segs,
            playlist: None,
        };

        let mut changed = true;

        if let Some(existing) = self.raw_presets.get(&preset_id) {
            changed = !self.compare_preset(existing);
        }

        if changed || config.force {
            let url = format!("http://{}/json/state", self.host);
            let client = reqwest::Client::new();

            let json = json!({
                "on": true,
                "bri": config.brightness,
                "v": true,
                "ps": preset_id,
                "psave": 1,
                "n": preset.name,
                "seg": json_preset.seg,
            });

            let res = client.post(&url)
                .json(&json)
                .send()
                .await
                .context("Failed to send preset to WLED")?;

            if !res.status().is_success() {
                return Err(anyhow::anyhow!("Failed to set preset: HTTP {}", res.status()));
            }

            sleep(Duration::from_millis(500)).await;

            self.load_presets().await
                .context("Failed to reload presets after setting new preset")?;
        }

        Ok(changed)
    }

    fn compare_preset(&self, preset: &JsonPreset) -> bool {
        let id = preset.psave.unwrap();
        if let Some(existing) = self.raw_presets.get(&id) {
            preset == existing
        } else {
            false
        }
    }

    pub async fn set_playlist(&mut self, index: usize, config: &config::WLed, playlist: &config::WLedPlaylist) -> Result<bool> {
        let preset_id = (index + 100) as u64;
        let end_playlist_id = self.get_preset_id(&playlist.end);

        let presets = playlist.presets.clone().into_iter()
            .map(|ps| self.get_preset_id(&ps))
            .collect();

        let json = JsonPreset {
            psave: Some(preset_id),
            n: playlist.name.clone(),
            seg: vec![],
            playlist: Some(JsonPlaylist {
                ps: presets,
                dur: playlist.durations.clone(),
                transition: playlist.transitions.clone(),
                repeat: playlist.repeat,
                end: end_playlist_id,
                r: 0,
            }),
        };

        if !config.force && self.compare_preset(&json) {
            return Ok(false);
        }

        // convert to state object (different than preset)
        let state = json!({
            "psave": preset_id,
            "on": true,
            "o": true,
            "n": json.n,
            "v": true,
            "playlist": json.playlist,
        });

        if let Ok(()) = set_state(&self.host, state).await {
            self.load_presets().await?;
        }

        Ok(true)
    }

    pub async fn run_preset(&self, preset: Preset) -> Result<()> {
        let id = self.get_preset_id(&preset.name);
        self.run_preset_id(id).await
            .context(format!("Failed to run preset: {}", preset.name))
    }

    pub async fn run_preset_id(&self, preset_id: u64) -> Result<()> {
        set_state(&self.host, json!({"ps": preset_id})).await
    }

    pub async fn trigger_toggle(toggle: &crate::config::Toggle, host: &str) -> Result<()> {
        let wled_config = toggle.wled.as_ref()
            .ok_or_else(|| anyhow::anyhow!("WLED toggle missing 'wled' configuration"))?;

        let mut wled = WLed::new();
        wled.load(host).await
            .context("Failed to load WLED for toggle")?;

        if let Some(preset) = wled.get_preset(&wled_config.preset) {
            wled.run_preset(preset).await
                .context(format!("Failed to run WLED preset: {}", wled_config.preset))
        } else {
            Err(anyhow::anyhow!("WLED preset not found: {}", wled_config.preset))
        }
    }
}

async fn get_effects(host: &str) -> Result<Vec<Effect>> {
    let addr = format!("http://{}/json/effects", host);
    let resp = reqwest::get(&addr).await
        .context(format!("Failed to connect to WLED at {}", addr))?
        .json::<Value>()
        .await
        .context("Failed to parse effects JSON response")?;

    let result = resp.as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected array of effects but got: {}", resp))?;

    let effects = result.iter().enumerate()
        .map(|(id, name)| {
            let name_str = name.as_str()
                .ok_or_else(|| anyhow::anyhow!("Effect name is not a string: {}", name))?;

            Ok(Effect {
                id: id as u64,
                name: name_str.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(effects)
}

async fn get_raw_presets(host: &str) -> Result<HashMap<u64, JsonPreset>> {
    let addr = format!("http://{}/presets.json", host);
    let resp = reqwest::get(&addr).await
        .context(format!("Failed to connect to WLED at {}", addr))?
        .json::<HashMap<u64, Value>>()
        .await
        .context("Failed to parse presets JSON response")?;

    let result = resp.into_iter()
        .filter(|(id, _)| *id != 0)
        .map(|(id, val)| {
            let value: JsonPreset = serde_json::from_value(val)
                .context(format!("Failed to parse preset JSON for ID {}", id))?;
            Ok((id, value))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    Ok(result)
}

async fn get_presets(host: &str) -> Result<Vec<Preset>> {
    let map = get_raw_presets(host).await
        .context("Failed to get raw presets")?;

    let pls = map.into_iter().map(
        |(id, preset)| Preset {
            id,
            name: preset.n,
        }
    ).collect();

    Ok(pls)
}

async fn set_state(host: &str, json: Value) -> Result<()> {
    let addr = format!("http://{}/json/state", host);
    let json_str = json.to_string();

    println!("{} {}", addr, json_str);

    let client = reqwest::Client::new();
    let res = client.post(&addr)
        .body(json_str)
        .send()
        .await
        .context(format!("Failed to send state to WLED at {}", addr))?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!("Failed to set state: HTTP {}", res.status()));
    }

    let body = res.text().await
        .context("Failed to read response body")?;

    println!("Response: {}", body);

    Ok(())
}
