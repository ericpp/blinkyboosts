use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, Result};
use rosc::OscType;

use crate::config::{Command, Config, OscArgValue, Output};

/// Expand `names` and send each command to its output.
///
/// A disabled output is skipped with a log line. A failure on one command is
/// logged and does not stop the rest. Returns an error only when nothing was sent.
pub async fn run(config: &Config, names: &[String]) -> Result<()> {
    let resolved = config.resolve(names);
    if resolved.is_empty() {
        return Err(anyhow!("No actions to run"));
    }

    let mut sent = 0;
    let mut last_err: Option<anyhow::Error> = None;

    for (name, action) in resolved {
        let Some(output_cfg) = config.outputs.get(&action.output) else {
            eprintln!(
                "Action '{}' skipped: unknown output '{}'",
                name, action.output
            );
            last_err = Some(anyhow!("Unknown output '{}'", action.output));
            hold(name, action.hold_ms).await;
            continue;
        };

        if !output_cfg.enabled {
            println!(
                "Skipping action '{}' on disabled output '{}'",
                name, action.output
            );
            hold(name, action.hold_ms).await;
            continue;
        }

        println!("Running action '{}' on {}", name, action.output);
        match dispatch(&output_cfg.kind, &action.command).await {
            Ok(()) => sent += 1,
            Err(e) => {
                eprintln!(
                    "Failed to run action '{}' on {}: {:#}",
                    name, action.output, e
                );
                last_err = Some(e);
            }
        }
        hold(name, action.hold_ms).await;
    }

    if sent == 0 {
        return Err(last_err.unwrap_or_else(|| anyhow!("No actions were sent")));
    }

    Ok(())
}

/// Names shown in the recent-effects list, e.g. "cue3, rainbow".
pub fn describe(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

async fn dispatch(kind: &Output, command: &Command) -> Result<()> {
    match (kind, command) {
        (
            Output::Osc { address },
            Command::Osc {
                path,
                arg_value,
                arg_value2,
            },
        ) => send_osc(address, path, arg_value, arg_value2),
        (Output::Osc { address }, Command::RandomOsc { commands }) => {
            if commands.is_empty() {
                return Err(anyhow!("OSC command list is empty"));
            }
            let command = &commands[random_index(commands.len())];
            println!(
                "Picked OSC command {} {:?}",
                command.path, command.arg_value
            );
            send_osc(
                address,
                &command.path,
                &command.arg_value,
                &command.arg_value2,
            )
        }
        (
            Output::Artnet(cfg),
            Command::Dmx {
                channel,
                value,
                universe,
            },
        ) => crate::artnet::ArtNet::trigger_toggle(cfg, *channel, *value, *universe),
        (
            Output::Sacn(cfg),
            Command::Dmx {
                channel,
                value,
                universe,
            },
        ) => crate::sacn::Sacn::trigger_toggle(cfg, *channel, *value, *universe),
        (Output::Wled(cfg), Command::Wled { preset }) => {
            crate::wled::WLed::trigger_toggle(cfg, preset).await
        }
        (Output::Obs(cfg), Command::Obs { scene }) => crate::obs::trigger_toggle(cfg, scene).await,
        (
            Output::Obs(cfg),
            Command::ObsSource {
                scene,
                source,
                visible,
            },
        ) => crate::obs::trigger_visible(cfg, scene.as_deref(), source, *visible).await,
        (Output::Obs(cfg), Command::ObsMute { input, muted }) => {
            crate::obs::trigger_mute(cfg, input, *muted).await
        }
        _ => Err(anyhow!("Action command does not match output type")),
    }
}

async fn hold(name: &str, hold_ms: Option<u64>) {
    let Some(ms) = hold_ms.filter(|ms| *ms > 0) else {
        return;
    };
    println!("Holding action '{}' for {}ms", name, ms);
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

fn send_osc(
    address: &str,
    path: &str,
    arg_value: &OscArgValue,
    arg_value2: &Option<OscArgValue>,
) -> Result<()> {
    let mut args = vec![osc_arg(arg_value)];
    if let Some(second) = arg_value2 {
        args.push(osc_arg(second));
    }
    crate::osc::Osc::new(address)?.trigger_path(path, args)
}

fn random_index(len: usize) -> usize {
    static STATE: AtomicU64 = AtomicU64::new(0xA5A5_A5A5_1234_5678);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut x = STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) ^ now;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (x as usize) % len
}

fn osc_arg(value: &OscArgValue) -> OscType {
    match value {
        crate::config::OscArgValue::String(s) => OscType::String(s.clone()),
        crate::config::OscArgValue::Int(i) => OscType::Int(*i as i32),
        crate::config::OscArgValue::Float(f) => OscType::Float(*f as f32),
    }
}
