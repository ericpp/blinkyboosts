use anyhow::{Result, anyhow};
use rosc::OscType;

use crate::config::{Command, Config, Target};

/// Expand `names` and send each command to its target.
///
/// A disabled target is skipped with a log line. A failure on one command is
/// logged and does not stop the rest. Returns an error only when nothing was sent.
pub async fn run(config: &Config, names: &[String]) -> Result<()> {
    let resolved = config.resolve(names);
    if resolved.is_empty() {
        return Err(anyhow!("No actions to run"));
    }

    let mut sent = 0;
    let mut last_err: Option<anyhow::Error> = None;

    for (name, action) in resolved {
        let Some(target_cfg) = config.targets.get(&action.target) else {
            eprintln!("Action '{}' skipped: unknown target '{}'", name, action.target);
            last_err = Some(anyhow!("Unknown target '{}'", action.target));
            continue;
        };

        if !target_cfg.enabled {
            println!("Skipping action '{}' on disabled target '{}'", name, action.target);
            continue;
        }

        println!("Running action '{}' on {}", name, action.target);
        match dispatch(&target_cfg.kind, &action.command).await {
            Ok(()) => sent += 1,
            Err(e) => {
                eprintln!("Failed to run action '{}' on {}: {:#}", name, action.target, e);
                last_err = Some(e);
            }
        }
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

async fn dispatch(kind: &Target, command: &Command) -> Result<()> {
    match (kind, command) {
        (Target::Osc { address }, Command::Osc { path, arg_value, arg_value2 }) => {
            let mut args = vec![osc_arg(arg_value)];
            if let Some(second) = arg_value2 {
                args.push(osc_arg(second));
            }
            crate::osc::Osc::new(address)?.trigger_path(path, args)
        }
        (Target::Artnet(cfg), Command::Dmx { channel, value, universe }) => {
            crate::artnet::ArtNet::trigger_toggle(cfg, *channel, *value, *universe)
        }
        (Target::Sacn(cfg), Command::Dmx { channel, value, universe }) => {
            crate::sacn::Sacn::trigger_toggle(cfg, *channel, *value, *universe)
        }
        (Target::Wled(cfg), Command::Wled { preset }) => {
            crate::wled::WLed::trigger_toggle(cfg, preset).await
        }
        (Target::Obs(cfg), Command::Obs { scene }) => {
            crate::obs::trigger_toggle(cfg, scene).await
        }
        _ => Err(anyhow!("Action command does not match target type")),
    }
}

fn osc_arg(value: &crate::config::OscArgValue) -> OscType {
    match value {
        crate::config::OscArgValue::String(s) => OscType::String(s.clone()),
        crate::config::OscArgValue::Int(i) => OscType::Int(*i as i32),
        crate::config::OscArgValue::Float(f) => OscType::Float(*f as f32),
    }
}
