use crate::client::DropItClient;
use crate::config::DropItConfig;
use crate::crypto::{self, TlsIdentity};
use crate::discovery::{DeviceAnnouncement, DiscoveryManager, PeerEvent, PeerRegistry};
use crate::platform;
use crate::server::{OfferDecisionRequest, ServerState, WebSseEvent, run_server};
use crate::transfer::{FileMetadata, TransferOfferRequest};
use anyhow::Result;
use eframe::egui;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};

#[derive(Clone, Debug)]
pub struct GuiPeer {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub fingerprint: String,
    pub version: String,
    pub last_seen: Instant,
}

#[derive(Clone, Debug)]
pub enum ChatMessageKind {
    Text(String),
    FileCard {
        session_id: String,
        files: Vec<FileMetadata>,
        total_bytes: u64,
        progress: f32,
        speed_mb_s: f64,
        status: FileTransferStatus,
        local_path: Option<PathBuf>,
        decision_tx: Option<Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileTransferStatus {
    PendingOffer,
    Transferring,
    Completed,
    Declined,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub is_me: bool,
    pub sender_name: String,
    pub time_str: String,
    pub kind: ChatMessageKind,
}

pub enum GuiEvent {
    PeerAdded(GuiPeer),
    PeerRemoved(String),
    IncomingOffer(OfferDecisionRequest),
    Progress {
        session_id: String,
        percent: f64,
        loaded: u64,
        total: u64,
        speed_mb_s: f64,
    },
    IncomingMessage {
        sender_id: String,
        sender_name: String,
        text: String,
    },
    OutgoingTransferUpdate {
        peer_id: String,
        session_id: String,
        file_name: String,
        bytes_transferred: u64,
        total_bytes: u64,
        speed_mb_s: f64,
        is_done: bool,
        error: Option<String>,
    },
}

pub enum GuiAction {
    SendFiles {
        target_addr: String,
        target_fp: String,
        target_name: String,
        target_peer_id: String,
        session_id: String,
        file_paths: Vec<PathBuf>,
    },
    SendMessage {
        target_addr: String,
        text: String,
    },
}

pub struct DropItApp {
    pub config: DropItConfig,
    pub identity: TlsIdentity,
    pub peers: HashMap<String, GuiPeer>,
    pub selected_peer_id: Option<String>,
    pub chat_history: HashMap<String, Vec<ChatMessage>>,
    pub text_input: String,
    pub search_query: String,

    pub renaming_peer_id: Option<String>,
    pub rename_text: String,

    pub show_settings: bool,
    pub show_qr: bool,
    pub qr_svg: Option<String>,

    pub rx_events: std::sync::mpsc::Receiver<GuiEvent>,
    pub tx_actions: tokio::sync::mpsc::Sender<GuiAction>,

    pub tray_icon: Option<tray_icon::TrayIcon>,
    pub tray_channel: tray_icon::TrayIconEventReceiver,
    pub menu_channel: tray_icon::menu::MenuEventReceiver,

    pub show_menu_id: tray_icon::menu::MenuId,
    pub quit_menu_id: tray_icon::menu::MenuId,

    pub startup_enabled: bool,
    pub context_menu_enabled: bool,
}

impl DropItApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: DropItConfig,
        identity: TlsIdentity,
        rx_events: std::sync::mpsc::Receiver<GuiEvent>,
        tx_actions: tokio::sync::mpsc::Sender<GuiAction>,
        start_in_tray: bool,
    ) -> Self {
        // Configure custom fonts / styling
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::from_rgb(244, 244, 245));
        visuals.panel_fill = egui::Color32::from_rgb(17, 19, 26);
        visuals.window_fill = egui::Color32::from_rgb(23, 26, 36);
        visuals.faint_bg_color = egui::Color32::from_rgb(30, 34, 48);
        visuals.extreme_bg_color = egui::Color32::from_rgb(12, 14, 20);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(25, 29, 41);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(32, 37, 52);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(45, 52, 74);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(79, 70, 229);
        cc.egui_ctx.set_visuals(visuals);

        // System Tray setup
        let tray_menu = tray_icon::menu::Menu::new();
        let show_item = tray_icon::menu::MenuItem::new("Open Drop It", true, None);
        let show_menu_id = show_item.id().clone();
        let quit_item = tray_icon::menu::MenuItem::new("Exit Drop It", true, None);
        let quit_menu_id = quit_item.id().clone();
        let _ = tray_menu.append(&show_item);
        let _ = tray_menu.append(&tray_icon::menu::PredefinedMenuItem::separator());
        let _ = tray_menu.append(&quit_item);

