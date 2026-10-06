use std::sync::OnceLock;
use tokio::task::JoinHandle;

use crate::config::{Config, Toggle};

/// The single pending revert. A newer threshold toggle replaces whatever is waiting.
static PENDING: OnceLock<tokio::sync::Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

fn slot() -> &'static tokio::sync::Mutex<Option<JoinHandle<()>>> {
    PENDING.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn abort_locked(guard: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = guard.take() {
        handle.abort();
        println!("Cancelled pending revert");
    }
}

/// Abort the revert that is still waiting, if any.
pub async fn cancel_pending() {
    let mut guard = slot().lock().await;
    abort_locked(&mut guard);
}

/// Cancel any waiting revert, then run `toggle.revert` after `revert_after` seconds.
pub async fn schedule(config: Config, toggle: Toggle) {
    let Some(secs) = toggle.revert_after else { return };
    if toggle.revert.is_empty() { return };
    let names = toggle.revert;

    let mut guard = slot().lock().await;
    abort_locked(&mut guard);

    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;

        println!("Firing revert after {}s", secs);
        if let Err(e) = crate::actions::run(&config, &names).await {
            eprintln!("Failed to run revert: {:#}", e);
        }
    });

    *guard = Some(handle);
}
