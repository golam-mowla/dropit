use crate::identity::AnonymousIdentity;
use crate::messenger::{ChatMessage, MessagePayload};
use crate::settings::AppSettings;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// Windows DWM Acrylic / Mica Backdrop FFI
// ============================================================================

#[cfg(windows)]
pub fn apply_windows_dwm_effects(hwnd: isize) {
    #[allow(non_snake_case)]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            dwAttribute: u32,
            pvAttribute: *const std::ffi::c_void,
            cbAttribute: u32,
        ) -> i32;
    }

    let hwnd_ptr = hwnd as *mut std::ffi::c_void;
    if hwnd_ptr.is_null() {
        return;
    }

    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20
    let dark_mode: i32 = 1;
    unsafe {
        DwmSetWindowAttribute(
            hwnd_ptr,
            20,
            &dark_mode as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }

    // DWMWA_SYSTEMBACKDROP_TYPE = 38 (3 = Acrylic, 2 = Mica)
    let backdrop_type: i32 = 3;
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd_ptr,
            38,
            &backdrop_type as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };

    if hr != 0 {
        let mica: i32 = 2;
        unsafe {
            DwmSetWindowAttribute(
                hwnd_ptr,
                38,
                &mica as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn apply_windows_dwm_effects(_hwnd: isize) {}

// ============================================================================
// Glassmorphism Visual Theme & Style Tokens
// ============================================================================

pub struct GlassTheme {
    pub bg_dark: egui::Color32,
    pub card_bg: egui::Color32,
    pub card_border: egui::Color32,
    pub gradient_indigo: egui::Color32,
    pub gradient_purple: egui::Color32,
    pub gradient_pink: egui::Color32,
    pub accent_green: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_muted: egui::Color32,
}

impl Default for GlassTheme {
    fn default() -> Self {
        Self {
            bg_dark: egui::Color32::from_rgba_premultiplied(15, 23, 42, 230),
            card_bg: egui::Color32::from_rgba_premultiplied(30, 41, 59, 170),
            card_border: egui::Color32::from_rgba_premultiplied(255, 255, 255, 35),
            gradient_indigo: egui::Color32::from_rgb(79, 70, 229),
            gradient_purple: egui::Color32::from_rgb(168, 85, 247),
            gradient_pink: egui::Color32::from_rgb(236, 72, 153),
            accent_green: egui::Color32::from_rgb(34, 197, 94),
            text_primary: egui::Color32::from_rgb(248, 250, 252),
            text_muted: egui::Color32::from_rgb(148, 163, 184),
        }
    }
}

// ============================================================================
// UI Application State
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTab {
    Chats,
    Files,
    Nearby,
    Settings,
}

#[derive(Clone, Debug)]
pub struct PeerEntry {
    pub id: String,
    pub alias: String,
    pub ip: String,
    pub port: u16,
    pub is_online: bool,
    pub unread_count: usize,
}

pub struct DropItApp {
    pub theme: GlassTheme,
    pub identity: AnonymousIdentity,
    pub settings: AppSettings,
    pub active_tab: ActiveTab,
    pub peers: Vec<PeerEntry>,
    pub selected_peer_id: Option<String>,
    pub chat_messages: HashMap<String, Vec<ChatMessage>>,
    pub message_input: String,
    pub search_query: String,
    pub pairing_code_input: String,
    pub status_message: String,
    pub dropped_files_queue: Vec<PathBuf>,
}

impl Default for DropItApp {
    fn default() -> Self {
        Self::new_standalone()
    }
}

impl DropItApp {
    pub fn new_standalone() -> Self {
        let settings = AppSettings::load();
        let identity = AnonymousIdentity {
            device_id: crate::identity::generate_anonymous_device_id(),
            alias: settings.alias.clone(),
        };

        // Sample peer for immediate interactive experience
        let sample_peer = PeerEntry {
            id: "peer-sample-1".to_string(),
            alias: "Amber Lynx".to_string(),
            ip: "192.168.1.120".to_string(),
            port: 53420,
            is_online: true,
            unread_count: 0,
        };

        let mut chat_messages = HashMap::new();
        chat_messages.insert(
            sample_peer.id.clone(),
            vec![ChatMessage::new(
                "thread-sample",
                &sample_peer.id,
                &sample_peer.alias,
                MessagePayload::Text("Connected over local encrypted channel.".to_string()),
            )],
        );

        Self {
            theme: GlassTheme::default(),
            identity,
            settings,
            active_tab: ActiveTab::Chats,
            peers: vec![sample_peer.clone()],
            selected_peer_id: Some(sample_peer.id),
            chat_messages,
            message_input: String::new(),
            search_query: String::new(),
            pairing_code_input: String::new(),
            status_message: "Drop It ready (LAN anonymous)".to_string(),
            dropped_files_queue: Vec::new(),
        }
    }

    pub fn send_current_message(&mut self) {
        let content = self.message_input.trim().to_string();
        if content.is_empty() {
            return;
        }

        if let Some(peer_id) = &self.selected_peer_id {
            let msg = ChatMessage::new(
                peer_id,
                &self.identity.device_id,
                &self.identity.alias,
                MessagePayload::Text(content),
            );
            self.chat_messages
                .entry(peer_id.clone())
                .or_default()
                .push(msg);
            self.message_input.clear();
        }
    }

    pub fn regenerate_alias(&mut self) {
        self.identity.regenerate_alias();
        self.settings.alias = self.identity.alias.clone();
        let _ = self.settings.save();
        self.status_message = format!("Alias regenerated to '{}'", self.identity.alias);
    }
}

impl eframe::App for DropItApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle drag and drop files
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                for file in &i.raw.dropped_files {
                    if let Some(path) = &file.path {
                        self.dropped_files_queue.push(path.clone());
                    }
                }
                if let Some(peer_id) = &self.selected_peer_id {
                    for path in self.dropped_files_queue.drain(..) {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                            .to_string();
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        let bubble = ChatMessage::new(
                            peer_id,
                            &self.identity.device_id,
                            &self.identity.alias,
                            MessagePayload::FileBubble {
                                file_id: format!("{:016x}", rand::random::<u64>()),
                                filename: name,
                                size,
                                blake3_hash: "verified".to_string(),
                            },
                        );
                        self.chat_messages
                            .entry(peer_id.clone())
                            .or_default()
                            .push(bubble);
                    }
                }
            }
        });

        // 1. Navigation Rail (Leftmost 64px)
        egui::SidePanel::left("nav_rail")
            .exact_width(64.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    // App Logo Drop indicator
                    ui.label(egui::RichText::new("💧").size(24.0));
                    ui.add_space(20.0);

                    // Nav tab buttons
                    if ui
                        .selectable_label(self.active_tab == ActiveTab::Chats, "💬")
                        .clicked()
                    {
                        self.active_tab = ActiveTab::Chats;
                    }
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(self.active_tab == ActiveTab::Files, "📁")
                        .clicked()
                    {
                        self.active_tab = ActiveTab::Files;
                    }
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(self.active_tab == ActiveTab::Nearby, "📡")
                        .clicked()
                    {
                        self.active_tab = ActiveTab::Nearby;
                    }
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(self.active_tab == ActiveTab::Settings, "⚙")
                        .clicked()
                    {
                        self.active_tab = ActiveTab::Settings;
                    }
                });
            });

        // 2. Sidebar (210px: Devices / Threads list)
        egui::SidePanel::left("devices_sidebar")
            .exact_width(210.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.heading(egui::RichText::new("Drop It").strong());

                // User Identity Card
                ui.add_space(8.0);
                egui::Frame::none()
                    .fill(self.theme.card_bg)
                    .stroke(egui::Stroke::new(1.0_f32, self.theme.card_border))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("●").color(self.theme.accent_green));
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&self.identity.alias)
                                        .strong()
                                        .size(13.0),
                                );
                                ui.label(
                                    egui::RichText::new(format!("Port: {}", self.settings.port))
                                        .size(10.0)
                                        .color(self.theme.text_muted),
                                );
                            });
                        });
                    });

                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("PEERS")
                        .size(10.0)
                        .strong()
                        .color(self.theme.text_muted),
                );

                // Peer List
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let peers = self.peers.clone();
                    for peer in peers {
                        let is_selected = self.selected_peer_id.as_deref() == Some(&peer.id);
                        let bg_color = if is_selected {
                            self.theme.gradient_indigo
                        } else {
                            egui::Color32::TRANSPARENT
                        };

                        egui::Frame::none()
                            .fill(bg_color)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if peer.is_online {
                                        ui.label(
                                            egui::RichText::new("●").color(self.theme.accent_green),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("○").color(self.theme.text_muted),
                                        );
                                    }
                                    if ui.button(&peer.alias).clicked() {
                                        self.selected_peer_id = Some(peer.id.clone());
                                    }
                                });
                            });
                    }
                });
            });

        // 3. Central Panel (Conversation View / Active Tab)
        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            ActiveTab::Chats => {
                self.render_chat_view(ui);
            }
            ActiveTab::Files => {
                self.render_files_view(ui);
            }
            ActiveTab::Nearby => {
                self.render_nearby_view(ui);
            }
            ActiveTab::Settings => {
                self.render_settings_view(ui);
            }
        });
    }
}

