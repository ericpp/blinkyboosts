use crate::config::{Config, BoostBoard, NWC, OSC, ArtNet, Sacn, WLed, Zaps};
use eframe::egui;
use egui::{Color32, RichText, Ui, ViewportBuilder};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// Status enum to track component status
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
            ComponentStatus::Disabled => Color32::from_rgb(150, 150, 150), // Light grey for dark background
            ComponentStatus::Enabled => Color32::from_rgb(100, 255, 100), // Bright green for enabled
            ComponentStatus::Running => Color32::from_rgb(100, 180, 255), // Bright blue for running
            ComponentStatus::Error(_) => Color32::from_rgb(255, 100, 100), // Bright red for dark background
        }
    }

    pub fn display_text(&self) -> String {
        match self {
            ComponentStatus::Disabled => "Disabled".to_string(),
            ComponentStatus::Enabled => "Enabled".to_string(),
            ComponentStatus::Running => "Running".to_string(),
            ComponentStatus::Error(msg) => format!("Error: {}", msg),
        }
    }
}

// Message types for communication between GUI and background tasks
pub enum GuiMessage {
    UpdateStatus(String, ComponentStatus),
    BoostReceived(String, i64, Vec<String>),
    TestTrigger(i64),
}

// Main application state
pub struct BlinkyBoostsApp {
    config: Config,
    modified_config: Config,
    component_statuses: std::collections::HashMap<String, ComponentStatus>,
    recent_boosts: Vec<(String, i64, Vec<String>, Instant)>,
    tx: mpsc::Sender<GuiMessage>,
    rx: Arc<Mutex<mpsc::Receiver<GuiMessage>>>,
    show_save_dialog: bool,
    save_error: Option<String>,
    show_settings: std::collections::HashMap<String, bool>,
    test_sat_amount: String,
}

