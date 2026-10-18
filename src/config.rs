use anyhow::{bail, Context, Result};
use serde_derive::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use toml;

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct Config {
    /// Legacy singleton sections. Moved into `inputs` on load and not written back.
    #[serde(default, skip_serializing)]
    pub nwc: Option<NWC>,
    #[serde(default, skip_serializing)]
    pub boostboard: Option<BoostBoard>,
    #[serde(default, skip_serializing)]
    pub zaps: Option<Zaps>,
    /// Legacy SplitKit section. Moved into `inputs` on load and not written back.
    #[serde(default, skip_serializing)]
    pub splitkit: Option<SplitKit>,
    /// Named boost sources, e.g. `[inputs.show_nwc]`. Several may share a type.
    #[serde(default)]
    pub inputs: BTreeMap<String, InputConfig>,
    /// Named output destinations, e.g. `[outputs.lights]`.
    #[serde(default)]
    pub outputs: BTreeMap<String, OutputConfig>,
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
    pub load_since: Option<String>, // Load zaps since this timestamp (e.g., "2025-01-11 00:00:00")
}

/// A SplitKit live socket. Fired effects arrive as broadcasts on `url`.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct SplitKit {
    pub url: String,
}

/// A named boost source plus whether the GUI has it switched on.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct InputConfig {
    /// Defaults to true. The GUI enable/disable switch writes this.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: Input,
}

/// What an input listens to. The `type` field selects the variant.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Input {
    Nwc(NWC),
    Boostboard(BoostBoard),
    Zaps(Zaps),
    Splitkit(SplitKit),
}

impl Input {
    /// Short name used in config errors, e.g. "nwc".
    pub fn type_name(&self) -> &'static str {
        match self {
            Input::Nwc(_) => "nwc",
            Input::Boostboard(_) => "boostboard",
            Input::Zaps(_) => "zaps",
            Input::Splitkit(_) => "splitkit",
        }
    }

    /// Label shown in the GUI, e.g. "NWC".
    pub fn label(&self) -> &'static str {
        match self {
            Input::Nwc(_) => "NWC",
            Input::Boostboard(_) => "Boostboard",
            Input::Zaps(_) => "Zaps",
            Input::Splitkit(_) => "SplitKit",
        }
    }
}

/// Effect ids the SplitKit live socket broadcasts, in catalog order.
// pub const SPLITKIT_EFFECTS: &[&str] = &["wide", "dj", "festoons", "bubbles", "mystery"];

/// A named destination plus whether the GUI has it switched on.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OutputConfig {
    /// Defaults to true. The GUI enable/disable switch writes this.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: Output,
}

/// What an output talks to. The `type` field selects the variant.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Output {
    Osc { address: String },
    Artnet(ArtNet),
    Sacn(Sacn),
    Wled(WLed),
    Obs(Obs),
}