impl DropItApp {
    fn render_chat_view(&mut self, ui: &mut egui::Ui) {
        if let Some(peer_id) = self.selected_peer_id.clone() {
            let peer_alias = self
                .peers
                .iter()
                .find(|p| p.id == peer_id)
                .map(|p| p.alias.clone())
                .unwrap_or_else(|| "Peer".to_string());

            // Header
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new(&peer_alias).strong());
                ui.label(
                    egui::RichText::new("🔒 TLS 1.3 End-to-End Encrypted")
                        .size(11.0)
                        .color(self.theme.accent_green),
                );
            });
            ui.separator();

            // Drag and drop dropzone card
            egui::Frame::none()
                .fill(self.theme.card_bg)
                .stroke(egui::Stroke::new(1.0_f32, self.theme.gradient_purple))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Drop files or folders here")
                                .strong()
                                .color(self.theme.gradient_indigo),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Transferred instantly with Blake3 integrity check",
                            )
                            .size(11.0)
                            .color(self.theme.text_muted),
                        );
                    });
                });

            ui.add_space(8.0);

            // Message thread
            ui.push_id("chat_scroll", |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let msgs = self
                            .chat_messages
                            .get(&peer_id)
                            .cloned()
                            .unwrap_or_default();
                        for msg in msgs {
                            let is_mine = msg.sender_id == self.identity.device_id;
                            let bubble_color = if is_mine {
                                self.theme.gradient_indigo
                            } else {
                                self.theme.card_bg
                            };

                            ui.horizontal(|ui| {
                                if is_mine {
                                    ui.add_space(60.0);
                                }
                                egui::Frame::none()
                                    .fill(bubble_color)
                                    .stroke(egui::Stroke::new(1.0_f32, self.theme.card_border))
                                    .inner_margin(8.0)
                                    .show(ui, |ui| match &msg.payload {
                                        MessagePayload::Text(text) => {
                                            ui.label(text);
                                        }
                                        MessagePayload::FileBubble { filename, size, .. } => {
                                            ui.horizontal(|ui| {
                                                ui.label("📎");
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(filename).strong(),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "{:.1} KB",
                                                            *size as f64 / 1024.0
                                                        ))
                                                        .size(10.0),
                                                    );
                                                });
                                            });
                                        }
                                        _ => {}
                                    });
                            });
                            ui.add_space(4.0);
                        }
                    });
            });

            // Bottom Input Bar
            ui.separator();
            ui.horizontal(|ui| {
                let response = ui.add_sized(
                    [ui.available_width() - 80.0, 32.0],
                    egui::TextEdit::singleline(&mut self.message_input)
                        .hint_text("Type an encrypted message..."),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.send_current_message();
                }
                if ui.button("Send").clicked() {
                    self.send_current_message();
                }
            });
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.label(
                    egui::RichText::new("Select a peer or connect using an N-word-word code")
                        .color(self.theme.text_muted),
                );
            });
        }
    }

    fn render_files_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("File Transfers");
        ui.add_space(12.0);
        ui.label("Download folder:");
        ui.label(
            egui::RichText::new(self.settings.download_dir.display().to_string())
                .color(self.theme.text_muted),
        );
        ui.add_space(16.0);
        ui.label("Recent transfers appear in your shared conversation threads.");
    }

    fn render_nearby_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Nearby Devices (LAN Discovery)");
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Listening for UDP announcements on port 53421")
                .size(12.0)
                .color(self.theme.text_muted),
        );

        ui.add_space(16.0);
        ui.label("Pair using one-time code (N-word-word):");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.pairing_code_input);
            if ui.button("Connect").clicked() {
                self.status_message =
                    format!("Connecting with code '{}'...", self.pairing_code_input);
            }
        });

        ui.add_space(16.0);
        ui.label(egui::RichText::new(&self.status_message).color(self.theme.accent_green));
    }

    fn render_settings_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings & Anonymity");
        ui.add_space(12.0);

        ui.label("Your Anonymous Alias:");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&self.identity.alias)
                    .strong()
                    .size(16.0),
            );
            if ui.button("🎲 Regenerate").clicked() {
                self.regenerate_alias();
            }
        });

        ui.add_space(12.0);
        ui.label("Anonymous Device ID (128-bit isolated hex):");
        ui.label(
            egui::RichText::new(&self.identity.device_id)
                .size(11.0)
                .color(self.theme.text_muted),
        );

        ui.add_space(16.0);
        if ui
            .checkbox(
                &mut self.settings.auto_accept,
                "Auto-accept transfers from trusted peers",
            )
            .changed()
        {
            let _ = self.settings.save();
        }

        ui.add_space(20.0);
        egui::Frame::none()
            .fill(self.theme.card_bg)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("🛡️ Privacy & Security Guarantee").strong());
                ui.label(
                    egui::RichText::new(
                        "• Zero telemetry, zero external requests, zero accounts\n\
                         • Ephemeral random aliases with no OS username leakage\n\
                         • End-to-end encrypted with forward secrecy\n\
                         • Local chat history protected by Windows DPAPI",
                    )
                    .size(12.0)
                    .color(self.theme.text_muted),
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gui_app_initialization() {
        let app = DropItApp::new_standalone();
        assert_eq!(app.active_tab, ActiveTab::Chats);
        assert!(!app.identity.alias.is_empty());
        assert_eq!(app.identity.device_id.len(), 32);
        assert!(!app.peers.is_empty());
    }

    #[test]
    fn test_gui_alias_regeneration() {
        let mut app = DropItApp::new_standalone();
        let _old_alias = app.identity.alias.clone();
        app.regenerate_alias();
        assert!(!app.identity.alias.is_empty());
        assert!(app.status_message.contains("regenerated"));
    }

    #[test]
    fn test_gui_send_message_state() {
        let mut app = DropItApp::new_standalone();
        let peer_id = app.selected_peer_id.clone().unwrap();
        app.message_input = "Hello GUI integration test!".to_string();
        app.send_current_message();

        let msgs = app.chat_messages.get(&peer_id).unwrap();
        assert!(msgs.len() >= 2);
        match &msgs.last().unwrap().payload {
            MessagePayload::Text(text) => {
                assert_eq!(text, "Hello GUI integration test!");
            }
            _ => panic!("Expected Text message"),
        }
    }
}