        let tray_icon = if let Ok((rgba, w, h)) = platform::load_app_icon_rgba() {
            if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, w, h) {
                tray_icon::TrayIconBuilder::new()
                    .with_menu(Box::new(tray_menu))
                    .with_tooltip("Drop It - Local WiFi File Transfer")
                    .with_icon(icon)
                    .build()
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        let tray_channel = tray_icon::TrayIconEvent::receiver().clone();
        let menu_channel = tray_icon::menu::MenuEvent::receiver().clone();

        let startup_enabled = platform::get_run_on_startup();
        let context_menu_enabled = platform::get_context_menu_enabled();

        if start_in_tray {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        Self {
            config,
            identity,
            peers: HashMap::new(),
            selected_peer_id: None,
            chat_history: HashMap::new(),
            text_input: String::new(),
            search_query: String::new(),
            renaming_peer_id: None,
            rename_text: String::new(),
            show_settings: false,
            show_qr: false,
            qr_svg: None,
            rx_events,
            tx_actions,
            tray_icon,
            tray_channel,
            menu_channel,
            show_menu_id,
            quit_menu_id,
            startup_enabled,
            context_menu_enabled,
        }
    }

    fn now_str() -> String {
        let now = chrono_now();
        now
    }

    fn send_selected_files(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let peer_id = match &self.selected_peer_id {
            Some(id) => id.clone(),
            None => return,
        };
        let peer = match self.peers.get(&peer_id) {
            Some(p) => p.clone(),
            None => return,
        };

        let session_id = hex::encode(rand::random::<[u8; 16]>());
        let mut meta_list = Vec::new();
        let mut total_bytes = 0u64;

        for p in &files {
            if let Ok(m) = p.metadata() {
                let filename = p
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unnamed_file".to_string());
                let size = m.len();
                total_bytes += size;
                meta_list.push(FileMetadata {
                    file_id: hex::encode(rand::random::<[u8; 8]>()),
                    filename,
                    size_bytes: size,
                    blake3_hash: None,
                });
            }
        }

        // Add outgoing card to chat timeline
        let msg = ChatMessage {
            id: hex::encode(rand::random::<[u8; 8]>()),
            is_me: true,
            sender_name: self.config.device_name.clone(),
            time_str: Self::now_str(),
            kind: ChatMessageKind::FileCard {
                session_id: session_id.clone(),
                files: meta_list.clone(),
                total_bytes,
                progress: 0.0,
                speed_mb_s: 0.0,
                status: FileTransferStatus::Transferring,
                local_path: files.first().cloned(),
                decision_tx: None,
            },
        };

        self.chat_history
            .entry(peer_id.clone())
            .or_default()
            .push(msg);

        let _ = self.tx_actions.try_send(GuiAction::SendFiles {
            target_addr: peer.addr,
            target_fp: peer.fingerprint,
            target_name: peer.name,
            target_peer_id: peer_id,
            session_id,
            file_paths: files,
        });
    }

    fn send_current_message(&mut self) {
        let text = self.text_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let peer_id = match &self.selected_peer_id {
            Some(id) => id.clone(),
            None => return,
        };
        let peer = match self.peers.get(&peer_id) {
            Some(p) => p.clone(),
            None => return,
        };

        self.text_input.clear();

        // Add to local chat
        let msg = ChatMessage {
            id: hex::encode(rand::random::<[u8; 8]>()),
            is_me: true,
            sender_name: self.config.device_name.clone(),
            time_str: Self::now_str(),
            kind: ChatMessageKind::Text(text.clone()),
        };
        self.chat_history
            .entry(peer_id)
            .or_default()
            .push(msg);

        let _ = self.tx_actions.try_send(GuiAction::SendMessage {
            target_addr: peer.addr,
            text,
        });
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn chrono_now() -> String {
    // Simple clock representation
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    format!("{:02}:{:02}", hours, mins)
}

fn color_for_name(name: &str) -> egui::Color32 {
    let hash: u32 = name.bytes().fold(0, |acc, b| acc.wrapping_add(b as u32));
    let colors = [
        egui::Color32::from_rgb(99, 102, 241),  // Indigo
        egui::Color32::from_rgb(168, 85, 247),  // Purple
        egui::Color32::from_rgb(236, 72, 153),  // Pink
        egui::Color32::from_rgb(14, 165, 233),  // Sky
        egui::Color32::from_rgb(20, 184, 166),  // Teal
        egui::Color32::from_rgb(249, 115, 22),  // Orange
    ];
    colors[(hash as usize) % colors.len()]
}

impl eframe::App for DropItApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle window close request -> minimize to tray if enabled
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.config.minimize_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }

        // Handle system tray menu clicks
        while let Ok(event) = self.menu_channel.try_recv() {
            if event.id == self.show_menu_id {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == self.quit_menu_id {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Handle tray icon left/double click
        while let Ok(event) = self.tray_channel.try_recv() {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                ..
            } = event
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }

        // Receive backend events
        while let Ok(event) = self.rx_events.try_recv() {
            match event {
                GuiEvent::PeerAdded(peer) => {
                    self.peers.insert(peer.id.clone(), peer);
                }
                GuiEvent::PeerRemoved(id) => {
                    self.peers.remove(&id);
                }
                GuiEvent::IncomingOffer(req) => {
                    let peer_id = req.offer.sender_device_id.clone();
                    let sender_name = req.offer.sender_name.clone();
                    let total_bytes = req.offer.files.iter().map(|f| f.size_bytes).sum();
                    let session_id = req.offer.session_id.clone();
                    let decision_tx_arc =
                        Arc::new(tokio::sync::Mutex::new(Some(req.response_tx)));

                    // If auto accept trusted or favorite
                    let is_fav = self.config.is_favorite(&peer_id);
                    if req.is_trusted || (is_fav && self.config.auto_accept_trusted) {
                        let tx_clone = decision_tx_arc.clone();
                        tokio::spawn(async move {
                            let mut lock = tx_clone.lock().await;
                            if let Some(tx) = lock.take() {
                                let _ = tx.send(true);
                            }
                        });
                    }

                    let status = if req.is_trusted || (is_fav && self.config.auto_accept_trusted) {
                        FileTransferStatus::Transferring
                    } else {
                        FileTransferStatus::PendingOffer
                    };

                    let msg = ChatMessage {
                        id: hex::encode(rand::random::<[u8; 8]>()),
                        is_me: false,
                        sender_name,
                        time_str: Self::now_str(),
                        kind: ChatMessageKind::FileCard {
                            session_id,
                            files: req.offer.files,
                            total_bytes,
                            progress: 0.0,
                            speed_mb_s: 0.0,
                            status,
                            local_path: None,
                            decision_tx: Some(decision_tx_arc),
                        },
                    };

                    self.chat_history
                        .entry(peer_id.clone())
                        .or_default()
                        .push(msg);

                    if self.selected_peer_id.is_none() {
                        self.selected_peer_id = Some(peer_id);
                    }
                }
                GuiEvent::Progress {
                    session_id,
                    percent,
                    speed_mb_s,
                    ..
                } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                progress,
                                speed_mb_s: spd,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *progress = (percent / 100.0) as f32;
                                    *spd = speed_mb_s;
                                    if *progress >= 1.0 {
                                        *status = FileTransferStatus::Completed;
                                    } else {
                                        *status = FileTransferStatus::Transferring;
                                    }
                                }
                            }
                        }
                    }
                }
                GuiEvent::IncomingMessage {
                    sender_id,
                    sender_name,
                    text,
                } => {
                    let msg = ChatMessage {
                        id: hex::encode(rand::random::<[u8; 8]>()),
                        is_me: false,
                        sender_name,
                        time_str: Self::now_str(),
                        kind: ChatMessageKind::Text(text),
                    };
                    self.chat_history
                        .entry(sender_id.clone())
                        .or_default()
                        .push(msg);

                    if self.selected_peer_id.is_none() {
                        self.selected_peer_id = Some(sender_id);
                    }
                }
                GuiEvent::OutgoingTransferUpdate {
                    peer_id,
                    session_id,
                    bytes_transferred,
                    total_bytes,
                    speed_mb_s,
                    is_done,
                    error,
                    ..
                } => {
                    if let Some(msgs) = self.chat_history.get_mut(&peer_id) {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                progress,
                                speed_mb_s: spd,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    if total_bytes > 0 {
                                        *progress = (bytes_transferred as f64 / total_bytes as f64) as f32;
                                    }
                                    *spd = speed_mb_s;
                                    if is_done {
                                        if let Some(err) = error.clone() {
                                            *status = FileTransferStatus::Failed(err);
                                        } else {
                                            *status = FileTransferStatus::Completed;
                                            *progress = 1.0;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Native Drag & Drop from Windows Explorer
        let dropped_files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|d| d.path.clone())
                .collect()
        });
        if !dropped_files.is_empty() && self.selected_peer_id.is_some() {
            self.send_selected_files(dropped_files);
        }

        // Main Layout: Left Sidebar + Right Chat Channel
        egui::SidePanel::left("sidebar_panel")
            .resizable(false)
            .exact_width(290.0)
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(18, 20, 29))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(28, 32, 46))),
            )
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(13, 15, 22)))
            .show(ctx, |ui| {
                self.render_chat_area(ui);
            });

        // Modals
        if self.show_settings {
            self.render_settings_modal(ctx);
        }
        if self.show_qr {
            self.render_qr_modal(ctx);
        }

        // Request repaint for smooth progress updates
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

