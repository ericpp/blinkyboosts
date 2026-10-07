use crate::config::{Config, BoostBoard, NWC, SplitKit, Target, Zaps, BoostFiltersConfig};
use eframe::egui;
use egui::{Color32, RichText, Ui, ViewportBuilder};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use chrono::Local;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
pub enum ComponentStatus {
    Disabled,
    Enabled,
    Running,
    Error(String),
}

impl ComponentStatus {
    pub fn color(&self) -> Color32 {
        match self {
            Self::Disabled => Color32::GRAY,
            Self::Enabled => Color32::GREEN,
            Self::Running => Color32::LIGHT_BLUE,
            Self::Error(_) => Color32::RED,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Disabled => "Disabled",
            Self::Enabled => "Enabled",
            Self::Running => "Running",
            Self::Error(_) => "Error",
        }
    }
}

pub enum GuiMessage {
    UpdateStatus(String, ComponentStatus),
    BoostReceived(String, i64, Vec<String>),
    TestTrigger(i64),
    UpdateSatTotal(i64),
    StartListener(String),
    StopListener(String),
    ReloadConfig,
    ConfigReloaded(Box<Config>),
    ConfigReloadFailed(String),
    RunAction(String),
    /// `None` means the action was sent. `Some` is the error text.
    ActionFinished(String, Option<String>),
}

enum ActionRun {
    Running,
    Ok(chrono::DateTime<Local>),
    Failed(String),
}

pub struct BlinkyBoostsApp {
    config: Config,
    modified_config: Config,
    statuses: std::collections::HashMap<String, ComponentStatus>,
    recent_boosts: Vec<(String, i64, Vec<String>, chrono::DateTime<Local>)>,
    tx: mpsc::Sender<GuiMessage>,
    rx: Arc<Mutex<mpsc::Receiver<GuiMessage>>>,
    show_save_dialog: bool,
    save_error: Option<String>,
    reload_note: Option<String>,
    reload_error: Option<String>,
    expanded: std::collections::HashMap<String, bool>,
    test_amount: String,
    sat_total: i64,
    action_status: std::collections::HashMap<String, ActionRun>,
    selected_action: Option<String>,
}

impl BlinkyBoostsApp {
    pub fn new(config: Config, tx: mpsc::Sender<GuiMessage>, rx: mpsc::Receiver<GuiMessage>) -> Self {
        let statuses = component_statuses(&config);

        Self {
            config: config.clone(),
            modified_config: config,
            statuses,
            recent_boosts: Vec::new(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
            show_save_dialog: false,
            save_error: None,
            reload_note: None,
            reload_error: None,
            expanded: std::collections::HashMap::new(),
            test_amount: "100".to_string(),
            sat_total: 0,
            action_status: std::collections::HashMap::new(),
            selected_action: None,
        }
    }

    fn save_config(&mut self) {
        match toml::to_string(&self.modified_config) {
            Ok(toml_str) => {
                match std::fs::write("./config.toml", toml_str) {
                    Ok(_) => {
                        self.config = self.modified_config.clone();
                        self.show_save_dialog = false;
                        self.save_error = None;
                        let _ = self.tx.try_send(GuiMessage::ReloadConfig);
                    }
                    Err(e) => self.save_error = Some(format!("Write failed: {}", e)),
                }
            }
            Err(e) => self.save_error = Some(format!("Serialize failed: {}", e)),
        }
    }

    fn process_messages(&mut self) {
        let messages = {
            let Ok(mut rx) = self.rx.try_lock() else { return };
            let mut messages = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                messages.push(msg);
            }
            messages
        };
        for msg in messages {
            match msg {
                GuiMessage::UpdateStatus(comp, status) => {
                    self.statuses.insert(comp, status);
                }
                GuiMessage::BoostReceived(src, amt, fx) => {
                    self.recent_boosts.push((src, amt, fx, Local::now()));
                }
                GuiMessage::TestTrigger(_) => {}
                GuiMessage::UpdateSatTotal(total) => {
                    self.sat_total = total;
                }
                GuiMessage::StartListener(_) | GuiMessage::StopListener(_) | GuiMessage::ReloadConfig | GuiMessage::RunAction(_) => {
                    // These are handled by main.rs, not by the GUI
                }
                GuiMessage::ActionFinished(name, err) => {
                    self.action_status.insert(
                        name,
                        match err {
                            None => ActionRun::Ok(Local::now()),
                            Some(msg) => ActionRun::Failed(msg),
                        },
                    );
                }
                GuiMessage::ConfigReloaded(cfg) => {
                    self.adopt_config(*cfg);
                    self.show_save_dialog = false;
                    self.save_error = None;
                    self.reload_error = None;
                    self.reload_note = Some("Config reloaded".to_string());
                }
                GuiMessage::ConfigReloadFailed(msg) => {
                    self.reload_error = Some(msg);
                    self.reload_note = None;
                }
            }
        }
    }

