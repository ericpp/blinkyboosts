use crate::config;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::value::Value;
use std::collections::HashMap;
use std::error::Error;
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
    pub n: String,
    pub start: Option<u64>,
    pub stop: Option<u64>,
    pub rev: bool,
    pub grp: u64,
    pub fx: u64,
    pub sx: u64,
    pub ix: u64,
    pub col: Vec<Vec<u64>>,
    pub frz: bool,
    pub bri: u64,
    pub sel: bool,
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

    pub async fn load(&mut self, host: &str) -> Result<(), Box<dyn Error>> {
        self.host = String::from(host);
        self.load_effects().await?;
        self.load_presets().await?;
        Ok(())
    }

    pub async fn load_effects(&mut self) -> Result<(), Box<dyn Error>> {
        self.effects = get_effects(&self.host).await?;
        Ok(())
    }

    pub async fn load_presets(&mut self) -> Result<(), Box<dyn Error>> {
        self.presets = get_presets(&self.host).await?;
        self.raw_presets = get_raw_presets(&self.host).await?;
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

    pub async fn set_preset(&mut self, config: &config::WLed, preset: &config::WLedPreset) -> Result<bool, Box<dyn Error>> {
        let preset_id = self.get_preset_id(&preset.name);
        let segments = config.segments.as_ref().unwrap();

        let mut segs = vec![];

        for s in 0..preset.colors.len() {
            let segment = segments[s].clone();
            let pset = preset.clone();

            let colors1 = pset.colors[s].clone();
            let colors2 = match &pset.colors2 {
                Some(col) => col[s].clone(),
                None => vec![0, 0, 0],
            };
            let colors3 = match &pset.colors3 {
                Some(col) => col[s].clone(),
                None => vec![0, 0, 0],
            };

            let effect_id = self.get_effect_id(&pset.effects[s]);

            segs.push(JsonSegmentEnum::Segment(JsonSegment {
                n: segment.name,
                start: Some(segment.start),
                stop: Some(segment.stop),
                bri: config.brightness,
                rev: segment.reverse.unwrap_or(false),
                grp: segment.grouping.unwrap_or(1),
                fx: effect_id,
                sx: match pset.speed {
                    Some(val) => val,
                    None => 128,
                },
                ix: match pset.intensity {
                    Some(val) => val,
                    None => 128,
                },
                col: vec![colors1, colors2, colors3],
                frz: false, // freeze
                sel: true, // selected
            }));
        }

        let json = JsonPreset {
            psave: Some(preset_id),
            n: preset.name.clone(),
            seg: segs,
            playlist: None,
        };

        if self.compare_preset(&json) {
            return Ok(false);
        }

        let json = json!({
            "psave": preset_id,
            "n": json.n,
            "bri": config.brightness,
            "seg": json.seg,
        });

        if let Ok(()) = set_state(&self.host, json).await {
            self.load_presets().await?;
        }

        Ok(true)
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

    pub async fn set_playlist(&mut self, playlist: &config::WLedPlaylist) -> Result<bool, Box<dyn Error>> {
        let preset_id = self.get_preset_id(&playlist.name);
        let end_playlist_id = self.get_preset_id(&playlist.end);

        let mut presets = vec![];

        for preset in &playlist.presets {
            presets.push(self.get_preset_id(&preset));
        }

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

        if self.compare_preset(&json) {
            return Ok(false);
        }

        // convert to state object (different than preset)
        let json = json!({
            "psave": preset_id,
            "on": true,
            "o": true,
            "n": json.n,
            "v": true,
            "playlist": json.playlist,
        });

        if let Ok(()) = set_state(&self.host, json).await {
            self.load_presets().await?;
        }

        Ok(true)
    }

    pub async fn run_preset(&self, preset: Preset) -> Result<(), Box<dyn Error>> {
        let id = self.get_preset_id(&preset.name);
        self.run_preset_id(id).await
    }

    pub async fn run_preset_id(&self, preset_id: u64) -> Result<(), Box<dyn Error>> {
        set_state(&self.host, json!({"ps": preset_id})).await
    }
}

async fn get_effects(host: &str) -> Result<Vec<Effect>, Box<dyn Error>> {
    let addr = format!("http://{}/json/effects", host);
    let resp = reqwest::get(addr).await?
        .json::<Value>()
        .await?;

    let result = resp.as_array();

    if result.is_none() {
        return Ok(vec![]);
    }

    let result = result.unwrap();

    let effects = result.into_iter().enumerate().map(
        |(id, name)| Effect {
            id: id.try_into().unwrap(),
            name: name.as_str().unwrap().to_string(),
        }
    ).collect();

    Ok(effects)
}

async fn get_raw_presets(host: &str) -> Result<HashMap<u64, JsonPreset>, Box<dyn Error>> {
    let addr = format!("http://{}/presets.json", host);
    let resp = reqwest::get(addr).await?
        .json::<HashMap<u64, Value>>()
        .await?;

    let result = resp.into_iter()
        .filter(|(id, _)| *id != 0)
        .map(|(id, val)| {
            let value: JsonPreset = serde_json::from_value(val).unwrap();
            (id, value)
        })
        .collect();

    Ok(result)
}

async fn get_presets(host: &str) -> Result<Vec<Preset>, Box<dyn Error>> {
    let map = get_raw_presets(host).await?;

    let pls = map.into_iter().map(
        |(id, preset)| Preset {
            id,
            name: preset.n,
        }
    ).collect();

    Ok(pls)
}

async fn set_state(host: &str, json: Value) -> Result<(), Box<dyn Error>> {
    let addr = format!("http://{}/json/state", host);
    let json = json.to_string();

    println!("{} {}", addr, json);

    let client = reqwest::Client::new();
    let res = client.post(addr)
        .body(json)
        .send()
        .await?;

    let body = res.text().await?;

    println!("body: {:#?}", body);
    println!();

    sleep(Duration::from_millis(1000)).await;

    Ok(())
}


fn compare_presets(preset: &JsonPreset, compare_to: &JsonPreset) -> bool {
    if preset.n != compare_to.n {
        return false;
    }

    let mut same = true;

    for (idx, item) in preset.seg.clone().into_iter().enumerate() {
        let compare = &compare_to.seg[idx];
        same = same && compare_segments(&item, compare);
    }

    if preset.playlist.is_some() && compare_to.playlist.is_none() {
        return false;
    }

    if preset.playlist.is_none() && compare_to.playlist.is_some() {
        return false;
    }

    if let Some(pl1) = &preset.playlist {
        if let Some(pl2) = &compare_to.playlist {
            same = same && compare_playlists(&pl1, &pl2);
        }
    }

    same
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

    for (idx, col1) in seg1.col.clone().into_iter().enumerate() {
        let col2 = &seg2.col[idx];

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

    let mut same = true;

    for (idx, ps1) in playlist.ps.clone().into_iter().enumerate() {
        let ps2 = compare_to.ps[idx];
        same = same && ps1 == ps2;
    }

    for (idx, dur1) in playlist.dur.clone().into_iter().enumerate() {
        let dur2 = compare_to.dur[idx];
        same = same && dur1 == dur2;
    }

    for (idx, tran1) in playlist.transition.clone().into_iter().enumerate() {
        let tran2 = compare_to.transition[idx];
        same = same && tran1 == tran2;
    }

    same
}