impl Output {
    /// Short name used in config errors, e.g. "wled".
    pub fn type_name(&self) -> &'static str {
        match self {
            Output::Osc { .. } => "osc",
            Output::Artnet(_) => "artnet",
            Output::Sacn(_) => "sacn",
            Output::Wled(_) => "wled",
            Output::Obs(_) => "obs",
        }
    }

    /// Label shown in the GUI, e.g. "Art-Net".
    pub fn label(&self) -> &'static str {
        match self {
            Output::Osc { .. } => "OSC",
            Output::Artnet(_) => "Art-Net",
            Output::Sacn(_) => "sACN",
            Output::Wled(_) => "WLED",
            Output::Obs(_) => "OBS",
        }
    }

    fn accepts(&self, command: &Command) -> bool {
        matches!(
            (self, command),
            (
                Output::Osc { .. },
                Command::Osc { .. } | Command::RandomOsc { .. }
            )
                | (Output::Artnet(_), Command::Dmx { .. })
                | (Output::Sacn(_), Command::Dmx { .. })
                | (Output::Wled(_), Command::Wled { .. })
                | (
                    Output::Obs(_),
                    Command::Obs { .. } | Command::ObsMute { .. } | Command::ObsSource { .. }
                )
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

/// An OBS instance reached over obs-websocket (v5).
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Obs {
    pub host: String,
    /// obs-websocket port. Defaults to 4455 when omitted.
    pub port: Option<u16>,
    /// WebSocket password. Omit when OBS authentication is disabled.
    pub password: Option<String>,
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

/// One step sent to one output. The command fields depend on the output's type.
/// An OSC step can list several commands in `commands` and send one of them.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Action {
    /// Name from `[outputs]`.
    pub output: String,
    /// Milliseconds to wait after this step before the next one runs.
    /// A hold on the last step finishes before the next queued action starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_ms: Option<u64>,
    #[serde(flatten)]
    pub command: Command,
}

/// The payload of an action. Which variant is valid depends on the output type.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum Command {
    /// Send one of `commands`, chosen at random. Only valid on an OSC output.
    /// Matched before [`Command::Osc`] so a `commands` list is not ignored.
    RandomOsc { commands: Vec<OscCommand> },
    Osc {
        path: String,
        arg_value: OscArgValue,
        /// Second OSC argument. Director Mini uses this for the scene transition
        /// (`0` cut, `1` fade). Omitted commands send only `arg_value`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg_value2: Option<OscArgValue>,
    },
    Dmx {
        channel: u16,
        value: u8,
        universe: Option<u16>,
    },
    Wled {
        preset: String,
    },
    /// Show, hide, or toggle a source in a scene.
    ///
    /// Must be matched before [`Command::Obs`]: a scene switch only has `scene`,
    /// and serde would otherwise ignore `source` and `visible`.
    /// `visible` omitted means toggle. `scene` omitted means the current program scene.
    ObsSource {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scene: Option<String>,
        /// `Some(true)` shows, `Some(false)` hides, `None` toggles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible: Option<bool>,
    },
    Obs {
        scene: String,
    },
    /// Mute, unmute, or toggle an OBS audio input by name.
    /// `muted` omitted means toggle.
    ObsMute {
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
    },
}

/// One OSC message. Used on its own, or as one entry in a [`Command::RandomOsc`] list.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct OscCommand {
    pub path: String,
    pub arg_value: OscArgValue,
    /// Second OSC argument. Omitted commands send only `arg_value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg_value2: Option<OscArgValue>,
}

