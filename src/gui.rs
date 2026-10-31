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
    pub port: u16,
    pub fingerprint: String,
    pub version: String,
    pub last_seen: Instant,
    pub is_online: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuiRailTab {
    Transfers,
    History,
    Favorites,
    Nearby,
    Settings,
}

#[derive(Clone, Debug)]
pub struct HistoryRecord {
    pub id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub is_outgoing: bool,
    pub peer_name: String,
    pub time_str: String,
    pub status: String,
    pub local_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum BubbleAction {
    Decision(String, bool),
    Control(String, crate::transfer::TransferControlCommand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileTransferStatus {
    WaitingForAccept,
    PendingOffer,
    Transferring,
    Paused(String),
    Completed,
    Declined,
    Cancelled,
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
    FileCompleted {
        session_id: String,
        file_id: String,
        dest_path: PathBuf,
    },
    TransferPaused {
        session_id: String,
    },
    TransferResumed {
        session_id: String,
    },
    TransferCancelled {
        session_id: String,
    },
    OutgoingTransferStarted {
        session_id: String,
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
        files: Vec<FileMetadata>,
        file_paths: Vec<PathBuf>,
    },
    SendMessage {
        target_addr: String,
        text: String,
    },
    TransferControl {
        target_addr: String,
        session_id: String,
        command: crate::transfer::TransferControlCommand,
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
    pub active_rail_tab: GuiRailTab,
    pub transfer_history: Vec<HistoryRecord>,
}

fn collect_paths_recursive(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(p) {
                let sub_paths: Vec<PathBuf> =
                    entries.filter_map(|e| e.ok().map(|x| x.path())).collect();
                files.extend(collect_paths_recursive(&sub_paths));
            }
        } else if p.is_file() {
            files.push(p.clone());
        }
    }
    files
}

#[derive(Clone, Copy)]
pub struct AppTheme {
    pub page_bg: egui::Color32,
    pub surface_1: egui::Color32,
    pub surface_2: egui::Color32,
    pub surface_3: egui::Color32,
    pub rail_bg: egui::Color32,
    pub border: egui::Color32,
    pub border_strong: egui::Color32,
    pub border_accent: egui::Color32,
    pub bg_accent: egui::Color32,
    pub bg_accent_muted: egui::Color32,
    pub fill_accent: egui::Color32,
    pub on_accent: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub text_muted: egui::Color32,
    pub text_accent: egui::Color32,
    pub text_success: egui::Color32,
    pub text_warning: egui::Color32,
    pub bg_warning: egui::Color32,
    pub bg_pro: egui::Color32,
    pub text_pro: egui::Color32,
}

impl AppTheme {
    pub fn new(dark: bool) -> Self {
        if dark {
            // Dark mode values from dropit_redesign_preview.html
            Self {
                page_bg: egui::Color32::from_rgb(11, 11, 11),
                surface_1: egui::Color32::from_rgb(21, 21, 21),
                surface_2: egui::Color32::from_rgb(28, 28, 27),
                surface_3: egui::Color32::from_rgb(32, 32, 31),
                rail_bg: egui::Color32::from_rgb(19, 19, 19),
                border: egui::Color32::from_rgba_premultiplied(255, 255, 255, 26),
                border_strong: egui::Color32::from_rgba_premultiplied(255, 255, 255, 46),
                border_accent: egui::Color32::from_rgb(13, 54, 107),
                bg_accent: egui::Color32::from_rgb(6, 43, 87),
                bg_accent_muted: egui::Color32::from_rgba_premultiplied(42, 120, 214, 38),
                fill_accent: egui::Color32::from_rgb(37, 99, 235), // Primary blue accent
                on_accent: egui::Color32::WHITE,
                text_primary: egui::Color32::from_rgb(240, 239, 236),
                text_secondary: egui::Color32::from_rgb(195, 194, 183),
                text_muted: egui::Color32::from_rgb(123, 121, 116),
                text_accent: egui::Color32::from_rgb(109, 167, 236),
                text_success: egui::Color32::from_rgb(50, 215, 75),
                text_warning: egui::Color32::from_rgb(250, 178, 25),
                bg_warning: egui::Color32::from_rgb(49, 26, 0),
                bg_pro: egui::Color32::from_rgb(29, 22, 73),
                text_pro: egui::Color32::from_rgb(191, 185, 245),
            }
        } else {
            // Light mode values from dropit_redesign_preview.html
            Self {
                page_bg: egui::Color32::from_rgb(249, 249, 247),
                surface_1: egui::Color32::from_rgb(252, 252, 251),
                surface_2: egui::Color32::from_rgb(255, 255, 255),
                surface_3: egui::Color32::from_rgb(255, 255, 255),
                rail_bg: egui::Color32::from_rgb(240, 239, 236),
                border: egui::Color32::from_rgba_premultiplied(0, 0, 0, 26),
                border_strong: egui::Color32::from_rgba_premultiplied(0, 0, 0, 51),
                border_accent: egui::Color32::from_rgb(134, 182, 239),
                bg_accent: egui::Color32::from_rgb(235, 244, 252),
                bg_accent_muted: egui::Color32::from_rgba_premultiplied(42, 120, 214, 20),
                fill_accent: egui::Color32::from_rgb(37, 99, 235), // Primary blue accent
                on_accent: egui::Color32::WHITE,
                text_primary: egui::Color32::from_rgb(11, 11, 11),
                text_secondary: egui::Color32::from_rgb(82, 81, 78),
                text_muted: egui::Color32::from_rgb(137, 135, 129),
                text_accent: egui::Color32::from_rgb(28, 92, 171),
                text_success: egui::Color32::from_rgb(30, 158, 60),
                text_warning: egui::Color32::from_rgb(183, 119, 0),
                bg_warning: egui::Color32::from_rgb(249, 238, 218),
                bg_pro: egui::Color32::from_rgb(239, 237, 255),
                text_pro: egui::Color32::from_rgb(98, 80, 214),
            }
        }
    }
}

fn paint_dashed_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, stroke: egui::Stroke) {
    let dash_len = 6.0;
    let gap_len = 4.0;
    let seg = dash_len + gap_len;
    
    // Top
    let mut x = rect.left() + radius;
    while x + dash_len <= rect.right() - radius {
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x + dash_len, rect.top())], stroke);
        x += seg;
    }
    // Bottom
    let mut x = rect.left() + radius;
    while x + dash_len <= rect.right() - radius {
        painter.line_segment([egui::pos2(x, rect.bottom()), egui::pos2(x + dash_len, rect.bottom())], stroke);
        x += seg;
    }
    // Left
    let mut y = rect.top() + radius;
    while y + dash_len <= rect.bottom() - radius {
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.left(), y + dash_len)], stroke);
        y += seg;
    }
    // Right
    let mut y = rect.top() + radius;
    while y + dash_len <= rect.bottom() - radius {
        painter.line_segment([egui::pos2(rect.right(), y), egui::pos2(rect.right(), y + dash_len)], stroke);
        y += seg;
    }
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
        // Configure custom fonts / modern styling from dropit_redesign_preview.html
        let is_dark = config.dark_theme;
        let theme = AppTheme::new(is_dark);
        let mut visuals = if is_dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        visuals.override_text_color = Some(theme.text_primary);
        visuals.panel_fill = theme.surface_2;
        visuals.window_fill = theme.page_bg;
        visuals.faint_bg_color = theme.surface_1;
        visuals.extreme_bg_color = theme.page_bg;
        visuals.widgets.noninteractive.bg_fill = theme.surface_2;
        visuals.widgets.noninteractive.corner_radius = 8.0.into();
        visuals.widgets.inactive.bg_fill = theme.surface_2;
        visuals.widgets.inactive.corner_radius = 8.0.into();
        visuals.widgets.hovered.bg_fill = theme.surface_1;
        visuals.widgets.hovered.corner_radius = 8.0.into();
        visuals.widgets.active.bg_fill = theme.fill_accent;
        visuals.widgets.active.corner_radius = 8.0.into();
        cc.egui_ctx.set_visuals(visuals);

        // Pre-populate discovered peers from persistent history
        let mut peers = HashMap::new();
        for saved in &config.recent_devices {
            peers.insert(
                saved.device_id.clone(),
                GuiPeer {
                    id: saved.device_id.clone(),
                    name: saved.name.clone(),
                    addr: format!("{}:{}", saved.address, saved.port),
                    port: saved.port,
                    fingerprint: saved.fingerprint.clone(),
                    version: "0.1.0".to_string(),
                    last_seen: Instant::now() - Duration::from_secs(3600),
                    is_online: false,
                },
            );
        }

        let selected_peer_id = if !peers.is_empty() {
            config
                .favorites
                .first()
                .cloned()
                .or_else(|| peers.keys().next().cloned())
        } else {
            None
        };

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
            peers,
            selected_peer_id,
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
            active_rail_tab: GuiRailTab::Transfers,
            transfer_history: Vec::new(),
        }
    }

    fn now_str() -> String {
        chrono_now()
    }

    fn send_selected_files(&mut self, paths: Vec<PathBuf>) {
        let files = collect_paths_recursive(&paths);
        if files.is_empty() {
            return;
        }
        let peer_id = match &self.selected_peer_id {
            Some(id) => id.clone(),
            None => {
                if let Some(first_id) = self.peers.keys().next() {
                    let id = first_id.clone();
                    self.selected_peer_id = Some(id.clone());
                    id
                } else {
                    return;
                }
            }
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
                    sha256_hash: None,
                });
            }
        }

        // Add outgoing card to chat timeline with WaitingForAccept
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
                status: FileTransferStatus::WaitingForAccept,
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
            files: meta_list,
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
        self.chat_history.entry(peer_id).or_default().push(msg);

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
        egui::Color32::from_rgb(99, 102, 241), // Indigo
        egui::Color32::from_rgb(168, 85, 247), // Purple
        egui::Color32::from_rgb(236, 72, 153), // Pink
        egui::Color32::from_rgb(14, 165, 233), // Sky
        egui::Color32::from_rgb(20, 184, 166), // Teal
        egui::Color32::from_rgb(249, 115, 22), // Orange
    ];
    colors[(hash as usize) % colors.len()]
}