    fn toggle_component(&mut self, name: &str, enabled: bool) {
        let cfg = &mut self.modified_config;

        // Get the current config value or create defaults
        let orig_cfg = &self.config;

        match name {
            "NWC" => {
                if enabled {
                    cfg.nwc = None;
                } else {
                    cfg.nwc = Some(orig_cfg.nwc.clone().unwrap_or_else(||
                        NWC { uri: "".into(), filters: BoostFiltersConfig::default() }
                    ));
                }
            },
            "Boostboard" => {
                if enabled {
                    cfg.boostboard = None;
                } else {
                    cfg.boostboard = Some(orig_cfg.boostboard.clone().unwrap_or_else(||
                        BoostBoard { relay_addrs: vec![], pubkey: "".into(), filters: BoostFiltersConfig::default() }
                    ));
                }
            },
            "Zaps" => {
                if enabled {
                    cfg.zaps = None;
                } else {
                    cfg.zaps = Some(orig_cfg.zaps.clone().unwrap_or_else(||
                        Zaps { relay_addrs: vec![], naddr: String::new(), load_since: None }
                    ));
                }
            },
            "SplitKit" => {
                if enabled {
                    cfg.splitkit = None;
                } else {
                    cfg.splitkit = Some(orig_cfg.splitkit.clone().unwrap_or_else(||
                        SplitKit { url: String::new() }
                    ));
                }
            },
            _ => return,
        }

        // Send start/stop message to control the listener
        if enabled {
            let _ = self.tx.try_send(GuiMessage::StopListener(name.to_string()));
        } else {
            let _ = self.tx.try_send(GuiMessage::StartListener(name.to_string()));
        }

        self.statuses.insert(
            name.to_string(),
            if enabled { ComponentStatus::Disabled } else { ComponentStatus::Enabled }
        );
        self.show_save_dialog = true;
    }

    /// Replace the displayed config. Keep a Running or Error status for anything still enabled,
    /// so a listener that did not restart does not flip back to Enabled.
    fn adopt_config(&mut self, config: Config) {
        let fresh = component_statuses(&config);
        for (name, status) in fresh {
            let keep_live = matches!(
                self.statuses.get(&name),
                Some(ComponentStatus::Running | ComponentStatus::Error(_))
            );
            let still_enabled = status != ComponentStatus::Disabled;
            if keep_live && still_enabled {
                continue;
            }
            self.statuses.insert(name, status);
        }
        self.statuses.retain(|name, _| {
            matches!(name.as_str(), "NWC" | "Boostboard" | "Zaps" | "SplitKit" | "Effects")
                || config.targets.contains_key(name)
        });
        self.action_status.retain(|name, _| config.actions.contains_key(name));
        if self.selected_action.as_ref().is_some_and(|name| !config.actions.contains_key(name)) {
            self.selected_action = None;
        }
        self.config = config.clone();
        self.modified_config = config;
    }

