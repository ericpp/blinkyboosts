use crate::config::{Config, BoostBoard, NWC, OSC, ArtNet, Sacn, WLed, Zaps, BoostFiltersConfig};
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
    expanded: std::collections::HashMap<String, bool>,
    test_amount: String,
    sat_total: i64,
}

impl BlinkyBoostsApp {
    pub fn new(config: Config, tx: mpsc::Sender<GuiMessage>, rx: mpsc::Receiver<GuiMessage>) -> Self {
        let mut statuses = std::collections::HashMap::new();
        for (name, enabled) in [
            ("NWC", config.nwc.is_some()),
            ("Boostboard", config.boostboard.is_some()),
            ("Zaps", config.zaps.is_some()),
            ("WLED", config.wled.is_some()),
            ("OSC", config.osc.is_some()),
            ("Art-Net", config.artnet.is_some()),
            ("sACN", config.sacn.is_some()),
        ] {
            statuses.insert(
                name.to_string(),
                if enabled { ComponentStatus::Enabled } else { ComponentStatus::Disabled }
            );
        }

        Self {
            config: config.clone(),
            modified_config: config,
            statuses,
            recent_boosts: Vec::new(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
            show_save_dialog: false,
            save_error: None,
            expanded: std::collections::HashMap::new(),
            test_amount: "100".to_string(),
            sat_total: 0,
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
                    }
                    Err(e) => self.save_error = Some(format!("Write failed: {}", e)),
                }
            }
            Err(e) => self.save_error = Some(format!("Serialize failed: {}", e)),
        }
    }

    fn process_messages(&mut self) {
        if let Ok(mut rx) = self.rx.try_lock() {
            while let Ok(msg) = rx.try_recv() {
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
                    GuiMessage::StartListener(_) | GuiMessage::StopListener(_) => {
                        // These are handled by main.rs, not by the GUI
                    }
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
            "WLED" => {
                if enabled {
                    cfg.wled = None;
                } else {
                    cfg.wled = Some(orig_cfg.wled.clone().unwrap_or_else(||
                        WLed {
                            host: String::new(), boost_playlist: "BOOST".into(), brightness: 128,
                            segments: None, presets: None, playlists: None, setup: false, force: false,
                        }
                    ));
                }
            },
            "OSC" => {
                if enabled {
                    cfg.osc = None;
                } else {
                    cfg.osc = Some(orig_cfg.osc.clone().unwrap_or_else(||
                        OSC { address: String::new() }
                    ));
                }
            },
            "Art-Net" => {
                if enabled {
                    cfg.artnet = None;
                } else {
                    cfg.artnet = Some(orig_cfg.artnet.clone().unwrap_or_else(||
                        ArtNet { broadcast_address: String::new(), local_address: None, universe: Some(0) }
                    ));
                }
            },
            "sACN" => {
                if enabled {
                    cfg.sacn = None;
                } else {
                    cfg.sacn = Some(orig_cfg.sacn.clone().unwrap_or_else(||
                        Sacn { broadcast_address: String::new(), universe: Some(1) }
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
            "WLED" => {
                if let Some(wled) = &mut self.modified_config.wled {
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
            }
            "OSC" => {
                if let Some(osc) = &mut self.modified_config.osc {
                    ui.horizontal(|ui| {
                        ui.label("Address:");
                        if ui.text_edit_singleline(&mut osc.address).changed() {
                            *changed = true;
                        }
                    });
                }
            }
            "Art-Net" => {
                if let Some(artnet) = &mut self.modified_config.artnet {
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
            }
            "sACN" => {
                if let Some(sacn) = &mut self.modified_config.sacn {
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
            }
            _ => {}
        }
    }
}

impl eframe::App for BlinkyBoostsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_messages();
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("BlinkyBoosts");
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
                for name in ["NWC", "Boostboard", "Zaps"] {
                    self.render_component(&mut cols[0], name);
                }

                cols[1].heading("Outputs");
                cols[1].separator();
                for name in ["WLED", "OSC", "Art-Net", "sACN"] {
                    self.render_component(&mut cols[1], name);
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

            ui.add_space(20.0);
            ui.heading("Recent Boosts");
            ui.separator();
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
                osc: None,
                artnet: None,
                sacn: None,
                wled: None,
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
