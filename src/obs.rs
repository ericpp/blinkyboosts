use anyhow::{bail, Context, Result};
use obws::requests::scene_items::{Id, SetEnabled};
use obws::requests::scenes::SceneId;
use obws::responses::StatusCode;

use crate::config;

struct Located {
    /// Scene or group that contains the source. Visibility is set on this container.
    container: String,
    item_id: i64,
}

/// Switch the current OBS program scene.
///
/// Opens a new obs-websocket connection for each trigger, the same way WLED
/// loads on each trigger. In Studio Mode this changes the Program scene.
pub async fn trigger_toggle(cfg: &config::Obs, scene: &str) -> Result<()> {
    let mut client = connect(cfg).await?;

    let result = client
        .scenes()
        .set_current_program_scene(scene)
        .await
        .with_context(|| format!("Failed to switch OBS scene to '{}'", scene));

    client.disconnect().await;
    result
}

/// Mute, unmute, or toggle mute on an OBS audio input by name.
///
/// `muted` of `Some(true)` mutes, `Some(false)` unmutes, and `None` toggles.
pub async fn trigger_mute(cfg: &config::Obs, input: &str, muted: Option<bool>) -> Result<()> {
    let mut client = connect(cfg).await?;
    let id = obws::requests::inputs::InputId::Name(input);

    let result = match muted {
        Some(muted) => client.inputs().set_muted(id, muted).await.with_context(|| {
            format!(
                "Failed to {} OBS input '{}'",
                if muted { "mute" } else { "unmute" },
                input
            )
        }),
        None => client
            .inputs()
            .toggle_mute(id)
            .await
            .map(|_| ())
            .with_context(|| format!("Failed to toggle mute on OBS input '{}'", input)),
    };

    client.disconnect().await;
    result
}

/// Show, hide, or toggle a source in an OBS scene.
///
/// `visible` of `Some(true)` shows, `Some(false)` hides, and `None` toggles.
/// `scene` of `None` uses the current program scene. A source inside a group
/// is found by searching the groups in that scene.
pub async fn trigger_visible(
    cfg: &config::Obs,
    scene: Option<&str>,
    source: &str,
    visible: Option<bool>,
) -> Result<()> {
    let mut client = connect(cfg).await?;
    let result = set_visible(&client, scene, source, visible).await;
    client.disconnect().await;
    result
}

async fn set_visible(
    client: &obws::Client,
    scene: Option<&str>,
    source: &str,
    visible: Option<bool>,
) -> Result<()> {
    let scene_owned;
    let scene = match scene {
        Some(scene) => scene,
        None => {
            scene_owned = client
                .scenes()
                .current_program_scene()
                .await
                .context("Failed to read the current OBS program scene")?
                .id
                .name;
            scene_owned.as_str()
        }
    };

    let located = locate_source(client, scene, source).await?;
    let scene_id = SceneId::Name(&located.container);
    let enabled = match visible {
        Some(visible) => visible,
        None => {
            let showing = client
                .scene_items()
                .enabled(scene_id, located.item_id)
                .await
                .with_context(|| {
                    format!(
                        "Failed to read visibility of OBS source '{}' in '{}'",
                        source, located.container
                    )
                })?;
            !showing
        }
    };

    client
        .scene_items()
        .set_enabled(SetEnabled {
            scene: scene_id,
            item_id: located.item_id,
            enabled,
        })
        .await
        .with_context(|| {
            format!(
                "Failed to {} OBS source '{}' in '{}'",
                if enabled { "show" } else { "hide" },
                source,
                located.container
            )
        })
}

async fn locate_source(client: &obws::Client, scene: &str, source: &str) -> Result<Located> {
    if let Some(item_id) = find_in(client, scene, source).await? {
        return Ok(Located {
            container: scene.to_string(),
            item_id,
        });
    }

    let items = client
        .scene_items()
        .list(SceneId::Name(scene))
        .await
        .with_context(|| format!("Failed to list sources in OBS scene '{}'", scene))?;

    for item in items {
        if item.is_group != Some(true) {
            continue;
        }
        if let Some(item_id) = find_in(client, &item.source_name, source).await? {
            return Ok(Located {
                container: item.source_name,
                item_id,
            });
        }
    }

    bail!("OBS source '{source}' was not found in scene '{scene}'")
}

/// `Ok(None)` when `source` is not in that scene or group.
async fn find_in(client: &obws::Client, container: &str, source: &str) -> Result<Option<i64>> {
    match client
        .scene_items()
        .id(Id {
            scene: SceneId::Name(container),
            source,
            search_offset: None,
        })
        .await
    {
        Ok(id) => Ok(Some(id)),
        Err(obws::error::Error::Api {
            code: StatusCode::ResourceNotFound,
            ..
        }) => Ok(None),
        Err(err) => Err(err)
            .with_context(|| format!("Failed to find OBS source '{source}' in '{container}'")),
    }
}

async fn connect(cfg: &config::Obs) -> Result<obws::Client> {
    let port = cfg.port.unwrap_or(4455);
    obws::Client::connect(&cfg.host, port, cfg.password.as_deref())
        .await
        .with_context(|| format!("Failed to connect to OBS at {}:{}", cfg.host, port))
}