    fn render_component(&mut self, ui: &mut Ui, name: &str) {
        let status = self.statuses.get(name).cloned().unwrap_or(ComponentStatus::Disabled);
        let enabled = status != ComponentStatus::Disabled;

        ui.horizontal(|ui| {
            ui.set_height(20.0);
            ui.label(name);
            ui.label(RichText::new(status.text()).color(status.color()));

            let btn_text = if enabled { "Disable" } else { "Enable" };
            if ui.add_sized([80.0, 20.0], egui::Button::new(btn_text)).clicked() {
                self.toggle_component(name, enabled);
            }

            if ui.add_sized([30.0, 20.0], egui::Button::new("⚙")).clicked() {
                if !enabled {
                    self.toggle_component(name, false);
                }
                let expanded = self.expanded.entry(name.to_string()).or_insert(false);
                *expanded = !*expanded;
            }
        });

        if *self.expanded.get(name).unwrap_or(&false) {
            ui.indent(name, |ui| self.render_settings(ui, name));
        }
    }

    fn render_settings(&mut self, ui: &mut Ui, name: &str) {
        let changed = &mut self.show_save_dialog;

        match name {
            "NWC" => {
                if let Some(nwc) = &mut self.modified_config.nwc {
                    ui.horizontal(|ui| {
                        ui.label("URI:");
                        if ui.text_edit_singleline(&mut nwc.uri).changed() {
                            *changed = true;
                        }
                    });
                }
            }
            "Boostboard" => {
                if let Some(bb) = &mut self.modified_config.boostboard {
                    ui.horizontal(|ui| {
                        ui.label("Pubkey:");
                        if ui.text_edit_singleline(&mut bb.pubkey).changed() {
                            *changed = true;
                        }
                    });
                    ui.label("Relays:");
                    let mut remove_idx = None;
                    for (i, addr) in bb.relay_addrs.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.text_edit_singleline(addr).changed() {
                                *changed = true;
                            }
                            if ui.button("✖").clicked() {
                                remove_idx = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove_idx {
                        bb.relay_addrs.remove(i);
                        *changed = true;
                    }
                    if ui.button("+ Add").clicked() {
                        bb.relay_addrs.push("".into());
                        *changed = true;
                    }
                }
            }
            "Zaps" => {
                if let Some(zaps) = &mut self.modified_config.zaps {
                    ui.horizontal(|ui| {
                        ui.label("NADDR:");
                        if ui.text_edit_singleline(&mut zaps.naddr).changed() {
                            *changed = true;
                        }
                    });
                    ui.label("Relays:");
                    let mut remove_idx = None;
                    for (i, addr) in zaps.relay_addrs.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.text_edit_singleline(addr).changed() {
                                *changed = true;
                            }
                            if ui.button("✖").clicked() {
                                remove_idx = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove_idx {
                        zaps.relay_addrs.remove(i);
                        *changed = true;
                    }
                    if ui.button("+ Add").clicked() {
                        zaps.relay_addrs.push("".into());
                        *changed = true;
                    }
                }
            }
            "SplitKit" => {
                if let Some(splitkit) = &mut self.modified_config.splitkit {
                    ui.horizontal(|ui| {
                        ui.label("URL:");
                        if ui.text_edit_singleline(&mut splitkit.url).changed() {
                            *changed = true;
                        }
                    });
                }
            }
            _ => {}
        }
    }

    fn toggle_target(&mut self, name: &str) {
        let Some(target) = self.modified_config.targets.get_mut(name) else { return };
        target.enabled = !target.enabled;
        let enabled = target.enabled;
        self.statuses.insert(
            name.to_string(),
            if enabled { ComponentStatus::Enabled } else { ComponentStatus::Disabled },
        );
        self.show_save_dialog = true;
    }

    fn render_target(&mut self, ui: &mut Ui, name: &str) {
        let (enabled, label) = match self.modified_config.targets.get(name) {
            Some(target) => (target.enabled, target.kind.label()),
            None => return,
        };
        let status = if enabled {
            self.statuses.get(name).cloned().unwrap_or(ComponentStatus::Enabled)
        } else {
            ComponentStatus::Disabled
        };

        ui.horizontal(|ui| {
            ui.set_height(20.0);
            ui.label(format!("{} ({})", name, label));
            ui.label(RichText::new(status.text()).color(status.color()));

            let btn_text = if enabled { "Disable" } else { "Enable" };
            if ui.add_sized([80.0, 20.0], egui::Button::new(btn_text)).clicked() {
                self.toggle_target(name);
            }

            if ui.add_sized([30.0, 20.0], egui::Button::new("⚙")).clicked() {
                let expanded = self.expanded.entry(name.to_string()).or_insert(false);
                *expanded = !*expanded;
            }
        });

        if *self.expanded.get(name).unwrap_or(&false) {
            ui.indent(name, |ui| self.render_target_settings(ui, name));
        }
    }

    fn render_target_settings(&mut self, ui: &mut Ui, name: &str) {
        let changed = &mut self.show_save_dialog;
        let Some(target) = self.modified_config.targets.get_mut(name) else { return };

        match &mut target.kind {
            Target::Osc { address } => {
                ui.horizontal(|ui| {
                    ui.label("Address:");
                    if ui.text_edit_singleline(address).changed() {
                        *changed = true;
                    }
                });
            }
            Target::Artnet(artnet) => {
                ui.horizontal(|ui| {
                    ui.label("Broadcast:");
                    if ui.text_edit_singleline(&mut artnet.broadcast_address).changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Local:");
                    let mut local = artnet.local_address.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut local).changed() {
                        artnet.local_address = if local.is_empty() { None } else { Some(local) };
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Universe:");
                    let mut s = artnet.universe.unwrap_or(0).to_string();
                    if ui.text_edit_singleline(&mut s).changed() {
                        if let Ok(n) = s.parse() {
                            artnet.universe = Some(n);
                            *changed = true;
                        }
                    }
                });
            }
            Target::Sacn(sacn) => {
                ui.horizontal(|ui| {
                    ui.label("Universe:");
                    let mut s = sacn.universe.unwrap_or(1).to_string();
                    if ui.text_edit_singleline(&mut s).changed() {
                        if let Ok(n) = s.parse() {
                            sacn.universe = Some(n);
                            *changed = true;
                        }
                    }
                });
            }
            Target::Wled(wled) => {
                ui.horizontal(|ui| {
                    ui.label("Host:");
                    if ui.text_edit_singleline(&mut wled.host).changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Playlist:");
                    if ui.text_edit_singleline(&mut wled.boost_playlist).changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Brightness:");
                    if ui.add(egui::Slider::new(&mut wled.brightness, 0..=255)).changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Setup:");
                    if ui.checkbox(&mut wled.setup, "").changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Force:");
                    if ui.checkbox(&mut wled.force, "").changed() {
                        *changed = true;
                    }
                });
            }
            Target::Obs(obs) => {
                ui.horizontal(|ui| {
                    ui.label("Host:");
                    if ui.text_edit_singleline(&mut obs.host).changed() {
                        *changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Port:");
                    let mut s = obs.port.map(|p| p.to_string()).unwrap_or_default();
                    if ui.text_edit_singleline(&mut s).changed() {
                        if s.is_empty() {
                            obs.port = None;
                            *changed = true;
                        } else if let Ok(n) = s.parse() {
                            obs.port = Some(n);
                            *changed = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Password:");
                    let mut password = obs.password.clone().unwrap_or_default();
                    if ui.add(egui::TextEdit::singleline(&mut password).password(true)).changed() {
                        obs.password = if password.is_empty() { None } else { Some(password) };
                        *changed = true;
                    }
                });
            }
        }
    }

    fn render_actions(&mut self, ui: &mut Ui) {
        let rows: Vec<(String, Vec<(String, bool)>)> = self
            .config
            .actions
            .iter()
            .map(|(name, group)| {
                let mut targets = Vec::new();
                for action in group.as_slice() {
                    if targets.iter().any(|(target, _)| target == &action.target) {
                        continue;
                    }
                    let enabled = self
                        .config
                        .targets
                        .get(&action.target)
                        .is_some_and(|target| target.enabled);
                    targets.push((action.target.clone(), enabled));
                }
                (name.clone(), targets)
            })
            .collect();

        ui.add_space(10.0);
        ui.heading("Actions");
        ui.separator();
        if rows.is_empty() {
            ui.label("No actions configured");
            return;
        }

        if self.selected_action.as_ref().is_some_and(|name| !rows.iter().any(|(n, _)| n == name)) {
            self.selected_action = None;
        }

        ui.horizontal(|ui| {
            ui.set_height(20.0);
            let selected_text = self
                .selected_action
                .clone()
                .unwrap_or_else(|| "Select an action".to_string());
            egui::ComboBox::from_id_source("manual_action")
                .selected_text(selected_text)
                .width(240.0)
                .height(240.0)
                .show_ui(ui, |ui| {
                    for (name, targets) in &rows {
                        let summary = targets
                            .iter()
                            .map(|(target, _)| target.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let label = if summary.is_empty() {
                            name.clone()
                        } else {
                            format!("{name} ({summary})")
                        };
                        ui.selectable_value(&mut self.selected_action, Some(name.clone()), label);
                    }
                });

            let selected = self.selected_action.clone();
            if let Some(name) = selected.as_deref() {
                if let Some((_, targets)) = rows.iter().find(|(n, _)| n == name) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (i, (target, enabled)) in targets.iter().enumerate() {
                            if i > 0 {
                                ui.label(", ");
                            }
                            let text = if *enabled {
                                RichText::new(target)
                            } else {
                                RichText::new(target).color(Color32::GRAY)
                            };
                            ui.label(text);
                        }
                    });
                }
            }

            let running = selected.as_deref().is_some_and(|name| {
                matches!(self.action_status.get(name), Some(ActionRun::Running))
            });
            if ui
                .add_enabled(
                    selected.is_some() && !running,
                    egui::Button::new("Run").min_size(egui::vec2(80.0, 20.0)),
                )
                .clicked()
            {
                if let Some(name) = selected.clone() {
                    if self.tx.try_send(GuiMessage::RunAction(name.clone())).is_ok() {
                        self.action_status.insert(name, ActionRun::Running);
                    }
                }
            }

            if let Some(name) = selected.as_deref() {
                match self.action_status.get(name) {
                    Some(ActionRun::Running) => {
                        ui.label(RichText::new("Running...").color(Color32::LIGHT_BLUE));
                    }
                    Some(ActionRun::Ok(time)) => {
                        ui.label(
                            RichText::new(format!("OK {}", time.format("%H:%M:%S")))
                                .color(Color32::LIGHT_GREEN),
                        );
                    }
                    Some(ActionRun::Failed(err)) => {
                        ui.label(RichText::new("Failed").color(Color32::RED))
                            .on_hover_text(err);
                    }
                    None => {}
                }
            }
        });
    }

}

impl eframe::App for BlinkyBoostsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_messages();
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("BlinkyBoosts");
                if ui.button("Reload Config").clicked() {
                    let _ = self.tx.try_send(GuiMessage::ReloadConfig);
                }
            });
            if let Some(err) = &self.reload_error {
                ui.colored_label(Color32::RED, format!("Reload failed: {}", err));
            } else if let Some(note) = &self.reload_note {
                ui.label(RichText::new(note).color(Color32::LIGHT_GREEN));
            }
            ui.add_space(10.0);

