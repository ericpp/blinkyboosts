use anyhow::{Context, Result};

use crate::config;

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

async fn connect(cfg: &config::Obs) -> Result<obws::Client> {
    let port = cfg.port.unwrap_or(4455);
    obws::Client::connect(&cfg.host, port, cfg.password.as_deref())
        .await
        .with_context(|| format!("Failed to connect to OBS at {}:{}", cfg.host, port))
}