fn get_file_type_icon(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" => "📄",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "ico" => "🖼️",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => "🎵",
        "mp4" | "mkv" | "avi" | "mov" | "webm" => "🎬",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => "📦",
        "exe" | "msi" | "bat" | "cmd" | "ps1" => "⚙️",
        "rs" | "py" | "js" | "ts" | "html" | "css" | "json" | "c" | "cpp" => "💻",
        _ => "📁",
    }
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
                GuiEvent::PeerAdded(mut peer) => {
                    peer.is_online = true;
                    let (ip_str, port_u16) = if let Some((ip, port)) = peer.addr.split_once(':') {
                        (ip, port.parse().unwrap_or(peer.port))
                    } else {
                        (peer.addr.as_str(), peer.port)
                    };
                    self.config.record_discovered_peer(
                        &peer.id,
                        &peer.name,
                        &peer.fingerprint,
                        ip_str,
                        port_u16,
                    );
                    let pid = peer.id.clone();
                    self.peers.insert(pid.clone(), peer);
                    if self.selected_peer_id.is_none() {
                        self.selected_peer_id = Some(pid);
                    }
                }
                GuiEvent::PeerRemoved(id) => {
                    // Do not discard device from UI: keep in history, mark offline
                    if let Some(p) = self.peers.get_mut(&id) {
                        p.is_online = false;
                    }
                }
                GuiEvent::IncomingOffer(req) => {
                    let peer_id = req.offer.sender_device_id.clone();
                    let sender_name = req.offer.sender_name.clone();
                    let total_bytes = req.offer.files.iter().map(|f| f.size_bytes).sum();
                    let session_id = req.offer.session_id.clone();
                    let decision_tx_arc = Arc::new(tokio::sync::Mutex::new(Some(req.response_tx)));

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
                GuiEvent::FileCompleted {
                    session_id,
                    file_id: _,
                    dest_path,
                } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                progress,
                                status,
                                local_path,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *progress = 1.0;
                                    *status = FileTransferStatus::Completed;
                                    *local_path = Some(dest_path.clone());

                                    let file_name = dest_path
                                        .file_name()
                                        .map(|f| f.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Received File".into());
                                    let size_bytes =
                                        std::fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
                                    self.transfer_history.push(HistoryRecord {
                                        id: session_id.clone(),
                                        filename: file_name,
                                        size_bytes,
                                        is_outgoing: false,
                                        peer_name: "Incoming".into(),
                                        time_str: Self::now_str(),
                                        status: "Completed".into(),
                                        local_path: Some(dest_path.clone()),
                                    });
                                }
                            }
                        }
                    }
                }
                GuiEvent::TransferPaused { session_id } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *status =
                                        FileTransferStatus::Paused("Peer paused transfer".into());
                                }
                            }
                        }
                    }
                }
                GuiEvent::TransferResumed { session_id } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *status = FileTransferStatus::Transferring;
                                }
                            }
                        }
                    }
                }
                GuiEvent::TransferCancelled { session_id } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *status = FileTransferStatus::Cancelled;
                                }
                            }
                        }
                    }
                }
                GuiEvent::OutgoingTransferStarted { session_id } => {
                    for msgs in self.chat_history.values_mut() {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id: s_id,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if s_id == &session_id {
                                    *status = FileTransferStatus::Transferring;
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
                    file_name,
                    bytes_transferred,
                    total_bytes,
                    speed_mb_s,
                    is_done,
                    error,
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
                                        *progress =
                                            (bytes_transferred as f64 / total_bytes as f64) as f32;
                                    }
                                    *spd = speed_mb_s;
                                    if is_done {
                                        if let Some(err) = error.clone() {
                                            if err.to_ascii_lowercase().contains("decline") {
                                                *status = FileTransferStatus::Declined;
                                            } else if err.to_ascii_lowercase().contains("connection") || err.to_ascii_lowercase().contains("reset") {
                                                *status = FileTransferStatus::Paused("paused (connection lost)".into());
                                            } else {
                                                *status = FileTransferStatus::Failed(err.clone());
                                                self.transfer_history.push(HistoryRecord {
                                                    id: session_id.clone(),
                                                    filename: file_name.clone(),
                                                    size_bytes: total_bytes,
                                                    is_outgoing: true,
                                                    peer_name: peer_id.clone(),
                                                    time_str: Self::now_str(),
                                                    status: format!("Failed: {err}"),
                                                    local_path: None,
                                                });
                                            }
                                        } else {
                                            *status = FileTransferStatus::Completed;
                                            *progress = 1.0;
                                            self.transfer_history.push(HistoryRecord {
                                                id: session_id.clone(),
                                                filename: file_name.clone(),
                                                size_bytes: total_bytes,
                                                is_outgoing: true,
                                                peer_name: peer_id.clone(),
                                                time_str: Self::now_str(),
                                                status: "Completed".into(),
                                                local_path: None,
                                            });
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
        if !dropped_files.is_empty() {
            self.send_selected_files(dropped_files);
        }

        // Visual theme adjustment matching dropit_redesign_preview.html
        let is_dark = self.config.dark_theme;
        let theme = AppTheme::new(is_dark);

        // 1. Narrow Icon Rail (Transfers, History, Favorites, Nearby, Settings) - 56px
        egui::SidePanel::left("icon_rail")
            .resizable(false)
            .exact_width(56.0)
            .frame(egui::Frame::NONE.fill(theme.rail_bg).stroke(egui::Stroke::new(0.5_f32, theme.border)))
            .show(ctx, |ui| {
                self.render_icon_rail(ui, theme);
            });

        // 2. Device Sidebar (This device card, search, Favorites, Nearby) - 195px
        egui::SidePanel::left("sidebar_panel")
            .resizable(false)
            .exact_width(195.0)
            .frame(egui::Frame::NONE.fill(theme.surface_1).stroke(egui::Stroke::new(0.5_f32, theme.border)))
            .show(ctx, |ui| {
                self.render_sidebar(ui, theme);
            });

        // 3. Main Panel (Based on active rail tab)
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme.surface_2))
            .show(ctx, |ui| match self.active_rail_tab {
                GuiRailTab::Transfers => self.render_chat_area(ui, theme),
                GuiRailTab::History => self.render_history_area(ui, theme),
                GuiRailTab::Favorites => self.render_favorites_area(ui, theme),
                GuiRailTab::Nearby => self.render_nearby_area(ui, theme),
                GuiRailTab::Settings => self.render_settings_area(ui, theme),
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
    fn render_icon_rail(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        ui.add_space(12.0);

        let tabs = [
            (GuiRailTab::Transfers, "⇄", "Transfers"),
            (GuiRailTab::History, "🕒", "History"),
            (GuiRailTab::Favorites, "⭐", "Favorites"),
            (GuiRailTab::Nearby, "📡", "Nearby"),
        ];

        for (tab, icon, label) in tabs {
            let is_selected = self.active_rail_tab == tab;
            let (btn_rect, response) =
                ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
            
            let bg_color = if is_selected {
                theme.bg_accent
            } else if response.hovered() {
                theme.surface_1
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(btn_rect, 10.0, bg_color);
            
            let text_color = if is_selected {
                theme.text_accent
            } else {
                theme.text_secondary
            };
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(17.0),
                text_color,
            );
            if response.on_hover_text(label).clicked() {
                self.active_rail_tab = tab;
            }
            ui.add_space(14.0);
        }

        // Bottom controls: Settings, QR Code, Theme toggle
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(12.0);

            // Settings icon
            let is_settings = self.active_rail_tab == GuiRailTab::Settings;
            let (settings_rect, settings_resp) =
                ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
            let bg_color = if is_settings {
                theme.bg_accent
            } else if settings_resp.hovered() {
                theme.surface_1
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(settings_rect, 10.0, bg_color);
            let text_color = if is_settings {
                theme.text_accent
            } else {
                theme.text_secondary
            };
            ui.painter().text(
                settings_rect.center(),
                egui::Align2::CENTER_CENTER,
                "⚙",
                egui::FontId::proportional(17.0),
                text_color,
            );
            if settings_resp.on_hover_text("Settings").clicked() {
                self.active_rail_tab = GuiRailTab::Settings;
            }

            ui.add_space(10.0);
            if ui
                .add(egui::Button::new(egui::RichText::new("📱").size(15.0).color(theme.text_secondary)).frame(false))
                .on_hover_text("Mobile Web UI (QR Code)")
                .clicked()
            {
                self.show_qr = true;
            }

            ui.add_space(10.0);
            let theme_icon = if self.config.dark_theme { "☀️" } else { "🌙" };
            let theme_label = if self.config.dark_theme {
                "Switch to Light Theme"
            } else {
                "Switch to Dark Theme"
            };
            if ui
                .add(egui::Button::new(egui::RichText::new(theme_icon).size(15.0).color(theme.text_secondary)).frame(false))
                .on_hover_text(theme_label)
                .clicked()
            {
                self.config.dark_theme = !self.config.dark_theme;
                let _ = self.config.save();
            }
        });
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        ui.add_space(12.0);

        // "This device" Header
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("This device")
                    .size(13.0)
                    .color(theme.text_secondary),
            );
        });
        ui.add_space(6.0);

        // "This device" Avatar DI + Device Name + Online dot + Stealth toggle
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 14.0, theme.bg_pro);

            let initials = if self.config.device_name.len() >= 2 {
                self.config.device_name[..2].to_uppercase()
            } else {
                "DI".to_string()
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::proportional(11.5),
                theme.text_pro,
            );

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(&self.config.device_name)
                    .size(14.0)
                    .color(theme.text_primary),
            );

            let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(
                dot_rect.center(),
                3.5,
                if self.config.visible_to_nearby {
                    theme.text_success
                } else {
                    theme.text_muted
                },
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let mut visible = self.config.visible_to_nearby;
                if ui
                    .checkbox(&mut visible, "")
                    .on_hover_text("Visible to nearby devices")
                    .changed()
                {
                    self.config.visible_to_nearby = visible;
                    let _ = self.config.save();
                }
            });
        });

        ui.add_space(12.0);

        // Search Input
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let width = ui.available_width() - 12.0;
            ui.add_sized(
                [width, 30.0],
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search devices")
                    .margin(egui::vec2(8.0, 5.0)),
            );
        });

        ui.add_space(12.0);

        // Device lists (Favorites & Nearby)
        egui::ScrollArea::vertical().show(ui, |ui| {
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
            let mut nearby_peers = Vec::new();

            for p in filtered_peers {
                if self.config.is_favorite(&p.id) {
                    fav_peers.push(p);
                } else {
                    nearby_peers.push(p);
                }
            }

            // Favorites section
            if !fav_peers.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("Favorites")
                            .size(12.0)
                            .color(theme.text_muted),
                    );
                });
                ui.add_space(4.0);
                for p in fav_peers {
                    self.render_peer_item(ui, &p, theme, true);
                }
                ui.add_space(14.0);
            }

            // Nearby section
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Nearby")
                        .size(12.0)
                        .color(theme.text_muted),
                );
            });
            ui.add_space(4.0);

            if nearby_peers.is_empty() && self.peers.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("Searching for devices...")
                            .size(12.0)
                            .color(theme.text_muted),
                    );
                });
            } else {
                for p in nearby_peers {
                    self.render_peer_item(ui, &p, theme, false);
                }
            }
        });
    }

    fn render_peer_item(&mut self, ui: &mut egui::Ui, peer: &GuiPeer, theme: AppTheme, is_fav_section: bool) {
        let is_selected = self.selected_peer_id.as_deref() == Some(&peer.id);
        let is_fav = self.config.is_favorite(&peer.id);
        let display_name = self.config.get_display_name(&peer.id, &peer.name).to_string();

        let (bg_color, stroke) = if is_selected {
            (theme.bg_accent_muted, egui::Stroke::new(0.5_f32, theme.border_accent))
        } else {
            (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
        };

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let frame = egui::Frame::NONE
                .fill(bg_color)
                .stroke(stroke)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(8, 6));

            let frame_res = frame.show(ui, |ui| {
                let avail_w = ui.available_width() - 8.0;
                ui.set_width(avail_w);

                ui.horizontal(|ui| {
                    // 30x30 avatar circle
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                    let (avatar_bg, avatar_fg) = if is_fav_section || is_fav {
                        (theme.bg_warning, theme.text_warning)
                    } else {
                        (theme.surface_1, theme.text_secondary)
                    };
                    ui.painter().circle_filled(rect.center(), 15.0, avatar_bg);

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
                        egui::FontId::proportional(13.0),
                        avatar_fg,
                    );

                    ui.add_space(8.0);

                    // Name + Status
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(&display_name)
                                .size(14.0)
                                .color(theme.text_primary),
                        );
                        let (status_text, status_color) = if peer.is_online {
                            ("Online", theme.text_success)
                        } else if !peer.fingerprint.is_empty() {
                            ("Offline", theme.text_muted)
                        } else {
                            ("Tap to pair", theme.text_muted)
                        };
                        ui.label(
                            egui::RichText::new(status_text)
                                .size(12.0)
                                .color(status_color),
                        );
                    });

                    // Star button
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let star_icon = if is_fav { "⭐" } else { "✩" };
                        let star_color = if is_fav {
                            theme.text_warning
                        } else {
                            theme.text_muted
                        };
                        if ui
                            .add(egui::Button::new(egui::RichText::new(star_icon).size(13.0).color(star_color)).frame(false))
                            .on_hover_text(if is_fav { "Remove favorite" } else { "Add favorite" })
                            .clicked()
                        {
                            let _ = self.config.toggle_favorite(&peer.id);
                        }
                    });
                });
            });

            if ui
                .interact(frame_res.response.rect, ui.id().with(&peer.id), egui::Sense::click())
                .clicked()
            {
                self.selected_peer_id = Some(peer.id.clone());
            }
        });
    }

    fn render_chat_area(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        let selected_id = match &self.selected_peer_id {
            Some(id) => id.clone(),
            None => {
                if let Some(first_id) = self.peers.keys().next().cloned() {
                    self.selected_peer_id = Some(first_id.clone());
                    first_id
                } else {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(egui::RichText::new("☁").size(36.0).color(theme.text_secondary));
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Drop It Local Transfer").size(18.0).strong().color(theme.text_primary));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Select a device from the sidebar to start sharing").size(13.0).color(theme.text_muted));
                    });
                    return;
                }
            }
        };

        let peer = match self.peers.get(&selected_id) {
            Some(p) => p.clone(),
            None => return,
        };

        let display_name = self.config.get_display_name(&peer.id, &peer.name).to_string();

        ui.add_space(14.0);

        // Header Row matching dropit_redesign_preview.html
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&display_name)
                        .size(16.0)
                        .strong()
                        .color(theme.text_primary),
                );
                ui.label(
                    egui::RichText::new("🔒 Encrypted · TLS 1.3 · Same Wi-Fi")
                        .size(12.0)
                        .color(theme.text_secondary),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);

                // Primary accent button: Send files
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Send files")
                                .size(13.0)
                                .color(theme.on_accent),
                        )
                        .fill(theme.fill_accent)
                        .corner_radius(8.0)
                        .min_size(egui::vec2(86.0, 32.0)),
                    )
                    .on_hover_text("Choose files to send")
                    .clicked()
                {
                    if let Some(files) = rfd::FileDialog::new().pick_files() {
                        self.send_selected_files(files);
                    }
                }

                ui.add_space(6.0);

                // Secondary button: Send folder
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Send folder")
                                .size(13.0)
                                .color(theme.text_primary),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0_f32, theme.border_strong))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(90.0, 32.0)),
                    )
                    .on_hover_text("Send entire folder")
                    .clicked()
                {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        let files = collect_paths_recursive(&[folder]);
                        self.send_selected_files(files);
                    }
                }
            });
        });

        ui.add_space(12.0);

        // Dashed Drop Zone matching dropit_redesign_preview.html
        let is_hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let drop_w = ui.available_width() - 16.0;
            let (drop_rect, drop_resp) =
                ui.allocate_exact_size(egui::vec2(drop_w, 92.0), egui::Sense::click());

            let drop_bg = if is_hovering || drop_resp.hovered() {
                theme.bg_accent_muted
            } else {
                theme.surface_1
            };
            ui.painter().rect_filled(drop_rect, 12.0, drop_bg);

            let stroke_color = if is_hovering || drop_resp.hovered() {
                theme.fill_accent
            } else {
                theme.border_strong
            };
            paint_dashed_rect(ui.painter(), drop_rect, 12.0, egui::Stroke::new(1.5_f32, stroke_color));

            let c = drop_rect.center();
            ui.painter().text(
                egui::pos2(c.x, c.y - 18.0),
                egui::Align2::CENTER_CENTER,
                "☁",
                egui::FontId::proportional(26.0),
                theme.text_secondary,
            );
            ui.painter().text(
                egui::pos2(c.x, c.y + 10.0),
                egui::Align2::CENTER_CENTER,
                "Drop files or folders here",
                egui::FontId::proportional(15.0),
                theme.text_primary,
            );
            ui.painter().text(
                egui::pos2(c.x, c.y + 26.0),
                egui::Align2::CENTER_CENTER,
                "They send right away, no extra steps",
                egui::FontId::proportional(12.0),
                theme.text_muted,
            );

            if drop_resp.clicked() {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    self.send_selected_files(files);
                }
            }
        });

        ui.add_space(14.0);

        // Section Title: Transfers
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Transfers")
                    .size(12.0)
                    .color(theme.text_muted),
            );
        });
        ui.add_space(6.0);

        // Transfers Scroll Queue
        let available_h = ui.available_height() - 56.0; // room for bottom bar
        let mut bubble_action_to_apply: Option<BubbleAction> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .max_height(available_h)
            .show(ui, |ui| {
                if let Some(messages) = self.chat_history.get(&selected_id) {
                    let msgs_clone = messages.clone();
                    for msg in msgs_clone {
                        if let Some(action) = self.render_chat_bubble(ui, &msg, theme) {
                            bubble_action_to_apply = Some(action);
                        }
                        ui.add_space(8.0);
                    }
                }
            });

        // Apply transfer control or decision
        if let Some(action) = bubble_action_to_apply {
            match action {
                BubbleAction::Decision(sess_id, accepted) => {
                    if let Some(msgs) = self.chat_history.get_mut(&selected_id) {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id,
                                status,
                                decision_tx,
                                ..
                            } = &mut m.kind
                            {
                                if session_id == &sess_id {
                                    *status = if accepted {
                                        FileTransferStatus::Transferring
                                    } else {
                                        FileTransferStatus::Declined
                                    };
                                    if let Some(tx_arc) = decision_tx.take() {
                                        tokio::spawn(async move {
                                            let mut lock = tx_arc.lock().await;
                                            if let Some(tx) = lock.take() {
                                                let _ = tx.send(accepted);
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                BubbleAction::Control(sess_id, cmd) => {
                    if let Some(msgs) = self.chat_history.get_mut(&selected_id) {
                        for m in msgs.iter_mut() {
                            if let ChatMessageKind::FileCard {
                                session_id,
                                status,
                                ..
                            } = &mut m.kind
                            {
                                if session_id == &sess_id {
                                    match cmd {
                                        crate::transfer::TransferControlCommand::Pause => {
                                            *status =
                                                FileTransferStatus::Paused("Paused by user".into());
                                        }
                                        crate::transfer::TransferControlCommand::Resume => {
                                            *status = FileTransferStatus::Transferring;
                                        }
                                        crate::transfer::TransferControlCommand::Cancel => {
                                            *status = FileTransferStatus::Cancelled;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(p) = self.peers.get(&selected_id) {
                        let _ = self.tx_actions.try_send(GuiAction::TransferControl {
                            target_addr: p.addr.clone(),
                            session_id: sess_id,
                            command: cmd,
                        });
                    }
                }
            }
        }

        // Pinned Bottom Message Input Bar matching dropit_redesign_preview.html
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);

                // Clipboard paste button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("📋")
                                .size(15.0)
                                .color(theme.text_secondary),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(0.5_f32, theme.border))
                        .corner_radius(8.0)
                        .min_size(egui::vec2(32.0, 32.0)),
                    )
                    .on_hover_text("Paste & Send Clipboard")
                    .clicked()
                {
                    if let Ok(out) = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", "Get-Clipboard"])
                        .output()
                    {
                        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if !text.is_empty() {
                            self.text_input = text;
                            self.send_current_message();
                        }
                    }
                }

                let input_w = ui.available_width() - 58.0;
                let text_edit = ui.add_sized(
                    [input_w, 32.0],
                    egui::TextEdit::singleline(&mut self.text_input)
                        .hint_text("Type a message")
                        .margin(egui::vec2(10.0, 6.0)),
                );

                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.send_current_message();
                }

                // Send button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("➤")
                                .size(14.0)
                                .color(theme.on_accent),
                        )
                        .fill(theme.fill_accent)
                        .corner_radius(8.0)
                        .min_size(egui::vec2(36.0, 32.0)),
                    )
                    .on_hover_text("Send message")
                    .clicked()
                {
                    self.send_current_message();
                }
            });
        });
    }

    fn render_chat_bubble(
        &mut self,
        ui: &mut egui::Ui,
        msg: &ChatMessage,
        theme: AppTheme,
    ) -> Option<BubbleAction> {
        let is_me = msg.is_me;
        let mut bubble_action: Option<BubbleAction> = None;

        match &msg.kind {
            ChatMessageKind::Text(text) => {
                let align = if is_me { egui::Align::Max } else { egui::Align::Min };
                ui.with_layout(egui::Layout::top_down(align), |ui| {
                    ui.horizontal(|ui| {
                        if !is_me {
                            ui.add_space(16.0);
                        }
                        let bg = if is_me { theme.fill_accent } else { theme.surface_1 };
                        let fg = if is_me { theme.on_accent } else { theme.text_primary };
                        let stroke = if is_me { egui::Stroke::NONE } else { egui::Stroke::new(0.5_f32, theme.border) };
                        egui::Frame::NONE
                            .fill(bg)
                            .stroke(stroke)
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.label(egui::RichText::new(text).size(13.5).color(fg));
                            });
                        if is_me {
                            ui.add_space(16.0);
                        }
                    });
                });
            }

            ChatMessageKind::FileCard {
                session_id,
                files,
                total_bytes,
                progress,
                speed_mb_s,
                status,
                local_path,
                ..
            } => {
                let first_name = files
                    .first()
                    .map(|f| f.filename.clone())
                    .unwrap_or_else(|| "File".into());

                let file_icon = match first_name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
                    "mp4" | "mkv" | "mov" | "avi" | "webm" => "🎬",
                    "png" | "jpg" | "jpeg" | "gif" | "webp" => "🖼",
                    "pdf" | "doc" | "docx" | "txt" => "📄",
                    "zip" | "rar" | "7z" | "tar" => "📦",
                    _ => "📄",
                };

                let title = if files.len() > 1 {
                    format!("{} (+{} more)", first_name, files.len() - 1)
                } else {
                    first_name
                };

                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let card_w = ui.available_width() - 16.0;
                    egui::Frame::NONE
                        .fill(theme.surface_2)
                        .stroke(egui::Stroke::new(0.5_f32, theme.border))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(card_w);

                            // Top row: File icon + Name, Right: Status or percent/speed
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(file_icon).size(14.0));
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(&title)
                                        .size(13.0)
                                        .color(theme.text_primary),
                                );

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    match status {
                                        FileTransferStatus::Transferring => {
                                            let pct = (*progress * 100.0).clamp(0.0, 100.0) as u32;
                                            ui.label(
                                                egui::RichText::new(format!("{}% · {:.1} MB/s", pct, speed_mb_s))
                                                    .size(12.5)
                                                    .color(theme.text_secondary),
                                            );
                                        }
                                        FileTransferStatus::Completed => {
                                            ui.label(
                                                egui::RichText::new("✔ Sent")
                                                    .size(12.5)
                                                    .color(theme.text_success),
                                            );
                                        }
                                        FileTransferStatus::Paused(reason) => {
                                            ui.label(
                                                egui::RichText::new(format!("⏸ {reason}"))
                                                    .size(12.0)
                                                    .color(theme.text_warning),
                                            );
                                        }
                                        FileTransferStatus::Cancelled => {
                                            ui.label(
                                                egui::RichText::new("✕ Cancelled")
                                                    .size(12.0)
                                                    .color(theme.text_muted),
                                            );
                                        }
                                        FileTransferStatus::Failed(err) => {
                                            ui.label(
                                                egui::RichText::new(format!("✕ Failed: {err}"))
                                                    .size(12.0)
                                                    .color(egui::Color32::from_rgb(239, 68, 68)),
                                            );
                                        }
                                        FileTransferStatus::WaitingForAccept => {
                                            ui.label(
                                                egui::RichText::new("Waiting for accept...")
                                                    .size(12.0)
                                                    .color(theme.text_warning),
                                            );
                                        }
                                        FileTransferStatus::PendingOffer => {
                                            ui.label(
                                                egui::RichText::new("Incoming offer")
                                                    .size(12.0)
                                                    .color(theme.text_warning),
                                            );
                                        }
                                        FileTransferStatus::Declined => {
                                            ui.label(
                                                egui::RichText::new("Declined")
                                                    .size(12.0)
                                                    .color(egui::Color32::from_rgb(239, 68, 68)),
                                            );
                                        }
                                    }
                                });
                            });

                            // Slim progress bar (4px height, 2px radius)
                            if matches!(status, FileTransferStatus::Transferring | FileTransferStatus::Paused(_)) {
                                ui.add_space(8.0);
                                let (bar_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 4.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(bar_rect, 2.0, theme.surface_1);
                                let fill_w = bar_rect.width() * (*progress).clamp(0.0, 1.0);
                                if fill_w > 0.0 {
                                    let mut filled_rect = bar_rect;
                                    filled_rect.set_width(fill_w);
                                    ui.painter().rect_filled(filled_rect, 2.0, theme.fill_accent);
                                }

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format_bytes(*total_bytes))
                                            .size(11.5)
                                            .color(theme.text_muted),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new("Cancel").size(11.5).color(egui::Color32::from_rgb(239, 68, 68)),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::new(0.5_f32, theme.border))
                                                .corner_radius(6.0)
                                                .min_size(egui::vec2(54.0, 24.0)),
                                            )
                                            .clicked()
                                        {
                                            bubble_action = Some(BubbleAction::Control(session_id.clone(), crate::transfer::TransferControlCommand::Cancel));
                                        }

                                        if matches!(status, FileTransferStatus::Transferring) {
                                            if ui
                                                .add(
                                                    egui::Button::new(egui::RichText::new("Pause").size(11.5).color(theme.text_secondary))
                                                        .fill(egui::Color32::TRANSPARENT)
                                                        .stroke(egui::Stroke::new(0.5_f32, theme.border))
                                                        .corner_radius(6.0)
                                                        .min_size(egui::vec2(54.0, 24.0)),
                                                )
                                                .clicked()
                                            {
                                                bubble_action = Some(BubbleAction::Control(session_id.clone(), crate::transfer::TransferControlCommand::Pause));
                                            }
                                        } else {
                                            if ui
                                                .add(
                                                    egui::Button::new(egui::RichText::new("Resume").size(11.5).color(theme.on_accent))
                                                        .fill(theme.fill_accent)
                                                        .corner_radius(6.0)
                                                        .min_size(egui::vec2(58.0, 24.0)),
                                                )
                                                .clicked()
                                            {
                                                bubble_action = Some(BubbleAction::Control(session_id.clone(), crate::transfer::TransferControlCommand::Resume));
                                            }
                                        }
                                    });
                                });
                            } else if matches!(status, FileTransferStatus::PendingOffer) {
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(egui::RichText::new("Accept").size(12.0).color(theme.on_accent))
                                                .fill(theme.fill_accent)
                                                .corner_radius(6.0),
                                        )
                                        .clicked()
                                    {
                                        bubble_action = Some(BubbleAction::Decision(session_id.clone(), true));
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(egui::RichText::new("Decline").size(12.0).color(egui::Color32::from_rgb(239, 68, 68)))
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::new(0.5_f32, theme.border))
                                                .corner_radius(6.0),
                                        )
                                        .clicked()
                                    {
                                        bubble_action = Some(BubbleAction::Decision(session_id.clone(), false));
                                    }
                                });
                            } else if matches!(status, FileTransferStatus::Completed) {
                                if let Some(path) = local_path {
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        let p1 = path.clone();
                                        if ui.button("Open file").clicked() {
                                            platform::open_file(&p1);
                                        }
                                        let p2 = path.clone();
                                        if ui.button("Show in folder").clicked() {
                                            platform::open_in_folder(&p2);
                                        }
                                    });
                                }
                            }
                        });
                });
            }
        }

        bubble_action
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
                    .checkbox(
                        &mut self.startup_enabled,
                        "Launch Drop It on Windows Startup",
                    )
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
                    ui.hyperlink_to(
                        "github.com/golam-mowla/dropit",
                        "https://github.com/golam-mowla/dropit",
                    );
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

    fn render_history_area(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        let is_dark = self.config.dark_theme;
        let text_color = theme.text_primary;
        let subtext_color = theme.text_secondary;
        let card_bg = theme.surface_1;
        let card_stroke = egui::Stroke::new(0.5_f32, theme.border);

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("🕒 Transfer History")
                    .size(18.0)
                    .strong()
                    .color(text_color),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Clear history").size(12.0),
                        )
                        .corner_radius(6.0)
                        .fill(if is_dark {
                            egui::Color32::from_rgb(37, 45, 64)
                        } else {
                            egui::Color32::from_rgb(229, 231, 235)
                        }),
                    )
                    .clicked()
                {
                    self.transfer_history.clear();
                }
            });
        });
        ui.add_space(12.0);
        ui.separator();

        if self.transfer_history.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new("No transfers recorded yet")
                        .size(14.0)
                        .color(subtext_color),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                for item in self.transfer_history.iter().rev() {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        egui::Frame::NONE
                            .fill(card_bg)
                            .stroke(card_stroke)
                            .corner_radius(12.0)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 32.0);
                                ui.horizontal(|ui| {
                                    let icon = get_file_type_icon(&item.filename);
                                    ui.label(egui::RichText::new(icon).size(22.0));
                                    ui.add_space(6.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&item.filename)
                                                .size(13.5)
                                                .strong()
                                                .color(text_color),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  •  {}  •  {}",
                                                format_bytes(item.size_bytes),
                                                item.time_str,
                                                item.status
                                            ))
                                            .size(11.0)
                                            .color(subtext_color),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if let Some(ref path) = item.local_path {
                                                let p1 = path.clone();
                                                if ui.button("Open file").clicked() {
                                                    platform::open_file(&p1);
                                                }
                                                let p2 = path.clone();
                                                if ui.button("Show in folder").clicked() {
                                                    platform::open_in_folder(&p2);
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                    });
                    ui.add_space(8.0);
                }
            });
        }
    }

    fn render_favorites_area(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        let text_color = theme.text_primary;
        let subtext_color = theme.text_secondary;
        let card_bg = theme.surface_1;
        let card_stroke = egui::Stroke::new(0.5_f32, theme.border);

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("⭐ Favorite Devices")
                    .size(18.0)
                    .strong()
                    .color(text_color),
            );
        });
        ui.add_space(12.0);
        ui.separator();

        let favs: Vec<GuiPeer> = self
            .peers
            .values()
            .filter(|p| self.config.is_favorite(&p.id))
            .cloned()
            .collect();
        if favs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new(
                        "No favorite devices yet. Click the ⭐ next to any device to add it.",
                    )
                    .size(13.5)
                    .color(subtext_color),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                for peer in favs {
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        egui::Frame::NONE
                            .fill(card_bg)
                            .stroke(card_stroke)
                            .corner_radius(12.0)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 32.0);
                                ui.horizontal(|ui| {
                                    let initial = peer
                                        .name
                                        .chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_uppercase()
                                        .to_string();
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(32.0, 32.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        16.0,
                                        color_for_name(&peer.name),
                                    );
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        initial,
                                        egui::FontId::proportional(14.0),
                                        egui::Color32::WHITE,
                                    );
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&peer.name)
                                                .size(14.0)
                                                .strong()
                                                .color(text_color),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  •  {}",
                                                peer.addr,
                                                if peer.is_online {
                                                    "🟢 Online"
                                                } else {
                                                    "Offline"
                                                }
                                            ))
                                            .size(11.0)
                                            .color(subtext_color),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let mut auto_acc = self
                                                .config
                                                .is_auto_accept_for_favorite(&peer.id);
                                            if ui.checkbox(&mut auto_acc, "Auto-accept").changed() {
                                                self.config
                                                    .set_auto_accept_for_favorite(&peer.id, auto_acc);
                                                let _ = self.config.save();
                                            }
                                            if ui.button("Send files").clicked() {
                                                self.selected_peer_id = Some(peer.id.clone());
                                                self.active_rail_tab = GuiRailTab::Transfers;
                                                if let Some(files) =
                                                    rfd::FileDialog::new().pick_files()
                                                {
                                                    self.send_selected_files(files);
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                    });
                    ui.add_space(8.0);
                }
            });
        }
    }

    fn render_nearby_area(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        let text_color = theme.text_primary;
        let subtext_color = theme.text_secondary;
        let card_bg = theme.surface_1;
        let card_stroke = egui::Stroke::new(0.5_f32, theme.border);

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("📡 Nearby Discovered Devices")
                    .size(18.0)
                    .strong()
                    .color(text_color),
            );
        });
        ui.add_space(12.0);
        ui.separator();

        let online: Vec<GuiPeer> = self.peers.values().filter(|p| p.is_online).cloned().collect();
        if online.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new("Searching for devices on same WiFi...")
                        .size(13.5)
                        .color(subtext_color),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                for peer in online {
                    let pairing_code = crypto::generate_pairing_code(
                        &self.identity.fingerprint,
                        &peer.fingerprint,
                    );
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        egui::Frame::NONE
                            .fill(card_bg)
                            .stroke(card_stroke)
                            .corner_radius(12.0)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width() - 32.0);
                                ui.horizontal(|ui| {
                                    let initial = peer
                                        .name
                                        .chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_uppercase()
                                        .to_string();
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(32.0, 32.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        16.0,
                                        color_for_name(&peer.name),
                                    );
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        initial,
                                        egui::FontId::proportional(14.0),
                                        egui::Color32::WHITE,
                                    );
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&peer.name)
                                                .size(14.0)
                                                .strong()
                                                .color(text_color),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  •  Pairing Code: [{}]",
                                                peer.addr, pairing_code
                                            ))
                                            .size(11.0)
                                            .color(subtext_color),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("Connect & Send").clicked() {
                                                self.selected_peer_id = Some(peer.id.clone());
                                                self.active_rail_tab = GuiRailTab::Transfers;
                                            }
                                        },
                                    );
                                });
                            });
                    });
                    ui.add_space(8.0);
                }
            });
        }
    }

    fn render_settings_area(&mut self, ui: &mut egui::Ui, theme: AppTheme) {
        let text_color = theme.text_primary;
        let card_bg = theme.surface_1;
        let card_stroke = egui::Stroke::new(0.5_f32, theme.border);

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("⚙ Application Settings")
                    .size(18.0)
                    .strong()
                    .color(text_color),
            );
        });
        ui.add_space(12.0);
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                egui::Frame::NONE
                    .fill(card_bg)
                    .stroke(card_stroke)
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::symmetric(18, 16))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width() - 32.0);
                        ui.label(
                            egui::RichText::new("Device Identity")
                                .size(14.0)
                                .strong()
                                .color(text_color),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label("Device name:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.device_name)
                                    .desired_width(220.0),
                            );
                            if ui.button("Save").clicked() {
                                let _ = self.config.save();
                            }
                        });
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(12.0);

                        ui.label(
                            egui::RichText::new("Save Location")
                                .size(14.0)
                                .strong()
                                .color(text_color),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(format!("Path: {}", self.config.download_dir.display()));
                            if ui.button("Browse...").clicked() {
                                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                    self.config.download_dir = folder;
                                    let _ = self.config.save();
                                }
                            }
                        });
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(12.0);

                        ui.label(
                            egui::RichText::new("Preferences")
                                .size(14.0)
                                .strong()
                                .color(text_color),
                        );
                        ui.add_space(8.0);
                        if ui
                            .checkbox(
                                &mut self.config.auto_accept_trusted,
                                "Auto-accept files from trusted devices",
                            )
                            .changed()
                        {
                            let _ = self.config.save();
                        }
                        if ui
                            .checkbox(
                                &mut self.config.visible_to_nearby,
                                "Visible to nearby devices",
                            )
                            .changed()
                        {
                            let _ = self.config.save();
                        }
                        if ui
                            .checkbox(
                                &mut self.config.minimize_to_tray,
                                "Minimize to system tray on close",
                            )
                            .changed()
                        {
                            let _ = self.config.save();
                        }
                        if ui
                            .checkbox(&mut self.config.dark_theme, "Use Dark Theme")
                            .changed()
                        {
                            let _ = self.config.save();
                        }
                        if ui
                            .checkbox(
                                &mut self.startup_enabled,
                                "Launch Drop It when Windows starts",
                            )
                            .changed()
                        {
                            let _ = platform::set_run_on_startup(self.startup_enabled);
                        }
                        if ui
                            .checkbox(
                                &mut self.context_menu_enabled,
                                "Add 'Send with Drop It' to Windows Explorer right-click menu",
                            )
                            .changed()
                        {
                            let _ = platform::set_context_menu_enabled(self.context_menu_enabled);
                        }
                    });
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
                PeerEvent::Added(p) | PeerEvent::Updated(p) => {
                    let _ = tx_peers.send(GuiEvent::PeerAdded(GuiPeer {
                        id: p.announcement.device_id,
                        name: p.announcement.name,
                        addr: p.address.to_string(),
                        port: p.announcement.port,
                        fingerprint: p.announcement.fingerprint,
                        version: p.announcement.version,
                        last_seen: Instant::now(),
                        is_online: true,
                    }));
                }
                PeerEvent::Removed(id) => {
                    let _ = tx_peers.send(GuiEvent::PeerRemoved(id));
                }
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
                WebSseEvent::FileCompleted {
                    session_id,
                    file_id,
                    dest_path,
                } => {
                    let _ = tx_sse.send(GuiEvent::FileCompleted {
                        session_id,
                        file_id,
                        dest_path: PathBuf::from(dest_path),
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
                WebSseEvent::TransferPaused { session_id } => {
                    let _ = tx_sse.send(GuiEvent::TransferPaused { session_id });
                }
                WebSseEvent::TransferResumed { session_id } => {
                    let _ = tx_sse.send(GuiEvent::TransferResumed { session_id });
                }
                WebSseEvent::TransferCancelled { session_id } => {
                    let _ = tx_sse.send(GuiEvent::TransferCancelled { session_id });
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
                        .send_quick_message(
                            &target_addr,
                            &cfg.device_id,
                            &cfg.device_name,
                            cfg.port,
                            &id_ref.fingerprint,
                            &text,
                        )
                        .await;
                }
                GuiAction::SendFiles {
                    target_addr,
                    target_fp: _,
                    target_name: _,
                    target_peer_id,
                    session_id,
                    files,
                    file_paths,
                } => {
                    let client_clone = client.clone();
                    let cfg_guard = cfg_ref.read().await;
                    let dev_name = cfg_guard.device_name.clone();
                    let dev_id = cfg_guard.device_id.clone();
                    let dev_port = cfg_guard.port;
                    let fp = id_ref.fingerprint.clone();
                    let tx_fb = tx_action_feedback.clone();
                    drop(cfg_guard);

                    tokio::spawn(async move {
                        let offer_req = TransferOfferRequest {
                            session_id: session_id.clone(),
                            sender_name: dev_name,
                            sender_device_id: dev_id,
                            sender_fingerprint: fp,
                            files: files.clone(),
                            sender_port: dev_port,
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
                                        error: Some(
                                            resp.reason
                                                .unwrap_or_else(|| "Declined by receiver".into()),
                                        ),
                                    });
                                    return;
                                }

                                let _ = tx_fb.send(GuiEvent::OutgoingTransferStarted {
                                    session_id: session_id.clone(),
                                });

                                let token = match resp.session_token {
                                    Some(t) => t,
                                    None => return,
                                };

                                let items: Vec<(FileMetadata, PathBuf)> =
                                    files.iter().cloned().zip(file_paths).collect();

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
                GuiAction::TransferControl {
                    target_addr,
                    session_id,
                    command,
                } => {
                    let _ = client
                        .send_transfer_control(&target_addr, &session_id, command)
                        .await;
                }
            }
        }
    });

    // Run native eframe desktop window
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Drop It")
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([720.0, 480.0])
        .with_drag_and_drop(true);

    if let Ok((rgba, w, h)) = platform::load_app_icon_rgba() {
        viewport = viewport.with_icon(egui::IconData {
            rgba,
            width: w,
            height: h,
        });
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

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