            // Display sat total
            ui.horizontal(|ui| {
                ui.label(RichText::new("Total Sats:").size(18.0));
                ui.label(RichText::new(format!("{}", self.sat_total)).size(18.0).color(Color32::LIGHT_GREEN));
            });
            ui.add_space(10.0);

            ui.columns(2, |cols| {
                cols[0].heading("Inputs");
                cols[0].separator();
                for name in ["NWC", "Boostboard", "Zaps", "SplitKit"] {
                    self.render_component(&mut cols[0], name);
                }

                cols[1].heading("Outputs");
                cols[1].separator();
                let names: Vec<String> = self.modified_config.targets.keys().cloned().collect();
                if names.is_empty() {
                    cols[1].label("No targets configured");
                } else {
                    for name in names {
                        self.render_target(&mut cols[1], &name);
                    }
                }
            });

            ui.add_space(20.0);
            ui.heading("Test");
            ui.separator();
            ui.horizontal(|ui| {
                ui.set_height(20.0);
                ui.label("Sats:");
                ui.text_edit_singleline(&mut self.test_amount);
                if ui.add_sized([80.0, 20.0], egui::Button::new("Trigger")).clicked() {
                    if let Ok(sats) = self.test_amount.parse::<i64>() {
                        if sats > 0 {
                            let _ = self.tx.try_send(GuiMessage::TestTrigger(sats));
                        }
                    }
                }
            });

