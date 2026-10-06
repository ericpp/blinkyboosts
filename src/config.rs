use serde_derive::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use anyhow::{Context, Result, bail};
use toml;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub nwc: Option<NWC>,
    pub boostboard: Option<BoostBoard>,
    pub zaps: Option<Zaps>,
    /// Named output destinations, e.g. `[targets.lights]`.
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
    /// Named commands toggles refer to by name, e.g. `[actions.cue2]`.
    #[serde(default)]
    pub actions: BTreeMap<String, OneOrMany<Action>>,
    pub toggles: Option<Vec<Toggle>>,
}

/// Common filter fields for boost sources
#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq)]
pub struct BoostFiltersConfig {
    pub load_since: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub podcasts: Option<Vec<String>>,
    pub episode_guids: Option<Vec<String>>,
    pub event_guids: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BoostBoard {
    #[serde(default)]
    pub relay_addrs: Vec<String>,
    pub pubkey: String,
    #[serde(flatten)]
    pub filters: BoostFiltersConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct NWC {
    pub uri: String,
    #[serde(flatten)]
    pub filters: BoostFiltersConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Zaps {
    pub relay_addrs: Vec<String>,
    pub naddr: String,
    pub load_since: Option<String>,  // Load zaps since this timestamp (e.g., "2025-01-11 00:00:00")
}

/// A named destination plus whether the GUI has it switched on.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TargetConfig {
    /// Defaults to true. The GUI enable/disable switch writes this.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: Target,
}

/// What a target talks to. The `type` field selects the variant.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Target {
    Osc { address: String },
    Artnet(ArtNet),
    Sacn(Sacn),
    Wled(WLed),
}

impl Target {
    /// Short name used in config errors, e.g. "wled".
    pub fn type_name(&self) -> &'static str {
        match self {
            Target::Osc { .. } => "osc",
            Target::Artnet(_) => "artnet",
            Target::Sacn(_) => "sacn",
            Target::Wled(_) => "wled",
        }
    }

