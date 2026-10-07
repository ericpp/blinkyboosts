use std::sync::OnceLock;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::queue::{self, Job};

/// The single pending revert. A newer look replaces whatever is waiting.
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

/// Cancel any waiting revert, then queue `names` after `secs` seconds.
/// The timer starts when this is called, which is when the triggering sequence finishes.
pub async fn schedule(config: Config, secs: u64, names: Vec<String>) {
    if names.is_empty() { return };

    let mut guard = slot().lock().await;
    abort_locked(&mut guard);

    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;

        println!("Firing revert after {}s", secs);
        queue::enqueue(Job {
            config,
            names,
            revert: None,
            replaces_look: false,
            skip_duplicate: false,
            done: None,
        });
    });

    *guard = Some(handle);
}