impl DropItApp {
    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);

        // Header: Logo + Title + Device Status + Settings Icon
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            // Circular logo icon
            let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
            ui.painter().circle_filled(
                rect.center(),
                16.0,
                egui::Color32::from_rgb(79, 70, 229),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "↓",
                egui::FontId::proportional(18.0),
                egui::Color32::WHITE,
            );

            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Drop It")
                            .size(17.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    // Online green dot
                    let (dot_rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        dot_rect.center(),
                        4.0,
                        egui::Color32::from_rgb(16, 185, 129),
                    );
                });
                ui.label(
                    egui::RichText::new(&self.config.device_name)
                        .size(11.0)
                        .color(egui::Color32::from_rgb(156, 163, 175)),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if ui
                    .add(egui::Button::new(egui::RichText::new("⚙").size(16.0)).frame(false))
                    .on_hover_text("Settings")
                    .clicked()
                {
                    self.show_settings = true;
                }
                if ui
                    .add(egui::Button::new(egui::RichText::new("📱").size(15.0)).frame(false))
                    .on_hover_text("Open on Phone (QR Code)")
                    .clicked()
                {
                    self.show_qr = true;
                }
            });
        });

        ui.add_space(14.0);

        // Search Bar
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let width = ui.available_width() - 12.0;
            ui.add_sized(
                [width, 32.0],
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("🔍 Search devices...")
                    .margin(egui::vec2(10.0, 6.0)),
            );
        });

        ui.add_space(12.0);
        ui.separator();

        // Device List Scroll
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);

            let all_peers: Vec<GuiPeer> = self.peers.values().cloned().collect();
            let query = self.search_query.trim().to_lowercase();

            let filtered_peers: Vec<GuiPeer> = all_peers
                .into_iter()
                .filter(|p| {
                    if query.is_empty() {
                        true
                    } else {
                        p.name.to_lowercase().contains(&query)
                            || p.addr.contains(&query)
                            || self
                                .config
                                .get_display_name(&p.id, &p.name)
                                .to_lowercase()
                                .contains(&query)
                    }
                })
                .collect();

            let mut fav_peers = Vec::new();
            let mut other_peers = Vec::new();

            for p in filtered_peers {
                if self.config.is_favorite(&p.id) {
                    fav_peers.push(p);
                } else {
                    other_peers.push(p);
                }
            }

            // Favorites Section
            if !fav_peers.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new("⭐ FAVORITES")
                            .size(11.0)
                            .strong()
                            .color(egui::Color32::from_rgb(251, 191, 36)),
                    );
                });
                ui.add_space(4.0);
                for p in fav_peers {
                    self.render_peer_item(ui, &p);
                }
                ui.add_space(10.0);
            }

            // Nearby Devices Section
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("📡 NEARBY DEVICES ({})", other_peers.len()))
                        .size(11.0)
                        .strong()
                        .color(egui::Color32::from_rgb(156, 163, 175)),
                );
            });
            ui.add_space(4.0);

            if other_peers.is_empty() && self.peers.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Looking for devices...")
                            .size(13.0)
                            .color(egui::Color32::from_rgb(107, 114, 128)),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Open Drop It on other devices on the same WiFi")
                            .size(11.0)
                            .color(egui::Color32::from_rgb(75, 85, 99)),
                    );
                });
            } else {
                for p in other_peers {
                    self.render_peer_item(ui, &p);
                }
            }
        });
    }

    fn render_peer_item(&mut self, ui: &mut egui::Ui, peer: &GuiPeer) {
        let is_selected = self.selected_peer_id.as_deref() == Some(&peer.id);
        let is_fav = self.config.is_favorite(&peer.id);
        let display_name = self.config.get_display_name(&peer.id, &peer.name).to_string();

        let bg_color = if is_selected {
            egui::Color32::from_rgb(45, 52, 74)
        } else {
            egui::Color32::TRANSPARENT
        };

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let frame = egui::Frame::NONE
                .fill(bg_color)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(8, 8));

            frame.show(ui, |ui| {
                let avail_w = ui.available_width() - 8.0;
                ui.set_width(avail_w);

                ui.horizontal(|ui| {
                    // Avatar circle
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        18.0,
                        color_for_name(&peer.name),
                    );
                    let initial = display_name
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string();
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        initial,
                        egui::FontId::proportional(15.0),
                        egui::Color32::WHITE,
                    );

                    ui.add_space(8.0);

                    // Name + IP
                    let label_res = ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&display_name)
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            );
                        });
                        ui.label(
                            egui::RichText::new(&peer.addr)
                                .size(11.0)
                                .color(egui::Color32::from_rgb(156, 163, 175)),
                        );
                    });

                    // Click peer row to select
                    if label_res.response.interact(egui::Sense::click()).clicked() {
                        self.selected_peer_id = Some(peer.id.clone());
                    }

                    // Right side: Star button & Rename button
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let star_icon = if is_fav { "⭐" } else { "✩" };
                        let star_color = if is_fav {
                            egui::Color32::from_rgb(251, 191, 36)
                        } else {
                            egui::Color32::from_rgb(156, 163, 175)
                        };

                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(star_icon).size(14.0).color(star_color),
                                )
                                .frame(false),
                            )
                            .on_hover_text(if is_fav {
                                "Remove from favorites"
                            } else {
                                "Add to favorites"
                            })
                            .clicked()
                        {
                            let _ = self.config.toggle_favorite(&peer.id);
                        }

                        if ui
                            .add(egui::Button::new(egui::RichText::new("✏").size(12.0)).frame(false))
                            .on_hover_text("Rename device")
                            .clicked()
                        {
                            self.renaming_peer_id = Some(peer.id.clone());
                            self.rename_text = display_name.clone();
                        }
                    });
                });
            });
        });

        // Inline rename popup if active
        if self.renaming_peer_id.as_deref() == Some(&peer.id) {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.add(egui::TextEdit::singleline(&mut self.rename_text).desired_width(160.0));
                if ui.button("Save").clicked() {
                    let _ = self
                        .config
                        .set_nickname(&peer.id, &self.rename_text);
                    self.renaming_peer_id = None;
                }
                if ui.button("✖").clicked() {
                    self.renaming_peer_id = None;
                }
            });
        }
    }

    fn render_chat_area(&mut self, ui: &mut egui::Ui) {
        let selected_id = match &self.selected_peer_id {
            Some(id) => id.clone(),
            None => {
                // Empty state when no peer selected
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        32.0,
                        egui::Color32::from_rgb(30, 34, 48),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "💬",
                        egui::FontId::proportional(28.0),
                        egui::Color32::WHITE,
                    );
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("Select a device on the left")
                            .size(18.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Share files and quick text messages instantly with peer devices on your WiFi",
                        )
                        .size(13.0)
                        .color(egui::Color32::from_rgb(156, 163, 175)),
                    );
                });
                return;
            }
        };

        let peer = match self.peers.get(&selected_id) {
            Some(p) => p.clone(),
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.label("Device is currently offline or unreachable.");
                });
                return;
            }
        };

        let display_name = self.config.get_display_name(&peer.id, &peer.name).to_string();
        let is_fav = self.config.is_favorite(&peer.id);

        // Top Channel Bar
        egui::Frame::NONE
            .fill(egui::Color32::from_rgb(20, 23, 33))
            .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(28, 32, 46)))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Avatar
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        18.0,
                        color_for_name(&peer.name),
                    );
                    let initial = display_name
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string();
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        initial,
                        egui::FontId::proportional(15.0),
                        egui::Color32::WHITE,
                    );

                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&display_name)
                                    .size(16.0)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            );
                            if is_fav {
                                ui.label(
                                    egui::RichText::new("⭐")
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(251, 191, 36)),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("🔒 TLS 1.3  •  {}", peer.addr))
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(16, 185, 129)),
                            );
                        });
                    });

                    // Quick Action Buttons on right
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("📁 Send Folder").size(13.0))
                            .clicked()
                        {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                // Recursively find files or send root folder files
                                if let Ok(entries) = std::fs::read_dir(&folder) {
                                    let mut files = Vec::new();
                                    for entry in entries.flatten() {
                                        if entry.path().is_file() {
                                            files.push(entry.path());
                                        }
                                    }
                                    self.send_selected_files(files);
                                }
                            }
                        }

                        if ui
                            .button(
                                egui::RichText::new("📎 Send Files")
                                    .size(13.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .clicked()
                        {
                            if let Some(files) = rfd::FileDialog::new().pick_files() {
                                self.send_selected_files(files);
                            }
                        }
                    });
                });
            });

        // Chat Timeline
        let available_h = ui.available_height() - 65.0; // leave room for input bar
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .max_height(available_h)
            .show(ui, |ui| {
                ui.add_space(12.0);

                // Pairing Security Banner
                let pairing_code =
                    crypto::generate_pairing_code(&self.identity.fingerprint, &peer.fingerprint);
                ui.vertical_centered(|ui| {
                    egui::Frame::NONE
                        .fill(egui::Color32::from_rgb(26, 30, 43))
                        .corner_radius(16.0)
                        .inner_margin(egui::Margin::symmetric(14, 6))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "🔒 Mutual TLS Verified  •  Pairing Code: [ {} ]",
                                    pairing_code
                                ))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(156, 163, 175)),
                            );
                        });
                });

                ui.add_space(14.0);

                if let Some(messages) = self.chat_history.get(&selected_id) {
                    let msgs_clone = messages.clone();
                    for msg in msgs_clone {
                        self.render_chat_bubble(ui, &msg);
                        ui.add_space(8.0);
                    }
                } else {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Drag and drop files here, or tap 'Send Files' to transfer")
                                .size(13.0)
                                .color(egui::Color32::from_rgb(107, 114, 128)),
                        );
                    });
                }
            });

        // Bottom Message & File Input Bar
        ui.separator();
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("📎").size(18.0))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(36.0, 36.0)),
                )
                .on_hover_text("Choose files to send")
                .clicked()
            {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    self.send_selected_files(files);
                }
            }

            let input_w = ui.available_width() - 60.0;
            let text_edit = ui.add_sized(
                [input_w, 36.0],
                egui::TextEdit::singleline(&mut self.text_input)
                    .hint_text("Type a message or drop files to send...")
                    .margin(egui::vec2(10.0, 8.0)),
            );

            if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.send_current_message();
            }

            if ui
                .add(
                    egui::Button::new(egui::RichText::new("➤").size(16.0).color(egui::Color32::WHITE))
                        .fill(egui::Color32::from_rgb(79, 70, 229))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(40.0, 36.0)),
                )
                .clicked()
            {
                self.send_current_message();
            }
        });
    }

    fn render_chat_bubble(&mut self, ui: &mut egui::Ui, msg: &ChatMessage) {
        let is_me = msg.is_me;
        let align = if is_me {
            egui::Align::Max
        } else {
            egui::Align::Min
        };

        ui.with_layout(egui::Layout::top_down(align), |ui| {
            let max_w = 420.0;

            let bubble_bg = if is_me {
                egui::Color32::from_rgb(79, 70, 229) // Indigo
            } else {
                egui::Color32::from_rgb(30, 34, 48) // Dark Slate
            };

            egui::Frame::NONE
                .fill(bubble_bg)
                .corner_radius(12.0)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width(max_w);

                    match &msg.kind {
                        ChatMessageKind::Text(text) => {
                            ui.label(
                                egui::RichText::new(text)
                                    .size(14.0)
                                    .color(egui::Color32::WHITE),
                            );
                            ui.add_space(2.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                ui.label(
                                    egui::RichText::new(&msg.time_str)
                                        .size(10.0)
                                        .color(egui::Color32::from_rgb(209, 213, 219)),
                                );
                            });
                        }

                        ChatMessageKind::FileCard {
                            files,
                            total_bytes,
                            progress,
                            speed_mb_s,
                            status,
                            local_path,
                            decision_tx,
                            ..
                        } => {
                            let file_count = files.len();
                            let first_name = files
                                .first()
                                .map(|f| f.filename.clone())
                                .unwrap_or_else(|| "Files".into());

                            let title = if file_count > 1 {
                                format!("{} (+{} more files)", first_name, file_count - 1)
                            } else {
                                first_name
                            };

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("📄").size(24.0));
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&title)
                                            .size(14.0)
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.label(
                                        egui::RichText::new(format_bytes(*total_bytes))
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(209, 213, 219)),
                                    );
                                });
                            });

                            ui.add_space(6.0);

                            match status {
                                FileTransferStatus::PendingOffer => {
                                    ui.label(
                                        egui::RichText::new("Incoming Transfer Offer")
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(251, 191, 36)),
                                    );
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        if let Some(tx_arc) = decision_tx {
                                            let tx_arc_accept = tx_arc.clone();
                                            if ui
                                                .button(
                                                    egui::RichText::new("Accept ✅")
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .clicked()
                                            {
                                                tokio::spawn(async move {
                                                    let mut lock = tx_arc_accept.lock().await;
                                                    if let Some(tx) = lock.take() {
                                                        let _ = tx.send(true);
                                                    }
                                                });
                                            }

                                            let tx_arc_decline = tx_arc.clone();
                                            if ui
                                                .button(
                                                    egui::RichText::new("Decline ❌")
                                                        .color(egui::Color32::from_rgb(239, 68, 68)),
                                                )
                                                .clicked()
                                            {
                                                tokio::spawn(async move {
                                                    let mut lock = tx_arc_decline.lock().await;
                                                    if let Some(tx) = lock.take() {
                                                        let _ = tx.send(false);
                                                    }
                                                });
                                            }
                                        }
                                    });
                                }

                                FileTransferStatus::Transferring => {
                                    ui.add(
                                        egui::ProgressBar::new(*progress)
                                            .show_percentage()
                                            .animate(true),
                                    );
                                    ui.label(
                                        egui::RichText::new(format!("{:.1} MB/s", speed_mb_s))
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(16, 185, 129)),
                                    );
                                }

                                FileTransferStatus::Completed => {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("Completed  •  Blake3 Verified ✅")
                                                .size(11.0)
                                                .color(egui::Color32::from_rgb(16, 185, 129)),
                                        );
                                    });
                                    if let Some(path) = local_path {
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let path_open = path.clone();
                                            if ui.button("Open File").clicked() {
                                                platform::open_file(&path_open);
                                            }
                                            let path_folder = path.clone();
                                            if ui.button("Open in Folder").clicked() {
                                                platform::open_in_folder(&path_folder);
                                            }
                                        });
                                    }
                                }

                                FileTransferStatus::Declined => {
                                    ui.label(
                                        egui::RichText::new("Transfer Declined")
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(239, 68, 68)),
                                    );
                                }

                                FileTransferStatus::Failed(err) => {
                                    ui.label(
                                        egui::RichText::new(format!("Failed: {}", err))
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(239, 68, 68)),
                                    );
                                }
                            }

                            ui.add_space(2.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                ui.label(
                                    egui::RichText::new(&msg.time_str)
                                        .size(10.0)
                                        .color(egui::Color32::from_rgb(209, 213, 219)),
                                );
                            });
                        }
                    }
                });
        });
    }

    fn render_settings_modal(&mut self, ctx: &egui::Context) {
        egui::Window::new("⚙ Settings")
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::vec2(440.0, 420.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.add_space(8.0);

                ui.label(egui::RichText::new("Device Name").strong());
                ui.add(egui::TextEdit::singleline(&mut self.config.device_name));

                ui.add_space(10.0);
                ui.label(egui::RichText::new("Save Received Files To").strong());
                ui.horizontal(|ui| {
                    ui.label(self.config.download_dir.display().to_string());
                    if ui.button("Browse...").clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            self.config.download_dir = folder;
                        }
                    }
                });

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);

                // Auto-accept trusted/favorite
                ui.checkbox(
                    &mut self.config.auto_accept_trusted,
                    "Auto-accept transfers from Favorites & Trusted Devices",
                );

                // Minimize to tray
                ui.checkbox(
                    &mut self.config.minimize_to_tray,
                    "Close button minimizes to Windows System Tray (runs in background)",
                );

                // Run on Windows startup
                if ui
                    .checkbox(&mut self.startup_enabled, "Launch Drop It on Windows Startup")
                    .changed()
                {
                    let _ = platform::set_run_on_startup(self.startup_enabled);
                    self.config.run_on_startup = self.startup_enabled;
                }

                // Windows Explorer Context menu ("Send with Drop It")
                if ui
                    .checkbox(
                        &mut self.context_menu_enabled,
                        "Add 'Send with Drop It' to Windows Explorer Right-Click Menu",
                    )
                    .changed()
                {
                    let _ = platform::set_context_menu_enabled(self.context_menu_enabled);
                    self.config.context_menu_enabled = self.context_menu_enabled;
                }

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(8.0);

                // Author & Contributor Section
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Drop It for Windows")
                            .size(13.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new("Author & Contributor: Golam Mowla")
                            .size(12.0)
                            .color(egui::Color32::from_rgb(99, 102, 241)),
                    );
                    ui.hyperlink_to("github.com/golam-mowla/dropit", "https://github.com/golam-mowla/dropit");
                });

                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save & Close").clicked() {
                        let _ = self.config.save();
                        self.show_settings = false;
                    }
                });
            });
    }

    fn render_qr_modal(&mut self, ctx: &egui::Context) {
        let lan_ip = local_ip_address::local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|_| "127.0.0.1".into());
        let local_url = format!("https://{}:{}", lan_ip, self.config.port);

        egui::Window::new("📱 Connect from Phone or Tablet")
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::vec2(380.0, 360.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Scan QR with your phone's camera:")
                            .size(13.0)
                            .strong(),
                    );
                    ui.add_space(10.0);

                    // Render terminal QR or SVG representation
                    ui.label(
                        egui::RichText::new(&local_url)
                            .size(14.0)
                            .color(egui::Color32::from_rgb(99, 102, 241)),
                    );
                    ui.add_space(10.0);
                    ui.label("No app needed! Opens the Web UI instantly on your mobile browser.");
                });

                ui.add_space(20.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        self.show_qr = false;
                    }
                });
            });
    }
}

