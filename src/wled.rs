use crate::config;
use reqwest;
use serde_json::value;
use std::error::Error;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: u64,
    pub name: String,
}

pub async fn get_presets(host: &str) -> Result<Vec<Preset>, Box<dyn Error>> {
    let addr = format!("http://{}/presets.json", host);
    let resp = reqwest::get(addr).await?
        .json::<value::Value>()
        .await?;

    let mut pls = Vec::new();
    let map = resp.as_object();

    if map.is_none() {
        return Ok(pls);
    }

    let map = map.unwrap();

    for (key, value) in map.into_iter() {
        // println!("{} {}", key, value);

        let id = key.parse::<u64>()?;
        let name = value["n"].as_str().unwrap_or_default().to_string();

        let item = Preset { id, name };

        pls.push(item);
    }

    Ok(pls)
}

pub async fn get_preset(host: &str, name: String) -> Result<Option<Preset>, Box<dyn Error>> {
    let lists = get_presets(host).await?;

    for list in lists {
        if list.name == name {
            return Ok(Some(list))
        }
    }

    Ok(None)
}

pub async fn run_preset(host: String, preset: Preset) -> Result<(), Box<dyn Error>> {
// curl -vvvv -H "Content-type: application/json" -X POST -d '{"ps":8}' http://192.168.2.84/json/state
    let addr = format!("http://{}/json/state", host);
    let json = format!(r#"{{"ps":{}}}"#, preset.id);

    let client = reqwest::Client::new();
    let _res = client.post(addr)
        .body(json)
        .send()
        .await?;

    Ok(())
}



pub async fn set_preset(host: &str, config: &config::WLed, preset: &config::WLedPreset) -> Result<(), Box<dyn Error>> {
    let mut segs = vec![];
    let mut start = 0;

    let num = config.leds.unwrap_or(0) / config.segments.unwrap_or(3);

    for s in 0..preset.colors.len() {
        let stop = start + num;

        segs.push(json!({
            "start": start,
            "stop": stop,
            "len": num,
            "fx": match preset.effect {
                Some(val) => val,
                None => 0,
            },
            "sx": match preset.speed {
                Some(val) => val,
                None => 128,
            },
            "ix": match preset.intensity {
                Some(val) => val,
                None => 128,
            },
            "col": [preset.colors[s], [0,0,0], [0,0,0]],
        }));

        start = stop
    }

    let json = json!({
        "psave": preset.id,
        "n": preset.name,
        "bri": config.brightness,
        "ib": true,
        "sb": true,
        "seg": segs,
    });

    let addr = format!("http://{}/json/state", host);
    let json = json.to_string();

    println!("{:#?} {:#?}", addr, json);

    let client = reqwest::Client::new();
    let res = client.post(addr)
        .body(json)
        .send()
        .await?;

    let body = res.text().await?;
    println!("body: {:#?}", body);

    Ok(())
}

pub async fn set_playlist(host: &str, playlist: &config::WLedPlaylist) -> Result<(), Box<dyn Error>> {
    let json = json!({
        "playlist": {
            "ps": playlist.presets,
            "dur": playlist.durations,
            "transition": playlist.transitions,
            "repeat": playlist.repeat,
            "end": playlist.end,
            "r": 0,
        },
        "on": true,
        "o": true,
        "psave": playlist.id,
        "n": playlist.name,
        "v": true,
    });

    let addr = format!("http://{}/json/state", host);
    let json = json.to_string();

    println!("{:#?} {:#?}", addr, json);

    let client = reqwest::Client::new();
    let res = client.post(addr)
        .body(json)
        .send()
        .await?;

    let body = res.text().await?;
    println!("body: {:#?}", body);

    Ok(())

}