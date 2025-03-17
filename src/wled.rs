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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum JsonSegmentEnum {
    Segment(JsonSegment),
    Empty { stop: u64 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JsonSegment {
    pub id: u64,
    pub start: u64,
    pub stop: u64,
    pub grp: u64,
    pub spc: u64,
    pub of: u64,
    pub on: bool,
    pub frz: bool,
    pub bri: u64,
    pub cct: u64,
    pub set: u64,
    pub n: String,
    pub col: Vec<Vec<u64>>,
    pub fx: u64,
    pub sx: u64,
    pub ix: u64,
    pub pal: u64,
    pub c1: u64,
    pub c2: u64,
    pub c3: u64,
    pub sel: bool,
    pub rev: bool,
    pub mi: bool,
    pub o1: bool,
    pub o2: bool,
    pub o3: bool,
    pub si: u64,
    pub m12: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
        let lists = self.presets.clone();
        let found = lists.clone().into_iter().find(|ps| ps.name == name);
        let max = lists.clone().into_iter().map(|ps| ps.id).max();

        if let Some(found) = found {
            return found.id
        }

        if let Some(max) = max {
            return max + 1
        }

        1
    }

    pub async fn set_preset(&mut self, index: usize, config: &config::WLed, preset: &config::WLedPreset) -> Result<bool> {
        // let preset_id = self.get_preset_id(&preset.name);
        let preset_id = (index + 1) as u64;
        let segments = config.segments.as_ref()
            .context("No segments defined in configuration")?;

        let mut segs = vec![];

        for s in 0..32 {
            if s < preset.colors.len() {
                let segment = segments[s].clone();
                let pset = preset.clone();

                let colors1 = pset.colors[s].clone();
                let colors2 = if let Some(colors) = &pset.colors2 {
                    if s < colors.len() {
                        colors[s].clone()
                    } else {
                        vec![0, 0, 0]
                    }
                } else {
                    vec![0, 0, 0]
                };

                let colors3 = if let Some(colors) = &pset.colors3 {
                    if s < colors.len() {
                        colors[s].clone()
                    } else {
                        vec![0, 0, 0]
                    }
                } else {
                    vec![0, 0, 0]
                };

                let effect_id = self.get_effect_id(&pset.effects[s]);

                let seg = JsonSegment {
                    id: s as u64,
                    start: segment.start,
                    stop: segment.stop,
                    grp: segment.grouping.unwrap_or(1),
                    spc: 0,
                    of: 0,
                    on: true,
                    frz: false,
                    bri: config.brightness,
                    cct: 127,
                    set: 0,
                    n: segment.name.clone(),
                    col: vec![colors1, colors2, colors3],
                    fx: effect_id,
                    sx: pset.speed.unwrap_or(128),
                    ix: pset.intensity.unwrap_or(128),
                    pal: 0,
                    c1: 0,
                    c2: 0,
                    c3: 0,
                    sel: true,
                    rev: segment.reverse.unwrap_or(false),
                    mi: false,
                    o1: false,
                    o2: false,
                    o3: false,
                    si: 0,
                    m12: 0,
                };

                segs.push(JsonSegmentEnum::Segment(seg));
            }
            else {
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
        let exists = self.raw_presets.get(&id);

        if exists.is_none() {
            return false;
        }

        let exists = exists.unwrap();

        compare_presets(preset, exists)
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
            .context(format!("Failed to set state for preset ID: {}", preset_id))
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


fn compare_presets(preset: &JsonPreset, compare_to: &JsonPreset) -> bool {
    if preset.n != compare_to.n {
        return false;
    }

    if preset.seg.len() != compare_to.seg.len() {
        return false;
    }

    for (item, compare) in preset.seg.iter().zip(compare_to.seg.iter()) {
        if !compare_segments(&item, compare) {
            return false;
        }
    }

    match (preset.playlist.as_ref(), compare_to.playlist.as_ref()) {
        (Some(pl1), Some(pl2)) => compare_playlists(pl1, pl2),
        (None, None) => true,
        _ => false,
    }
}

fn compare_segments(segment: &JsonSegmentEnum, compare_to: &JsonSegmentEnum) -> bool {
    let (seg1, seg2) = match (segment, compare_to) {
        (JsonSegmentEnum::Empty { .. }, JsonSegmentEnum::Empty { .. }) => (None, None),
        (JsonSegmentEnum::Segment(s), JsonSegmentEnum::Empty { .. }) => (Some(s), None),
        (JsonSegmentEnum::Empty { .. }, JsonSegmentEnum::Segment(s)) => (None, Some(s)),
        (JsonSegmentEnum::Segment(s1), JsonSegmentEnum::Segment(s2)) => (Some(s1), Some(s2)),
    };

    if seg1.is_none() && seg2.is_none() {
        return true; // assume same
    }

    if seg1.is_none() || seg2.is_none() {
        return false; // different
    }

    let seg1 = seg1.unwrap();
    let seg2 = seg2.unwrap();

    if seg1.n != seg2.n || seg1.rev != seg2.rev || seg1.grp != seg2.grp ||
        seg1.fx != seg2.fx || seg1.sx != seg2.sx || seg1.ix != seg2.ix ||
        seg1.frz != seg2.frz || seg1.bri != seg2.bri || seg1.sel != seg2.sel {

        return false;
    }

    for (col1, col2) in seg1.col.iter().zip(&seg2.col) {
        if col1[0] != col2[0] || col1[1] != col2[1] || col1[2] != col2[2] {
            return false;
        }
    }

    true
}


fn compare_playlists(playlist: &JsonPlaylist, compare_to: &JsonPlaylist) -> bool {

    if playlist.repeat != compare_to.repeat || playlist.end != compare_to.end || playlist.r != compare_to.r {
        return false;
    }

    if playlist.ps.len() != compare_to.ps.len() {
        return false;
    }

    for (ps1, ps2) in playlist.ps.iter().zip(compare_to.ps.iter()) {
        if ps1 != ps2 {
            return false;
        }
    }

    if playlist.dur.len() != compare_to.dur.len() {
        return false;
    }

    for (dur1, dur2) in playlist.dur.iter().zip(compare_to.dur.iter()) {
        if dur1 != dur2 {
            return false;
        }
    }

    if playlist.transition.len() != compare_to.transition.len() {
        return false;
    }

    for (tran1, tran2) in playlist.transition.iter().zip(compare_to.transition.iter()) {
        if tran1 != tran2 {
            return false;
        }
    }

    true
}