impl Command {
    /// The fields this command was recognized by, used in config errors.
    fn shape(&self) -> &'static str {
        match self {
            Command::RandomOsc { .. } => "commands",
            Command::Osc { .. } => "path/arg_value",
            Command::Dmx { .. } => "channel/value",
            Command::Wled { .. } => "preset",
            Command::ObsSource { .. } => "source/visible",
            Command::Obs { .. } => "scene",
            Command::ObsMute { .. } => "input/muted",
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
    /// Fires when SplitKit broadcasts an effect with this id, e.g. "dj".
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "default_true")]
    pub trigger_multiple: bool, // If true, trigger for every multiple of the threshold (e.g., 250k triggers at 250k, 500k, 750k, etc.)
    pub endswith_range: Option<(u8, u8)>, // If set, only trigger when the last two digits of sats are within this range (inclusive), e.g., (0, 3) for 00-03, (10, 25) for 10-25
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
    /// Move legacy `[nwc]`, `[boostboard]`, `[zaps]`, and `[splitkit]` sections
    /// into `inputs` under those names. A name already used by `[inputs]` is an error.
    fn migrate_legacy_inputs(&mut self) -> Result<()> {
        let legacy = [
            ("nwc", self.nwc.take().map(Input::Nwc)),
            ("boostboard", self.boostboard.take().map(Input::Boostboard)),
            ("zaps", self.zaps.take().map(Input::Zaps)),
            ("splitkit", self.splitkit.take().map(Input::Splitkit)),
        ];
        for (name, kind) in legacy {
            let Some(kind) = kind else { continue };
            if self.inputs.contains_key(name) {
                bail!(
                    "input '{}' is defined both as [{}] and [inputs.{}]",
                    name,
                    name,
                    name
                );
            }
            self.inputs.insert(
                name.to_string(),
                InputConfig {
                    enabled: true,
                    kind,
                },
            );
        }
        Ok(())
    }

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
    /// real output whose type matches the command.
    pub fn validate(&self) -> Result<()> {
        for name in self.inputs.keys() {
            if name == "Effects" {
                bail!("input name 'Effects' is reserved");
            }
            if self.outputs.contains_key(name) {
                bail!("input '{}' uses the same name as an output", name);
            }
        }

        for (name, group) in &self.actions {
            for action in group.as_slice() {
                let Some(output) = self.outputs.get(&action.output) else {
                    bail!("action '{}': unknown output '{}'", name, action.output);
                };
                if !output.kind.accepts(&action.command) {
                    bail!(
                        "action '{}': output '{}' is {} but action has {}",
                        name,
                        action.output,
                        output.kind.type_name(),
                        action.command.shape()
                    );
                }
                if let Command::RandomOsc { commands } = &action.command {
                    if commands.is_empty() {
                        bail!("action '{}': commands list is empty", name);
                    }
                }
            }
        }

        for (index, toggle) in self.toggles.as_deref().unwrap_or(&[]).iter().enumerate() {
            if toggle.threshold > 0 && toggle.amount > 0 {
                bail!("toggle #{}: set threshold or amount, not both", index + 1);
            }
            if let Some(effect) = &toggle.effect {
                if toggle.threshold > 0
                    || toggle.amount > 0
                    || toggle.ends_with.is_some()
                    || toggle.endswith_range.is_some()
                    || toggle.is_default
                {
                    bail!(
                        "toggle #{}: effect toggles cannot also set threshold/amount/ends_with/is_default",
                        index + 1
                    );
                }
                // if !SPLITKIT_EFFECTS.contains(&effect.as_str()) {
                //     bail!(
                //         "toggle #{}: unknown effect '{}' (expected one of {})",
                //         index + 1,
                //         effect,
                //         SPLITKIT_EFFECTS.join(", ")
                //     );
                // }
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
    let mut cfg: Config =
        toml::from_str(contents).context("Failed to parse config file as TOML")?;
    cfg.migrate_legacy_inputs()?;
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
        [outputs.lights]
        type = "osc"
        address = "10.0.0.184:8000"

        [outputs.dmx]
        type = "artnet"
        broadcast_address = "10.0.0.255"
        universe = 2

        [outputs.pixels]
        type = "sacn"
        broadcast_address = ""
        universe = 3

        [outputs.strip]
        type = "wled"
        host = "10.0.0.20"
        boost_playlist = "BOOST"
        brightness = 128
        setup = false
        force = false

        [actions.cue1]
        output = "lights"
        path = "/gma3/cmd"
        arg_value = "Goto Sequence 54 Cue 1"

        [[actions.big_hit]]
        output = "dmx"
        channel = 1
        value = 255
        hold_ms = 1500
        [[actions.big_hit]]
        output = "strip"
        preset = "BOOST-Fire"
        [[actions.big_hit]]
        output = "pixels"
        channel = 10
        value = 128
        universe = 4
        hold_ms = 4000

        [[toggles]]
        threshold = 25000
        revert_after = 30
        actions = ["big_hit"]
        revert = ["cue1"]
    "#;

    #[test]
    fn resolves_single_and_scene_actions_across_output_types() {
        let cfg = parse(MIXED);
        cfg.validate().unwrap();
        assert!(cfg.outputs["lights"].enabled);

        let toggle = &cfg.toggles.as_ref().unwrap()[0];
        assert_eq!(toggle.revert_after, Some(30));

        let scene = cfg.resolve(&toggle.actions);
        assert_eq!(scene.len(), 3);
        assert!(scene.iter().all(|(name, _)| *name == "big_hit"));
        assert!(matches!(
            scene[0].1.command,
            Command::Dmx {
                channel: 1,
                value: 255,
                universe: None
            }
        ));
        assert_eq!(scene[0].1.hold_ms, Some(1500));
        assert!(matches!(&scene[1].1.command, Command::Wled { preset } if preset == "BOOST-Fire"));
        assert_eq!(scene[1].1.hold_ms, None);
        assert!(matches!(
            scene[2].1.command,
            Command::Dmx {
                channel: 10,
                value: 128,
                universe: Some(4)
            }
        ));
        assert_eq!(scene[2].1.hold_ms, Some(4000));

        let revert = cfg.resolve(&toggle.revert);
        assert_eq!(revert.len(), 1);
        assert_eq!(revert[0].0, "cue1");
        assert!(
            matches!(&revert[0].1.command, Command::Osc { path, arg_value2: None, .. } if path == "/gma3/cmd")
        );
    }

    #[test]
    fn osc_action_accepts_a_second_argument() {
        let cfg = parse(
            r#"
            [outputs.camera]
            type = "osc"
            address = "10.0.0.244:7000"

            [actions.scene2]
            output = "camera"
            path = "/scene/switchByIndex"
            arg_value = 2
            arg_value2 = 1
        "#,
        );
        cfg.validate().unwrap();
        let action = &cfg.actions["scene2"].as_slice()[0];
        assert!(matches!(
            &action.command,
            Command::Osc { path, arg_value: OscArgValue::Int(2), arg_value2: Some(OscArgValue::Int(1)) }
                if path == "/scene/switchByIndex"
        ));
    }

    #[test]
    fn obs_output_and_action_parse() {
        let cfg = parse(
            r#"
            [outputs.obs]
            type = "obs"
            host = "127.0.0.1"
            port = 4455
            password = "secret"

            [actions.obs_cam2]
            output = "obs"
            scene = "Camera 2"
        "#,
        );
        cfg.validate().unwrap();
        assert!(matches!(
            &cfg.outputs["obs"].kind,
            Output::Obs(obs)
                if obs.host == "127.0.0.1"
                    && obs.port == Some(4455)
                    && obs.password.as_deref() == Some("secret")
        ));
        let action = &cfg.actions["obs_cam2"].as_slice()[0];
        assert!(matches!(&action.command, Command::Obs { scene } if scene == "Camera 2"));
    }

    #[test]
    fn obs_port_and_password_are_optional() {
        let cfg = parse(
            r#"
            [outputs.obs]
            type = "obs"
            host = "127.0.0.1"

            [actions.obs_cam2]
            output = "obs"
            scene = "Camera 2"
        "#,
        );
        cfg.validate().unwrap();
        assert!(matches!(
            &cfg.outputs["obs"].kind,
            Output::Obs(obs) if obs.port.is_none() && obs.password.is_none()
        ));
    }

    #[test]
    fn obs_mute_action_parses_set_and_toggle() {
        let cfg = parse(
            r#"
            [outputs.obs]
            type = "obs"
            host = "127.0.0.1"

            [actions.mic_off]
            output = "obs"
            input = "Mic/Aux"
            muted = true

            [actions.mic_toggle]
            output = "obs"
            input = "Desktop Audio"
        "#,
        );
        cfg.validate().unwrap();
        let off = &cfg.actions["mic_off"].as_slice()[0];
        assert!(matches!(
            &off.command,
            Command::ObsMute { input, muted: Some(true) } if input == "Mic/Aux"
        ));
        let toggle = &cfg.actions["mic_toggle"].as_slice()[0];
        assert!(matches!(
            &toggle.command,
            Command::ObsMute { input, muted: None } if input == "Desktop Audio"
        ));
    }

    #[test]
    fn obs_source_visibility_parses_show_hide_and_toggle() {
        let cfg = parse(
            r#"
            [outputs.obs]
            type = "obs"
            host = "127.0.0.1"

            [actions.show_logo]
            output = "obs"
            scene = "Scene 1"
            source = "Logo"
            visible = true

            [actions.hide_logo]
            output = "obs"
            source = "Logo"
            visible = false

            [actions.toggle_logo]
            output = "obs"
            scene = "Scene 1"
            source = "Logo"

            [actions.obs_cam2]
            output = "obs"
            scene = "Camera 2"
        "#,
        );
        cfg.validate().unwrap();
        let show = &cfg.actions["show_logo"].as_slice()[0];
        assert!(matches!(
            &show.command,
            Command::ObsSource { source, scene: Some(scene), visible: Some(true) }
                if source == "Logo" && scene == "Scene 1"
        ));
        let hide = &cfg.actions["hide_logo"].as_slice()[0];
        assert!(matches!(
            &hide.command,
            Command::ObsSource { source, scene: None, visible: Some(false) } if source == "Logo"
        ));
        let toggle = &cfg.actions["toggle_logo"].as_slice()[0];
        assert!(matches!(
            &toggle.command,
            Command::ObsSource { source, scene: Some(scene), visible: None }
                if source == "Logo" && scene == "Scene 1"
        ));
        let scene = &cfg.actions["obs_cam2"].as_slice()[0];
        assert!(matches!(&scene.command, Command::Obs { scene } if scene == "Camera 2"));
    }

    #[test]
    fn rejects_obs_source_on_wled_output() {
        let cfg = parse(
            r#"
            [outputs.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.logo]
            output = "strip"
            source = "Logo"
            visible = false
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "action 'logo': output 'strip' is wled but action has source/visible"
        );
    }

    #[test]
    fn rejects_obs_mute_on_wled_output() {
        let cfg = parse(
            r#"
            [outputs.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.mic]
            output = "strip"
            input = "Mic/Aux"
            muted = false
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "action 'mic': output 'strip' is wled but action has input/muted"
        );
    }

    #[test]
    fn rejects_obs_scene_on_wled_output() {
        let cfg = parse(
            r#"
            [outputs.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.cam]
            output = "strip"
            scene = "Camera 2"
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "action 'cam': output 'strip' is wled but action has scene"
        );
    }

    #[test]
    fn rejects_threshold_and_amount_on_the_same_toggle() {
        let cfg = parse(
            r#"
            [[toggles]]
            threshold = 25000
            amount = 1000
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "toggle #1: set threshold or amount, not both");
    }

    #[test]
    fn random_osc_action_picks_from_its_command_list() {
        let cfg = parse(
            r#"
            [outputs.camera]
            type = "osc"
            address = "10.0.0.244:7000"

            [actions.random_camera]
            output = "camera"
            commands = [
              { path = "/scene/switchByIndex", arg_value = 1 },
              { path = "/scene/switchByIndex", arg_value = 2, arg_value2 = 0 },
              { path = "/gma3/cmd", arg_value = "Goto Sequence 54 Cue 3" },
            ]

            [[toggles]]
            actions = ["random_camera"]
        "#,
        );
        cfg.validate().unwrap();
        let action = &cfg.actions["random_camera"].as_slice()[0];
        match &action.command {
            Command::RandomOsc { commands } => {
                assert_eq!(commands.len(), 3);
                assert_eq!(commands[0].path, "/scene/switchByIndex");
                assert!(matches!(commands[0].arg_value, OscArgValue::Int(1)));
                assert!(matches!(
                    commands[1].arg_value2,
                    Some(OscArgValue::Int(0))
                ));
                assert!(matches!(commands[2].arg_value, OscArgValue::String(ref s) if s == "Goto Sequence 54 Cue 3"));
            }
            other => panic!("expected a command list, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_osc_command_list() {
        let cfg = parse(
            r#"
            [outputs.camera]
            type = "osc"
            address = "10.0.0.244:7000"

            [actions.random_camera]
            output = "camera"
            commands = []
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "action 'random_camera': commands list is empty");
    }

    #[test]
    fn rejects_osc_command_list_on_wled_output() {
        let cfg = parse(
            r#"
            [outputs.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.random_camera]
            output = "strip"
            commands = [
              { path = "/scene/switchByIndex", arg_value = 1 },
            ]
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "action 'random_camera': output 'strip' is wled but action has commands"
        );
    }

    #[test]
    fn effect_toggle_parses() {
        let cfg = parse(
            r#"
            [splitkit]
            url = "wss://example.com/api/live/ws"

            [actions.dj_look]
            output = "lights"
            path = "/gma3/cmd"
            arg_value = "Goto Sequence 54 Cue 3"

            [outputs.lights]
            type = "osc"
            address = "10.0.0.184:8000"

            [[toggles]]
            effect = "dj"
            revert_after = 30
            actions = ["dj_look"]
        "#,
        );
        cfg.validate().unwrap();
        assert_eq!(cfg.splitkit.unwrap().url, "wss://example.com/api/live/ws");
        let toggle = &cfg.toggles.unwrap()[0];
        assert_eq!(toggle.effect.as_deref(), Some("dj"));
        assert_eq!(toggle.revert_after, Some(30));
    }

    #[test]
    fn rejects_effect_toggle_that_also_matches_sats() {
        let cfg = parse(
            r#"
            [[toggles]]
            threshold = 1
            [[toggles]]
            effect = "dj"
            amount = 1000
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "toggle #2: effect toggles cannot also set threshold/amount/ends_with/is_default"
        );
    }

    #[test]
    fn rejects_unknown_effect_id() {
        let cfg = parse(
            r#"
            [[toggles]]
            effect = "rainbow"
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "toggle #1: unknown effect 'rainbow' (expected one of wide, dj, festoons, bubbles, mystery)"
        );
    }

    #[test]
    fn rejects_unknown_action_name() {
        let cfg = parse(
            r#"
            [[toggles]]
            threshold = 1
            [[toggles]]
            threshold = 2
            [[toggles]]
            actions = ["rainbw"]
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "toggle #3: unknown action 'rainbw'");
    }

    #[test]
    fn rejects_command_that_does_not_match_output_type() {
        let cfg = parse(
            r#"
            [outputs.strip]
            type = "wled"
            host = "10.0.0.20"
            boost_playlist = "BOOST"
            brightness = 128
            setup = false
            force = false

            [actions.rainbow]
            output = "strip"
            path = "/gma3/cmd"
            arg_value = "nope"
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(
            err,
            "action 'rainbow': output 'strip' is wled but action has path/arg_value"
        );
    }

    #[test]
    fn rejects_unknown_output() {
        let cfg = parse(
            r#"
            [actions.rainbow]
            output = "strip"
            preset = "BOOST-Rainbow"
        "#,
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert_eq!(err, "action 'rainbow': unknown output 'strip'");
    }

    #[test]
    fn multiple_inputs_of_the_same_type_parse() {
        let cfg = parse_config(
            r#"
            [inputs.show_nwc]
            type = "nwc"
            uri = "nostr+walletconnect://show"

            [inputs.guest_nwc]
            type = "nwc"
            enabled = false
            uri = "nostr+walletconnect://guest"

            [inputs.board]
            type = "boostboard"
            relay_addrs = ["wss://relay.damus.io"]
            pubkey = "aa"

            [inputs.other_board]
            type = "boostboard"
            relay_addrs = ["wss://nos.lol"]
            pubkey = "bb"
        "#,
        )
        .unwrap();

        assert!(matches!(
            &cfg.inputs["show_nwc"].kind,
            Input::Nwc(nwc) if nwc.uri == "nostr+walletconnect://show"
        ));
        assert!(!cfg.inputs["guest_nwc"].enabled);
        assert!(
            matches!(&cfg.inputs["board"].kind, Input::Boostboard(board) if board.pubkey == "aa")
        );
        assert!(
            matches!(&cfg.inputs["other_board"].kind, Input::Boostboard(board) if board.pubkey == "bb")
        );
        assert!(
            cfg.inputs
                .values()
                .filter(|input| input.kind.type_name() == "nwc")
                .count()
                == 2
        );
    }

    #[test]
    fn legacy_singleton_sections_migrate_into_inputs() {
        let cfg = parse_config(
            r#"
            [nwc]
            uri = "nostr+walletconnect://show"

            [splitkit]
            url = "wss://example.com/api/live/ws"
        "#,
        )
        .unwrap();

        assert!(cfg.nwc.is_none());
        assert!(cfg.splitkit.is_none());
        assert!(cfg.inputs["nwc"].enabled);
        assert!(matches!(
            &cfg.inputs["nwc"].kind,
            Input::Nwc(nwc) if nwc.uri == "nostr+walletconnect://show"
        ));
        assert!(matches!(
            &cfg.inputs["splitkit"].kind,
            Input::Splitkit(splitkit) if splitkit.url == "wss://example.com/api/live/ws"
        ));
    }

    #[test]
    fn rejects_legacy_section_that_duplicates_a_named_input() {
        let err = parse_config(
            r#"
            [nwc]
            uri = "nostr+walletconnect://old"

            [inputs.nwc]
            type = "nwc"
            uri = "nostr+walletconnect://new"
        "#,
        )
        .unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("input 'nwc' is defined both as [nwc] and [inputs.nwc]"),
            "{msg}"
        );
    }

    #[test]
    fn rejects_input_name_that_matches_an_output() {
        let err = parse_config(
            r#"
            [inputs.lights]
            type = "nwc"
            uri = "nostr+walletconnect://show"

            [outputs.lights]
            type = "osc"
            address = "10.0.0.184:8000"
        "#,
        )
        .unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("input 'lights' uses the same name as an output"),
            "{msg}"
        );
    }

    #[test]
    fn rejects_reserved_input_name() {
        let err = parse_config(
            r#"
            [inputs.Effects]
            type = "zaps"
            relay_addrs = ["wss://relay.damus.io"]
            naddr = "naddr1example"
        "#,
        )
        .unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("input name 'Effects' is reserved"), "{msg}");
    }

    #[test]
    fn project_config_still_parses() {
        let cfg = load_config().unwrap();
        assert!(cfg.outputs.contains_key("lights"));
        assert!(cfg.actions.contains_key("cue2"));
        assert!(cfg.inputs.contains_key("nwc"));
        assert!(cfg.inputs.contains_key("boostboard"));
        assert!(cfg.inputs.contains_key("zaps"));
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
        assert_eq!(cfg.splitkit, again.splitkit);
        assert_eq!(cfg.inputs, again.inputs);
        assert_eq!(cfg.outputs.len(), again.outputs.len());
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
        assert!(parse_config("outputs = 1").is_err());
        assert!(previous.outputs.contains_key("lights"));
    }
}