impl BlinkyBoostsApp {
    pub fn new(config: Config, tx: mpsc::Sender<GuiMessage>, rx: mpsc::Receiver<GuiMessage>) -> Self {
        let mut app = BlinkyBoostsApp {
            config: config.clone(),
            modified_config: config,
            component_statuses: std::collections::HashMap::new(),
            recent_boosts: Vec::new(),
            tx,
            rx: Arc::new(Mutex::new(rx)),
            show_save_dialog: false,
            save_error: None,
            show_settings: std::collections::HashMap::new(),
            test_sat_amount: "100".to_string(),
        };

        // Initialize component statuses
        app.component_statuses.insert("NWC".to_string(),
            if app.config.nwc.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("Boostboard".to_string(),
            if app.config.boostboard.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("Zaps".to_string(),
            if app.config.zaps.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("WLED".to_string(),
            if app.config.wled.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("OSC".to_string(),
            if app.config.osc.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("Art-Net".to_string(),
            if app.config.artnet.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });
        app.component_statuses.insert("sACN".to_string(),
            if app.config.sacn.is_some() { ComponentStatus::Enabled } else { ComponentStatus::Disabled });

        // Initialize settings visibility
        app.show_settings.insert("NWC".to_string(), false);
        app.show_settings.insert("Boostboard".to_string(), false);
        app.show_settings.insert("Zaps".to_string(), false);
        app.show_settings.insert("WLED".to_string(), false);
        app.show_settings.insert("OSC".to_string(), false);
        app.show_settings.insert("Art-Net".to_string(), false);
        app.show_settings.insert("sACN".to_string(), false);

        app
    }

    fn save_config(&mut self) {
        // Save the modified config to file
        match toml::to_string(&self.modified_config) {
            Ok(toml_str) => {
                match std::fs::write("./config.toml", toml_str) {
                    Ok(_) => {
                        self.config = self.modified_config.clone();
                        self.show_save_dialog = false;
                        self.save_error = None;
                    },
                    Err(e) => {
                        self.save_error = Some(format!("Failed to write config file: {}", e));
                    }
                }
            },
            Err(e) => {
                self.save_error = Some(format!("Failed to serialize config: {}", e));
            }
        }
    }

    fn process_messages(&mut self) {
        if let Ok(mut rx) = self.rx.try_lock() {
            while let Ok(message) = rx.try_recv() {
                match message {
                    GuiMessage::UpdateStatus(component, status) => {
                        self.component_statuses.insert(component, status);
                    },
                    GuiMessage::BoostReceived(source, amount, effects) => {
                        self.recent_boosts.push((source, amount, effects, Instant::now()));
                    },
                    GuiMessage::TestTrigger(_) => {
                        // TestTrigger messages are handled by the background task
                        // They shouldn't reach here, but we handle it for completeness
                    }
                }
            }
        }

    }

    fn render_component_status(&mut self, ui: &mut Ui, component: &str) {
        let status = self.component_statuses.get(component).cloned().unwrap_or(ComponentStatus::Disabled);

        ui.horizontal(|ui| {
            ui.label(component);
            ui.label(RichText::new(status.display_text()).color(status.color()));

            // Toggle button
            let is_enabled = status != ComponentStatus::Disabled;
            if ui.button(if is_enabled { "Disable" } else { "Enable" }).clicked() {
                match component {
                    "NWC" => {
                        if is_enabled {
                            self.modified_config.nwc = None;
                        } else if self.modified_config.nwc.is_none() {
                            self.modified_config.nwc = Some(NWC { uri: "".to_string() });
                        }
                    },
                    "Boostboard" => {
                        if is_enabled {
                            self.modified_config.boostboard = None;
                        } else if self.modified_config.boostboard.is_none() {
                            self.modified_config.boostboard = Some(BoostBoard {
                                relay_addr: "".to_string(),
                                pubkey: "".to_string(),
                            });
                        }
                    },
                    "Zaps" => {
                        if is_enabled {
                            self.modified_config.zaps = None;
                        } else if self.modified_config.zaps.is_none() {
                            self.modified_config.zaps = Some(Zaps {
                                relay_addrs: vec!["".to_string()],
                                naddr: "".to_string(),
                            });
                        }
                    },
                    "WLED" => {
                        if is_enabled {
                            self.modified_config.wled = None;
                        } else if self.modified_config.wled.is_none() {
                            self.modified_config.wled = Some(WLed {
                                host: "".to_string(),
                                boost_playlist: "BOOST".to_string(),
                                brightness: 128,
                                segments: None,
                                presets: None,
                                playlists: None,
                                setup: false,
                                force: false,
                            });
                        }
                    },
                    "OSC" => {
                        if is_enabled {
                            self.modified_config.osc = None;
                        } else if self.modified_config.osc.is_none() {
                            self.modified_config.osc = Some(OSC {
                                address: "".to_string(),
                            });
                        }
                    },
                    "Art-Net" => {
                        if is_enabled {
                            self.modified_config.artnet = None;
                        } else if self.modified_config.artnet.is_none() {
                            self.modified_config.artnet = Some(ArtNet {
                                broadcast_address: "".to_string(),
                                local_address: None,
                                universe: Some(0),
                            });
                        }
                    },
                    "sACN" => {
                        if is_enabled {
                            self.modified_config.sacn = None;
                        } else if self.modified_config.sacn.is_none() {
                            self.modified_config.sacn = Some(Sacn {
                                broadcast_address: "".to_string(),
                                universe: Some(1),
                            });
                        }
                    },
                    _ => {}
                }

                // Update status
                self.component_statuses.insert(component.to_string(),
                    if is_enabled { ComponentStatus::Disabled } else { ComponentStatus::Enabled });

                self.show_save_dialog = true;
            }

            // Settings button - always show, even when disabled
            if ui.button("Settings").clicked() {
                // If component is disabled and config doesn't exist, create it
                if status == ComponentStatus::Disabled {
                    match component {
                        "NWC" => {
                            if self.modified_config.nwc.is_none() {
                                self.modified_config.nwc = Some(NWC { uri: "".to_string() });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "Boostboard" => {
                            if self.modified_config.boostboard.is_none() {
                                self.modified_config.boostboard = Some(BoostBoard {
                                    relay_addr: "".to_string(),
                                    pubkey: "".to_string(),
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "Zaps" => {
                            if self.modified_config.zaps.is_none() {
                                self.modified_config.zaps = Some(Zaps {
                                    relay_addrs: vec!["".to_string()],
                                    naddr: "".to_string(),
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "WLED" => {
                            if self.modified_config.wled.is_none() {
                                self.modified_config.wled = Some(WLed {
                                    host: "".to_string(),
                                    boost_playlist: "BOOST".to_string(),
                                    brightness: 128,
                                    segments: None,
                                    presets: None,
                                    playlists: None,
                                    setup: false,
                                    force: false,
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "OSC" => {
                            if self.modified_config.osc.is_none() {
                                self.modified_config.osc = Some(OSC {
                                    address: "".to_string(),
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "Art-Net" => {
                            if self.modified_config.artnet.is_none() {
                                self.modified_config.artnet = Some(ArtNet {
                                    broadcast_address: "".to_string(),
                                    local_address: None,
                                    universe: Some(0),
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        "sACN" => {
                            if self.modified_config.sacn.is_none() {
                                self.modified_config.sacn = Some(Sacn {
                                    broadcast_address: "".to_string(),
                                    universe: Some(1),
                                });
                                self.component_statuses.insert(component.to_string(), ComponentStatus::Enabled);
                            }
                        },
                        _ => {}
                    }
                    self.show_save_dialog = true;
                }
                let current = self.show_settings.get(component).cloned().unwrap_or(false);
                self.show_settings.insert(component.to_string(), !current);
            }
        });

        // Show settings if expanded - now works even when disabled
        if *self.show_settings.get(component).unwrap_or(&false) {
            ui.indent(component, |ui| {
                match component {
                    "NWC" => {
                        if let Some(nwc) = &mut self.modified_config.nwc {
                            ui.horizontal(|ui| {
                                ui.label("URI:");
                                if ui.text_edit_singleline(&mut nwc.uri).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                        }
                    },
                    "Boostboard" => {
                        if let Some(boostboard) = &mut self.modified_config.boostboard {
                            ui.horizontal(|ui| {
                                ui.label("Relay Address:");
                                if ui.text_edit_singleline(&mut boostboard.relay_addr).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Public Key:");
                                if ui.text_edit_singleline(&mut boostboard.pubkey).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                        }
                    },
                    "Zaps" => {
                        if let Some(zaps) = &mut self.modified_config.zaps {
                            ui.horizontal(|ui| {
                                ui.label("NADDR:");
                                if ui.text_edit_singleline(&mut zaps.naddr).changed() {
                                    self.show_save_dialog = true;
                                }
                            });

                            ui.label("Relay Addresses:");
                            let mut to_remove = None;
                            for (i, addr) in zaps.relay_addrs.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui.text_edit_singleline(addr).changed() {
                                        self.show_save_dialog = true;
                                    }
                                    if ui.button("Remove").clicked() {
                                        to_remove = Some(i);
                                        self.show_save_dialog = true;
                                    }
                                });
                            }

                            if let Some(idx) = to_remove {
                                zaps.relay_addrs.remove(idx);
                            }

                            if ui.button("Add Relay").clicked() {
                                zaps.relay_addrs.push("".to_string());
                                self.show_save_dialog = true;
                            }
                        }
                    },
                    "WLED" => {
                        if let Some(wled) = &mut self.modified_config.wled {
                            ui.horizontal(|ui| {
                                ui.label("Host:");
                                if ui.text_edit_singleline(&mut wled.host).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Boost Playlist:");
                                if ui.text_edit_singleline(&mut wled.boost_playlist).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Brightness:");
                                if ui.add(egui::Slider::new(&mut wled.brightness, 0..=255)).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Setup:");
                                if ui.checkbox(&mut wled.setup, "").changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Force:");
                                if ui.checkbox(&mut wled.force, "").changed() {
                                    self.show_save_dialog = true;
                                }
                            });

                            // Segments, presets, and playlists would need more complex UI
                            // This is a simplified version
                            ui.label("Note: For advanced WLED settings (segments, presets, playlists), please edit config.toml directly.");
                        }
                    },
                    "OSC" => {
                        if let Some(osc) = &mut self.modified_config.osc {
                            ui.horizontal(|ui| {
                                ui.label("Address:");
                                if ui.text_edit_singleline(&mut osc.address).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                        }
                    },
                    "Art-Net" => {
                        if let Some(artnet) = &mut self.modified_config.artnet {
                            ui.horizontal(|ui| {
                                ui.label("Broadcast Address:");
                                if ui.text_edit_singleline(&mut artnet.broadcast_address).changed() {
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Local Address:");
                                let mut local_addr_str = artnet.local_address.clone().unwrap_or_default();
                                if ui.text_edit_singleline(&mut local_addr_str).changed() {
                                    artnet.local_address = if local_addr_str.is_empty() {
                                        None
                                    } else {
                                        Some(local_addr_str)
                                    };
                                    self.show_save_dialog = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Universe:");
                                let mut universe_str = artnet.universe.unwrap_or(0).to_string();
                                if ui.text_edit_singleline(&mut universe_str).changed() {
                                    if let Ok(u) = universe_str.parse::<u16>() {
                                        artnet.universe = Some(u);
                                        self.show_save_dialog = true;
                                    }
                                }
                            });
                        }
                    },
                    "sACN" => {
                        if let Some(sacn) = &mut self.modified_config.sacn {
                            ui.horizontal(|ui| {
                                ui.label("Universe:");
                                let mut universe_str = sacn.universe.unwrap_or(1).to_string();
                                if ui.text_edit_singleline(&mut universe_str).changed() {
                                    if let Ok(u) = universe_str.parse::<u16>() {
                                        sacn.universe = Some(u);
                                        self.show_save_dialog = true;
                                    }
                                }
                            });
                        }
                    },
                    _ => {}
                }
            });
        }
    }
}

impl eframe::App for BlinkyBoostsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process any pending messages
        self.process_messages();

        // Request repaint frequently to update status
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("BlinkyBoosts");

            ui.add_space(10.0);

            // Split components into input/output columns for quicker scanning
            ui.columns(2, |columns| {
                columns[0].heading("Inputs");
                columns[0].separator();
                self.render_component_status(&mut columns[0], "NWC");
                self.render_component_status(&mut columns[0], "Boostboard");
                self.render_component_status(&mut columns[0], "Zaps");

                columns[1].heading("Outputs");
                columns[1].separator();
                self.render_component_status(&mut columns[1], "WLED");
                self.render_component_status(&mut columns[1], "OSC");
                self.render_component_status(&mut columns[1], "Art-Net");
                self.render_component_status(&mut columns[1], "sACN");
            });

            ui.add_space(20.0);

            // Test section
            ui.heading("Test Effects");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Sat Amount:");
                ui.text_edit_singleline(&mut self.test_sat_amount);
                if ui.button("Trigger Test").clicked() {
                    if let Ok(sats) = self.test_sat_amount.parse::<i64>() {
                        if sats > 0 {
                            if let Err(e) = self.tx.try_send(GuiMessage::TestTrigger(sats)) {
                                eprintln!("Failed to send test trigger: {:?}", e);
                            }
                            // Effects will be added when the test trigger is processed via BoostReceived message
                        }
                    }
                }
            });

            ui.add_space(20.0);

            // Recent boosts section
            ui.heading("Recent Boosts");
            ui.separator();

            if self.recent_boosts.is_empty() {
                ui.label("No recent boosts");
            } else {
                for (source, amount, effects, time) in self.recent_boosts.iter().rev() {
                    let elapsed = time.elapsed().as_secs();
                    ui.horizontal(|ui| {
                        let effects_str = if effects.is_empty() {
                            "none".to_string()
                        } else {
                            effects.join(", ")
                        };
                        ui.label(format!("[{}s ago] {} sats from {} → {}", elapsed, amount, source, effects_str));
                    });
                }
            }

            // Save dialog
            if self.show_save_dialog {
                egui::Window::new("Save Configuration")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Configuration has been modified. Save changes?");

                        if let Some(error) = &self.save_error {
                            ui.label(RichText::new(error).color(Color32::from_rgb(255, 100, 100)));
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

// Function to launch the GUI
pub fn run_gui(tx: mpsc::Sender<GuiMessage>, rx: mpsc::Receiver<GuiMessage>) -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = match crate::config::load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            // Create a default config if loading fails
            Config {
                nwc: None,
                boostboard: None,
                zaps: None,
                osc: None,
                artnet: None,
                sacn: None,
                wled: None,
            }
        }
    };

    let app = BlinkyBoostsApp::new(config, tx, rx);

    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title("BlinkyBoosts"),
        ..Default::default()
    };

    // Run the app with the receiver
    eframe::run_native(
        "BlinkyBoosts",
        options,
        Box::new(|cc| {
            // Increase font sizes and set dark mode
            let mut style = (*cc.egui_ctx.style()).clone();
            style.text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::new(16.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(16.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Heading,
                egui::FontId::new(24.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Name("Heading2".into()),
                egui::FontId::new(20.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Monospace,
                egui::FontId::new(14.0, egui::FontFamily::Monospace),
            );
            // Set dark mode visuals
            style.visuals = egui::style::Visuals::dark();
            // Customize dark mode colors for better appearance and higher contrast
            style.visuals.panel_fill = Color32::from_rgb(20, 20, 20); // Very dark background
            style.visuals.window_fill = Color32::from_rgb(15, 15, 15); // Almost black for windows
            style.visuals.extreme_bg_color = Color32::from_rgb(30, 30, 30); // Slightly lighter for contrast
            style.visuals.faint_bg_color = Color32::from_rgb(35, 35, 35); // For subtle backgrounds
            // Make text off-white/light grey for comfortable viewing
            style.visuals.override_text_color = Some(Color32::from_rgb(220, 220, 220)); // Off-white/light grey text
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 40, 40); // Darker widget backgrounds
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(35, 35, 35);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 50, 50); // Slightly lighter on hover
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(60, 60, 60); // Lighter when active
            cc.egui_ctx.set_style(style);

            Box::new(app)
        }),
    )?;

    Ok(())
}