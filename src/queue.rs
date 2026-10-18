use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

use crate::config::Config;

/// One run of named actions, played in the order jobs are enqueued.
pub struct Job {
    pub config: Config,
    pub names: Vec<String>,
    /// `(seconds, action names)` scheduled after this job succeeds.
    pub revert: Option<(u64, Vec<String>)>,
    /// Cancel a revert that is still waiting on its timer before this job runs.
    pub replaces_look: bool,
    /// Drop this job when the same action names are already running or waiting.
    pub skip_duplicate: bool,
    pub done: Option<oneshot::Sender<Result<()>>>,
}

static SENDER: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();
/// Action-name lists of jobs that are waiting or running, in queue order.
static PENDING: OnceLock<Mutex<Vec<Vec<String>>>> = OnceLock::new();

fn pending() -> &'static Mutex<Vec<Vec<String>>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

fn sender() -> &'static mpsc::UnboundedSender<Job> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(worker(rx));
        tx
    })
}

fn release(names: &[String], last: bool) {
    let mut queued = pending().lock().expect("action queue lock");
    let index = if last {
        queued.iter().rposition(|queued_names| queued_names == names)
    } else {
        queued.iter().position(|queued_names| queued_names == names)
    };
    if let Some(index) = index {
        queued.remove(index);
    }
}

/// Add `job` to the queue. Returns `false` when a duplicate boost is skipped
/// or the worker has stopped.
pub fn enqueue(job: Job) -> bool {
    let label = crate::actions::describe(&job.names);
    let mut queued = pending().lock().expect("action queue lock");
    if job.skip_duplicate && queued.iter().any(|names| names == &job.names) {
        println!("Skipped '{}': already queued", label);
        return false;
    }
    let ahead = queued.len();
    let names = job.names.clone();
    queued.push(names.clone());
    drop(queued);

    if ahead > 0 {
        println!("Queued '{}' ({} ahead)", label, ahead);
    }
    if sender().send(job).is_err() {
        release(&names, true);
        false
    } else {
        true
    }
}

async fn worker(mut rx: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = rx.recv().await {
        let label = crate::actions::describe(&job.names);
        if job.replaces_look {
            crate::revert::cancel_pending().await;
        }

        let result = crate::actions::run(&job.config, &job.names).await;
        release(&job.names, false);

        if let Err(e) = &result {
            eprintln!("Failed to run '{}': {:#}", label, e);
        } else if let Some((secs, names)) = job.revert {
            crate::revert::schedule(job.config.clone(), secs, names).await;
        }

        if let Some(done) = job.done {
            let _ = done.send(result);
        }
    }
}
