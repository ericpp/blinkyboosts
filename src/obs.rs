use anyhow::{Context, Result};

use crate::config;

/// Switch the current OBS program scene.
///
/// Opens a new obs-websocket connection for each trigger, the same way WLED
/// loads on each trigger. In Studio Mode this changes the Program scene.
pub async fn trigger_toggle(cfg: &config::Obs, scene: &str) -> Result<()> {
    let port = cfg.port.unwrap_or(4455);
    let mut client = obws::Client::connect(&cfg.host, port, cfg.password.as_deref())
        .await
        .with_context(|| format!("Failed to connect to OBS at {}:{}", cfg.host, port))?;

    let result = client
        .scenes()
        .set_current_program_scene(scene)
        .await
        .with_context(|| format!("Failed to switch OBS scene to '{}'", scene));

    client.disconnect().await;
    result
}