            self.render_actions(ui);

            ui.add_space(10.0);
            ui.heading("Recent Boosts");
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    if self.recent_boosts.is_empty() {
                        ui.label("No recent boosts");
                    } else {
                        for (src, amt, fx, time) in self.recent_boosts.iter().rev() {
                            let fx_str = if fx.is_empty() { "none" } else { &fx.join(", ") };
                            let time_str = time.format("%Y-%m-%d %H:%M:%S").to_string();
                            ui.label(format!("[{}] {} sats from {} → {}",
                                time_str, amt, src, fx_str));
                        }
                    }
                });

            if self.show_save_dialog {
                egui::Window::new("Save Configuration")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Save changes?");
                        if let Some(err) = &self.save_error {
                            ui.colored_label(Color32::RED, err);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                self.save_config();
                            }
                            if ui.button("Cancel").clicked() {
                                self.modified_config = self.config.clone();
                                self.show_save_dialog = false;
                                self.save_error = None;
                            }
                        });
                    });
            }
        });
    }
}

fn component_statuses(config: &Config) -> std::collections::HashMap<String, ComponentStatus> {
    let mut statuses = std::collections::HashMap::new();
    for (name, enabled) in [
        ("NWC", config.nwc.is_some()),
        ("Boostboard", config.boostboard.is_some()),
        ("Zaps", config.zaps.is_some()),
        ("SplitKit", config.splitkit.is_some()),
    ] {
        statuses.insert(
            name.to_string(),
            if enabled { ComponentStatus::Enabled } else { ComponentStatus::Disabled },
        );
    }
    for (name, target) in &config.targets {
        statuses.insert(
            name.clone(),
            if target.enabled { ComponentStatus::Enabled } else { ComponentStatus::Disabled },
        );
    }
    statuses
}

pub fn run_gui(tx: mpsc::Sender<GuiMessage>, rx: mpsc::Receiver<GuiMessage>)
    -> Result<(), Box<dyn std::error::Error>>
{
    let config = match crate::config::load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            Config {
                nwc: None,
                boostboard: None,
                zaps: None,
                splitkit: None,
                targets: std::collections::BTreeMap::new(),
                actions: std::collections::BTreeMap::new(),
                toggles: None,
            }
        }
    };

    let app = BlinkyBoostsApp::new(config, tx, rx);

    eframe::run_native(
        "BlinkyBoosts",
        eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_inner_size([800.0, 600.0])
                .with_min_inner_size([400.0, 300.0])
                .with_title("BlinkyBoosts"),
            ..Default::default()
        },
        Box::new(|cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.text_styles.insert(egui::TextStyle::Body,
                egui::FontId::new(16.0, egui::FontFamily::Proportional));
            style.text_styles.insert(egui::TextStyle::Heading,
                egui::FontId::new(24.0, egui::FontFamily::Proportional));
            style.visuals = egui::Visuals::dark();
            cc.egui_ctx.set_style(style);
            Box::new(app)
        }),
    )?;

    Ok(())
}