    /// Label shown in the GUI, e.g. "Art-Net".
    pub fn label(&self) -> &'static str {
        match self {
            Target::Osc { .. } => "OSC",
            Target::Artnet(_) => "Art-Net",
            Target::Sacn(_) => "sACN",
            Target::Wled(_) => "WLED",
        }
    }

    fn accepts(&self, command: &Command) -> bool {
        matches!(
            (self, command),
            (Target::Osc { .. }, Command::Osc { .. })
                | (Target::Artnet(_), Command::Dmx { .. })
                | (Target::Sacn(_), Command::Dmx { .. })
                | (Target::Wled(_), Command::Wled { .. })
        )
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ArtNet {
    pub broadcast_address: String,
    pub local_address: Option<String>,
    pub universe: Option<u16>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Sacn {
    pub broadcast_address: String,
    pub universe: Option<u16>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLed {
    pub host: String,
    pub boost_playlist: String,
    pub brightness: u64,
    pub segments: Option<Vec<WLedSegment>>,
    pub presets: Option<Vec<WLedPreset>>,
    pub playlists: Option<Vec<WLedPlaylist>>,
    pub setup: bool,
    pub force: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedSegment {
    pub name: String,
    pub start: u64,
    pub stop: u64,
    pub reverse: Option<bool>,
    pub grouping: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedPreset {
    pub name: String,
    pub speed: Option<u64>,
    pub intensity: Option<u64>,
    pub colors: Vec<Vec<u64>>,
    pub colors2: Option<Vec<Vec<u64>>>,
    pub colors3: Option<Vec<Vec<u64>>>,
    pub effects: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct WLedPlaylist {
    pub name: String,
    pub presets: Vec<String>,
    pub durations: Vec<u64>,
    pub transitions: Vec<u64>,
    pub repeat: u64,
    pub end: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum OscArgValue {
    Int(i64),
    Float(f64),
    String(String),
}

/// One command sent to one target. The command fields depend on the target's type.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Action {
    /// Name from `[targets]`.
    pub target: String,
    #[serde(flatten)]
    pub command: Command,
}

/// The payload of an action. Which variant is valid depends on the target type.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum Command {
    Osc {
        path: String,
        arg_value: OscArgValue,
        /// Second OSC argument. Director Mini uses this for the scene transition
        /// (`0` cut, `1` fade). Omitted commands send only `arg_value`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg_value2: Option<OscArgValue>,
    },
    Dmx { channel: u16, value: u8, universe: Option<u16> },
    Wled { preset: String },
}

impl Command {
    /// The fields this command was recognized by, used in config errors.
    fn shape(&self) -> &'static str {
        match self {
            Command::Osc { .. } => "path/arg_value",
            Command::Dmx { .. } => "channel/value",
            Command::Wled { .. } => "preset",
        }
    }
}

/// Accepts either a single table (`[actions.cue1]`) or an array of tables (`[[actions.big_hit]]`).
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn as_slice(&self) -> &[T] {
        match self {
            OneOrMany::One(item) => std::slice::from_ref(item),
            OneOrMany::Many(items) => items,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Toggle {
    /// Fires when the running sat total crosses this value.
    #[serde(default)]
    pub threshold: i64,
    /// Fires when a single boost is at least this many sats.
    #[serde(default)]
    pub amount: i64,
    /// Fires when the boost's digits end with this value. `1` matches 1, 11, and 101.
    #[serde(default)]
    pub ends_with: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "default_true")]
    pub trigger_multiple: bool,  // If true, trigger for every multiple of the threshold (e.g., 250k triggers at 250k, 500k, 750k, etc.)
    pub endswith_range: Option<(u8, u8)>,  // If set, only trigger when the last two digits of sats are within this range (inclusive), e.g., (0, 3) for 00-03, (10, 25) for 10-25
    /// Seconds to wait after this toggle fires before running `revert`.
    #[serde(default)]
    pub revert_after: Option<u64>,
    /// Names from `[actions]`, run in order when this toggle fires.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Names from `[actions]`, run `revert_after` seconds later.
    #[serde(default)]
    pub revert: Vec<String>,
}

impl Config {
    /// Expand action names into their commands, in the order given.
    /// A scene name contributes one entry per command, each carrying that name.
    pub fn resolve<'a>(&'a self, names: &'a [String]) -> Vec<(&'a str, &'a Action)> {
        let mut resolved = Vec::new();
        for name in names {
            if let Some((key, group)) = self.actions.get_key_value(name) {
                for action in group.as_slice() {
                    resolved.push((key.as_str(), action));
                }
            }
        }
        resolved
    }

    /// Check that every toggle names a real action, and every action names a
    /// real target whose type matches the command.
    pub fn validate(&self) -> Result<()> {
        for (name, group) in &self.actions {
            for action in group.as_slice() {
                let Some(target) = self.targets.get(&action.target) else {
                    bail!("action '{}': unknown target '{}'", name, action.target);
                };
                if !target.kind.accepts(&action.command) {
                    bail!(
                        "action '{}': target '{}' is {} but action has {}",
                        name,
                        action.target,
                        target.kind.type_name(),
                        action.command.shape()
                    );
                }
            }
        }

        for (index, toggle) in self.toggles.as_deref().unwrap_or(&[]).iter().enumerate() {
            if toggle.threshold > 0 && toggle.amount > 0 {
                bail!("toggle #{}: set threshold or amount, not both", index + 1);
            }
            for name in toggle.actions.iter().chain(toggle.revert.iter()) {
                if !self.actions.contains_key(name) {
                    bail!("toggle #{}: unknown action '{}'", index + 1, name);
                }
            }
        }

        Ok(())
    }
}

fn default_true() -> bool {
    true
}

pub fn load_config() -> Result<Config> {
    let filename = "./config.toml";

    let contents = fs::read_to_string(filename)
        .context(format!("Failed to read config file: {}", filename))?;
    parse_config(&contents)
}

/// Parse and validate config text. A reload uses this through `load_config`.
pub fn parse_config(contents: &str) -> Result<Config> {
    let cfg: Config = toml::from_str(contents)
        .context("Failed to parse config file as TOML")?;
    cfg.validate()?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> Config {
        toml::from_str(toml).unwrap()
    }

    const MIXED: &str = r#"
        [targets.lights]
        type = "osc"
        address = "10.0.0.184:8000"

        [targets.dmx]
        type = "artnet"
        broadcast_address = "10.0.0.255"
        universe = 2

        [targets.pixels]
        type = "sacn"
        broadcast_address = ""
        universe = 3

        [targets.strip]
        type = "wled"
        host = "10.0.0.20"
        boost_playlist = "BOOST"
        brightness = 128
        setup = false
        force = false

        [actions.cue1]
        target = "lights"
        path = "/gma3/cmd"
        arg_value = "Goto Sequence 54 Cue 1"

        [[actions.big_hit]]
        target = "dmx"
        channel = 1
        value = 255
        [[actions.big_hit]]
        target = "strip"
        preset = "BOOST-Fire"
        [[actions.big_hit]]
        target = "pixels"
        channel = 10
        value = 128
        universe = 4

        [[toggles]]
        threshold = 25000
        revert_after = 30
        actions = ["big_hit"]
        revert = ["cue1"]
    "#;

    #[test]
    fn resolves_single_and_scene_actions_across_target_types() {
        let cfg = parse(MIXED);
        cfg.validate().unwrap();
        assert!(cfg.targets["lights"].enabled);

        let toggle = &cfg.toggles.as_ref().unwrap()[0];
        assert_eq!(toggle.revert_after, Some(30));

        let scene = cfg.resolve(&toggle.actions);
        assert_eq!(scene.len(), 3);
        assert!(scene.iter().all(|(name, _)| *name == "big_hit"));
        assert!(matches!(scene[0].1.command, Command::Dmx { channel: 1, value: 255, universe: None }));
        assert!(matches!(&scene[1].1.command, Command::Wled { preset } if preset == "BOOST-Fire"));
        assert!(matches!(scene[2].1.command, Command::Dmx { channel: 10, value: 128, universe: Some(4) }));

        let revert = cfg.resolve(&toggle.revert);
        assert_eq!(revert.len(), 1);
        assert_eq!(revert[0].0, "cue1");
        assert!(matches!(&revert[0].1.command, Command::Osc { path, arg_value2: None, .. } if path == "/gma3/cmd"));
    }

    #[test]
    fn osc_action_accepts_a_second_argument() {
        let cfg = parse(r#"
            [targets.camera]
            type = "osc"
            address = "10.0.0.244:7000"

            [actions.scene2]
            target = "camera"
            path = "/scene/switchByIndex"
            arg_value = 2
            arg_value2 = 1
        "#);
        cfg.validate().unwrap();
        let action = &cfg.actions["scene2"].as_slice()[0];
        assert!(matches!(
            &action.command,
            Command::Osc { path, arg_value: OscArgValue::Int(2), arg_value2: Some(OscArgValue::Int(1)) }
                if path == "/scene/switchByIndex"
        ));
    }

    #[test]
    fn rejects_threshold_and_amount_on_the_same_toggle() {
        let cfg = parse(r#"
            [[toggles]]
            threshold = 25000
            amount = 1000
        "#);
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "toggle #1: set threshold or amount, not both");
    }

    #[test]
    fn rejects_unknown_action_name() {
        let cfg = parse(r#"
            [[toggles]]
            threshold = 1
            [[toggles]]
            threshold = 2
            [[toggles]]
            actions = ["rainbw"]
        "#);
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "toggle #3: unknown action 'rainbw'");
    }

    #[test]
    fn rejects_command_that_does_not_match_target_type() {
        let cfg = parse(r#"
            [targets.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.rainbow]
            target = "strip"
            path = "/gma3/cmd"
            arg_value = "nope"
        "#);
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "action 'rainbow': target 'strip' is wled but action has path/arg_value");
    }

    #[test]
    fn rejects_unknown_target() {
        let cfg = parse(r#"
            [actions.rainbow]
            target = "strip"
            preset = "BOOST-Rainbow"
        "#);
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "action 'rainbow': unknown target 'strip'");
    }

    #[test]
    fn project_config_still_parses() {
        let cfg = load_config().unwrap();
        assert!(cfg.targets.contains_key("lights"));
        assert!(cfg.actions.contains_key("cue2"));
        assert!(cfg.actions.contains_key("boost"));
    }

    #[test]
    fn example_configs_parse() {
        for filename in ["./config.toml.example", "./config_sat_total_example.toml"] {
            let contents = fs::read_to_string(filename).unwrap();
            parse_config(&contents).unwrap();
        }
    }

    #[test]
    fn saved_config_reloads() {
        let cfg = load_config().unwrap();
        let text = toml::to_string(&cfg).unwrap();
        let again = parse_config(&text).unwrap();
        assert_eq!(cfg.nwc, again.nwc);
        assert_eq!(cfg.boostboard, again.boostboard);
        assert_eq!(cfg.zaps, again.zaps);
        assert_eq!(cfg.targets.len(), again.targets.len());
        assert_eq!(cfg.actions.len(), again.actions.len());
    }

    #[test]
    fn invalid_config_text_is_rejected() {
        let err = parse_config("this is not [valid").unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("Failed to parse config file as TOML"), "{msg}");
    }

    #[test]
    fn invalid_config_keeps_the_previous_one() {
        let previous = parse_config(&fs::read_to_string("./config.toml").unwrap()).unwrap();
        assert!(parse_config("targets = 1").is_err());
        assert!(previous.targets.contains_key("lights"));
    }
}