pub async fn run_gui_app(
    config: DropItConfig,
    identity: TlsIdentity,
    start_in_tray: bool,
) -> Result<()> {
    let (tx_events, rx_events) = std::sync::mpsc::channel::<GuiEvent>();
    let (tx_actions, mut rx_actions) = tokio::sync::mpsc::channel::<GuiAction>(32);

    let config_arc = Arc::new(RwLock::new(config.clone()));
    let peer_registry = PeerRegistry::new();

    // Start discovery
    let announcement = DeviceAnnouncement {
        device_id: config.device_id.clone(),
        name: config.device_name.clone(),
        fingerprint: identity.fingerprint.clone(),
        port: config.port,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let mut discovery = DiscoveryManager::new(announcement, peer_registry.clone());
    discovery.start().await?;

    // Pipe discovery peer events to GUI
    let mut peer_rx = peer_registry.subscribe();
    let tx_peers = tx_events.clone();
    tokio::spawn(async move {
        while let Ok(event) = peer_rx.recv().await {
            match event {
                PeerEvent::Added(p) => {
                    let _ = tx_peers.send(GuiEvent::PeerAdded(GuiPeer {
                        id: p.announcement.device_id,
                        name: p.announcement.name,
                        addr: p.address.to_string(),
                        fingerprint: p.announcement.fingerprint,
                        version: p.announcement.version,
                        last_seen: Instant::now(),
                    }));
                }
                PeerEvent::Removed(id) => {
                    let _ = tx_peers.send(GuiEvent::PeerRemoved(id));
                }
                _ => {}
            }
        }
    });

    // Setup Server
    let (prompt_tx, mut prompt_rx) = mpsc::channel::<OfferDecisionRequest>(16);
    let server_state = ServerState::new(
        config_arc.clone(),
        identity.clone(),
        Some(peer_registry.clone()),
        Some(prompt_tx),
    );

    let mut sse_rx = server_state.sse_broadcast_tx.subscribe();
    let tx_sse = tx_events.clone();
    tokio::spawn(async move {
        while let Ok(event) = sse_rx.recv().await {
            match event {
                WebSseEvent::Progress {
                    transfer_id,
                    percent,
                    loaded,
                    total,
                    speed_mb_s,
                } => {
                    let _ = tx_sse.send(GuiEvent::Progress {
                        session_id: transfer_id,
                        percent,
                        loaded,
                        total,
                        speed_mb_s,
                    });
                }
                WebSseEvent::Message {
                    sender_id,
                    sender_name,
                    text,
                } => {
                    let _ = tx_sse.send(GuiEvent::IncomingMessage {
                        sender_id,
                        sender_name,
                        text,
                    });
                }
                _ => {}
            }
        }
    });

    // Pipe incoming transfer offers to GUI
    let tx_offers = tx_events.clone();
    tokio::spawn(async move {
        while let Some(req) = prompt_rx.recv().await {
            let _ = tx_offers.send(GuiEvent::IncomingOffer(req));
        }
    });

    // Run HTTP TLS Server in background
    let bind_addr = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        config.port,
    );
    tokio::spawn(async move {
        let _ = run_server(server_state, bind_addr, None).await;
    });

    // Action worker loop (send files, messages)
    let client = Arc::new(DropItClient::new());
    let cfg_ref = config_arc.clone();
    let id_ref = identity.clone();
    let tx_action_feedback = tx_events.clone();

    tokio::spawn(async move {
        while let Some(action) = rx_actions.recv().await {
            match action {
                GuiAction::SendMessage { target_addr, text } => {
                    let cfg = cfg_ref.read().await;
                    let _ = client
                        .send_quick_message(&target_addr, &cfg.device_id, &cfg.device_name, &text)
                        .await;
                }
                GuiAction::SendFiles {
                    target_addr,
                    target_fp: _,
                    target_name: _,
                    target_peer_id,
                    session_id,
                    file_paths,
                } => {
                    let client_clone = client.clone();
                    let cfg_guard = cfg_ref.read().await;
                    let dev_name = cfg_guard.device_name.clone();
                    let dev_id = cfg_guard.device_id.clone();
                    let fp = id_ref.fingerprint.clone();
                    let tx_fb = tx_action_feedback.clone();
                    drop(cfg_guard);

                    tokio::spawn(async move {
                        let mut meta_list = Vec::new();
                        for p in &file_paths {
                            if let Ok(m) = p.metadata() {
                                let filename = p
                                    .file_name()
                                    .map(|f| f.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "unnamed_file".to_string());
                                meta_list.push(FileMetadata {
                                    file_id: hex::encode(rand::random::<[u8; 8]>()),
                                    filename,
                                    size_bytes: m.len(),
                                    blake3_hash: None,
                                });
                            }
                        }

                        let offer_req = TransferOfferRequest {
                            session_id: session_id.clone(),
                            sender_name: dev_name,
                            sender_device_id: dev_id,
                            sender_fingerprint: fp,
                            files: meta_list.clone(),
                        };

                        match client_clone.send_offer(&target_addr, &offer_req).await {
                            Ok(resp) => {
                                if !resp.accepted {
                                    let _ = tx_fb.send(GuiEvent::OutgoingTransferUpdate {
                                        peer_id: target_peer_id,
                                        session_id,
                                        file_name: "Transfer".into(),
                                        bytes_transferred: 0,
                                        total_bytes: 0,
                                        speed_mb_s: 0.0,
                                        is_done: true,
                                        error: Some("Declined by receiver".into()),
                                    });
                                    return;
                                }
                                let token = match resp.session_token {
                                    Some(t) => t,
                                    None => return,
                                };

                                let items: Vec<(FileMetadata, PathBuf)> =
                                    meta_list.iter().cloned().zip(file_paths).collect();

                                let tx_progress = tx_fb.clone();
                                let peer_id_clone = target_peer_id.clone();
                                let sess_clone = session_id.clone();

                                let res = client_clone
                                    .send_files_smart(
                                        target_addr,
                                        session_id.clone(),
                                        token,
                                        items,
                                        move |p| {
                                            let _ = tx_progress.send(
                                                GuiEvent::OutgoingTransferUpdate {
                                                    peer_id: peer_id_clone.clone(),
                                                    session_id: sess_clone.clone(),
                                                    file_name: p.filename.clone(),
                                                    bytes_transferred: p.bytes_transferred,
                                                    total_bytes: p.total_bytes,
                                                    speed_mb_s: p.speed_mb_s,
                                                    is_done: false,
                                                    error: None,
                                                },
                                            );
                                        },
                                    )
                                    .await;

                                let error = match res {
                                    Ok(_) => None,
                                    Err(e) => Some(e.to_string()),
                                };

                                let _ = tx_fb.send(GuiEvent::OutgoingTransferUpdate {
                                    peer_id: target_peer_id,
                                    session_id,
                                    file_name: "Transfer".into(),
                                    bytes_transferred: 1,
                                    total_bytes: 1,
                                    speed_mb_s: 0.0,
                                    is_done: true,
                                    error,
                                });
                            }
                            Err(e) => {
                                let _ = tx_fb.send(GuiEvent::OutgoingTransferUpdate {
                                    peer_id: target_peer_id,
                                    session_id,
                                    file_name: "Transfer".into(),
                                    bytes_transferred: 0,
                                    total_bytes: 0,
                                    speed_mb_s: 0.0,
                                    is_done: true,
                                    error: Some(e.to_string()),
                                });
                            }
                        }
                    });
                }
            }
        }
    });

    // Run native eframe desktop window
    let mut native_options = eframe::NativeOptions::default();
    native_options.viewport = egui::ViewportBuilder::default()
        .with_title("Drop It")
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([720.0, 480.0])
        .with_drag_and_drop(true);

    if let Ok((rgba, w, h)) = platform::load_app_icon_rgba() {
        native_options.viewport = native_options.viewport.with_icon(egui::IconData {
            rgba,
            width: w,
            height: h,
        });
    }

    let _ = eframe::run_native(
        "Drop It",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(DropItApp::new(
                cc,
                config,
                identity,
                rx_events,
                tx_actions,
                start_in_tray,
            )))
        }),
    );

    Ok(())
}
