use reqwest;
use serde_json::value;
use std::error::Error;

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: u64,
    pub name: String,
}

pub async fn get_playlists(host: &str) -> Result<Vec<Playlist>, Box<dyn Error>> {
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

        let item = Playlist { id, name };

        pls.push(item);
    }

    Ok(pls)
}

pub async fn get_playlist(host: &str, name: String) -> Result<Option<Playlist>, Box<dyn Error>> {
    let lists = get_playlists(host).await?;

    for list in lists {
        if list.name == name {
            return Ok(Some(list))
        }
    }

    Ok(None)
}

pub async fn run_playlist(host: String, playlist: Playlist) -> Result<(), Box<dyn Error>> {
// curl -vvvv -H "Content-type: application/json" -X POST -d '{"ps":8}' http://192.168.2.84/json/state
    let addr = format!("http://{}/json/state", host);
    let json = format!(r#"{{"ps":{}}}"#, playlist.id);

    let client = reqwest::Client::new();
    let res = client.post(addr)
        .body(json)
        .send()
        .await?;

    Ok(())
}