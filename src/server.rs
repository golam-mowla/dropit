use crate::client::DropItClient;
use crate::config::{DropItConfig, PeerTrustStatus};
use crate::crypto::{self, TlsIdentity};
use crate::discovery::PeerRegistry;
use crate::security::{is_private_or_loopback_ip, resolve_destination_path};
use crate::transfer::{
    ActiveSession, FileMetadata, TransferOfferRequest, TransferOfferResponse, UploadResult,
};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Multipart, Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        Html, IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::io::SeekFrom;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

pub const BUFFER_SIZE: usize = 1024 * 1024; // 1 MB buffer chunk
pub const MAX_CONCURRENT_SESSIONS: usize = 8;
pub const SESSION_TTL: Duration = Duration::from_secs(300); // 5 minutes
pub const MAX_FILES_PER_OFFER: usize = 10_000;
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024 * 1024; // 1 TB per file
pub const MAX_OFFER_TOTAL_SIZE: u64 = 2 * 1024 * 1024 * 1024 * 1024; // 2 TB total per offer

static EMBEDDED_UI: &str = include_str!("../ui/index.html");

#[derive(Clone)]
pub struct ServerState {
    pub config: Arc<RwLock<DropItConfig>>,
    pub identity: TlsIdentity,
    pub active_sessions: Arc<RwLock<HashMap<String, ActiveSession>>>,
    pub recent_offers: Arc<RwLock<HashMap<IpAddr, Instant>>>,
    pub peer_registry: Option<PeerRegistry>,
    pub offer_prompt_tx: Option<mpsc::Sender<OfferDecisionRequest>>,
    pub pending_web_decisions: Arc<RwLock<HashMap<String, oneshot::Sender<bool>>>>,
    pub sse_broadcast_tx: broadcast::Sender<WebSseEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum WebSseEvent {
    Peers(serde_json::Value),
    Offer {
        decision_id: String,
        sender_name: String,
        sender_device_id: String,
        sender_fingerprint: String,
        pairing_code: String,
        files: Vec<FileMetadata>,
        total_bytes: u64,
    },
    Progress {
        transfer_id: String,
        percent: f64,
        loaded: u64,
        total: u64,
        speed_mb_s: f64,
    },
    FileCompleted {
        session_id: String,
        file_id: String,
        dest_path: String,
    },
    Message {
        sender_id: String,
        sender_name: String,
        text: String,
    },
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct QuickMessagePayload {
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
}

pub struct OfferDecisionRequest {
    pub offer: TransferOfferRequest,
    pub pairing_code: String,
    pub is_trusted: bool,
    pub response_tx: oneshot::Sender<bool>,
}

#[derive(Deserialize)]
pub struct DecisionPayload {
    pub decision_id: String,
    pub accepted: bool,
}

#[derive(Deserialize)]
pub struct UpdateSettingsPayload {
    pub name: Option<String>,
    pub auto_accept_trusted: Option<bool>,
    pub verify_hash: Option<bool>,
}

impl ServerState {
    pub fn new(
        config: Arc<RwLock<DropItConfig>>,
        identity: TlsIdentity,
        peer_registry: Option<PeerRegistry>,
        offer_prompt_tx: Option<mpsc::Sender<OfferDecisionRequest>>,
    ) -> Self {
        let (sse_broadcast_tx, _) = broadcast::channel(64);
        Self {
            config,
            identity,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            recent_offers: Arc::new(RwLock::new(HashMap::new())),
            peer_registry,
            offer_prompt_tx,
            pending_web_decisions: Arc::new(RwLock::new(HashMap::new())),
            sse_broadcast_tx,
        }
    }
}

pub async fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(handle_get_index))
        .route("/api/info", get(handle_get_info))
        .route("/api/peers", get(handle_get_peers))
        .route("/api/qr", get(handle_get_qr))
        .route("/api/events", get(handle_get_events))
        .route("/api/decision", post(handle_post_decision))
        .route("/api/settings", post(handle_post_settings))
        .route("/api/web_send", post(handle_post_web_send))
        .route("/api/offer", post(handle_post_offer))
        .route("/api/message", post(handle_post_message))
        .route(
            "/api/upload/{session_id}/{file_id}",
            put(handle_put_upload)
                .head(handle_get_upload_status)
                .get(handle_get_upload_status),
        )
        .layer(axum::middleware::from_fn(private_ip_guard))
        .with_state(state)
}

async fn private_ip_guard(
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, (StatusCode, &'static str)> {
    if !is_private_or_loopback_ip(&addr.ip()) {
        warn!(
            "🚨 Blocked unauthorized request from non-private IP: {}",
            addr.ip()
        );
        return Err((
            StatusCode::FORBIDDEN,
            "Access denied: Drop It accepts local network connections only",
        ));
    }
    Ok(next.run(req).await)
}

pub async fn run_server(
    state: ServerState,
    bind_addr: SocketAddr,
    shutdown_rx: Option<broadcast::Receiver<()>>,
) -> Result<()> {
    crypto::init_crypto_provider();
    let rustls_config = RustlsConfig::from_pem(
        state.identity.cert_pem.as_bytes().to_vec(),
        state.identity.key_pem.as_bytes().to_vec(),
    )
    .await
    .context("Failed to build RustlsConfig for HTTPS server")?;

    let app = build_router(state).await;

    info!("Starting Drop It TLS 1.3 server on https://{}", bind_addr);

    let handle = axum_server::Handle::new();
    let handle_clone = handle.clone();

    if let Some(mut rx) = shutdown_rx {
        tokio::spawn(async move {
            let _ = rx.recv().await;
            info!("Gracefully stopping Drop It server...");
            handle_clone.graceful_shutdown(Some(Duration::from_secs(3)));
        });
    }

    axum_server::bind_rustls(bind_addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("Server execution failure")?;

    Ok(())
}

async fn handle_get_index() -> impl IntoResponse {
    Html(EMBEDDED_UI)
}

async fn handle_get_info(
    State(state): State<ServerState>,
    _headers: HeaderMap,
) -> impl IntoResponse {
    let config = state.config.read().await;
    let payload = serde_json::json!({
        "device_id": config.device_id,
        "name": config.device_name,
        "fingerprint": state.identity.fingerprint,
        "port": config.port,
        "download_dir": config.download_dir.to_string_lossy(),
        "auto_accept_trusted": config.auto_accept_trusted,
        "verify_hash": config.verify_hash,
        "version": env!("CARGO_PKG_VERSION"),
    });
    Json(payload)
}

async fn handle_get_peers(State(state): State<ServerState>) -> impl IntoResponse {
    let peer_list = if let Some(ref reg) = state.peer_registry {
        reg.get_active_peer_infos().await
    } else {
        vec![]
    };
    Json(serde_json::json!({ "peers": peer_list }))
}

async fn handle_get_qr(State(state): State<ServerState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let port = config.port;

    // Detect primary local IPv4 address
    let mut lan_ip = "127.0.0.1".to_string();
    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in interfaces {
            if let IpAddr::V4(ipv4) = ip
                && !ipv4.is_loopback()
                && !ipv4.is_link_local()
            {
                lan_ip = ipv4.to_string();
                break;
            }
        }
    }

    let url = format!("https://{lan_ip}:{port}");
    let svg = crypto::generate_qr_svg(&url).unwrap_or_else(|_| "<svg></svg>".into());

    ([(header::CONTENT_TYPE, "image/svg+xml")], svg)
}

async fn handle_get_events(
    State(state): State<ServerState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(32);

    // Initial peers snapshot
    if let Some(ref reg) = state.peer_registry {
        let peers = reg.get_active_peer_infos().await;
        if let Ok(json_str) = serde_json::to_string(&peers) {
            let _ = tx
                .send(Ok(Event::default().event("peers").data(json_str)))
                .await;
        }
    }

    // Subscribe to SSE broadcast
    let mut sse_rx = state.sse_broadcast_tx.subscribe();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        while let Ok(evt) = sse_rx.recv().await {
            let event = match evt {
                WebSseEvent::Peers(val) => Event::default().event("peers").data(val.to_string()),
                WebSseEvent::Offer {
                    decision_id,
                    sender_name,
                    sender_device_id,
                    sender_fingerprint,
                    pairing_code,
                    files,
                    total_bytes,
                } => {
                    let json = serde_json::json!({
                        "decision_id": decision_id,
                        "sender_name": sender_name,
                        "sender_device_id": sender_device_id,
                        "sender_fingerprint": sender_fingerprint,
                        "pairing_code": pairing_code,
                        "files": files,
                        "total_bytes": total_bytes,
                    });
                    Event::default().event("offer").data(json.to_string())
                }
                WebSseEvent::Progress {
                    transfer_id,
                    percent,
                    loaded,
                    total,
                    speed_mb_s,
                } => {
                    let json = serde_json::json!({
                        "transfer_id": transfer_id,
                        "percent": percent,
                        "loaded": loaded,
                        "total": total,
                        "speed_mb_s": speed_mb_s,
                    });
                    Event::default().event("progress").data(json.to_string())
                }
                WebSseEvent::FileCompleted {
                    session_id,
                    file_id,
                    dest_path,
                } => {
                    let json = serde_json::json!({
                        "session_id": session_id,
                        "file_id": file_id,
                        "dest_path": dest_path,
                    });
                    Event::default()
                        .event("file_completed")
                        .data(json.to_string())
                }
                WebSseEvent::Message {
                    sender_id,
                    sender_name,
                    text,
                } => {
                    let json = serde_json::json!({
                        "sender_id": sender_id,
                        "sender_name": sender_name,
                        "text": text,
                    });
                    Event::default().event("message").data(json.to_string())
                }
            };
            if tx_clone.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    // Also pipe registry updates if available
    if let Some(ref reg) = state.peer_registry {
        let mut reg_rx = reg.subscribe();
        let reg_ref = reg.clone();
        let tx_reg = tx.clone();
        tokio::spawn(async move {
            while let Ok(_evt) = reg_rx.recv().await {
                let peers = reg_ref.get_active_peer_infos().await;
                if let Ok(json_str) = serde_json::to_string(&peers) {
                    let event = Event::default().event("peers").data(json_str);
                    if tx_reg.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

async fn handle_post_decision(
    State(state): State<ServerState>,
    Json(payload): Json<DecisionPayload>,
) -> impl IntoResponse {
    let mut pending = state.pending_web_decisions.write().await;
    if let Some(tx) = pending.remove(&payload.decision_id) {
        let _ = tx.send(payload.accepted);
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "status": "expired_or_not_found" })),
        )
    }
}

async fn handle_post_message(
    State(state): State<ServerState>,
    Json(payload): Json<QuickMessagePayload>,
) -> impl IntoResponse {
    let _ = state.sse_broadcast_tx.send(WebSseEvent::Message {
        sender_id: payload.sender_id,
        sender_name: payload.sender_name,
        text: payload.text,
    });
    StatusCode::OK
}

async fn handle_post_settings(
    State(state): State<ServerState>,
    Json(payload): Json<UpdateSettingsPayload>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut config = state.config.write().await;
    if let Some(name) = payload.name
        && !name.trim().is_empty()
    {
        config.device_name = name.trim().to_string();
    }
    if let Some(auto) = payload.auto_accept_trusted {
        config.auto_accept_trusted = auto;
    }
    if let Some(vh) = payload.verify_hash {
        config.verify_hash = vh;
    }
    config
        .save()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "success": true })))
}

async fn handle_post_web_send(
    State(state): State<ServerState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut target = String::new();
    let mut temp_files = Vec::new();
    let temp_dir = std::env::temp_dir().join(format!("dropit_web_{}", rand::random::<u32>()));
    let _ = tokio::fs::create_dir_all(&temp_dir).await;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "target" {
            target = field.text().await.unwrap_or_default();
        } else if name == "files" {
            let filename = field.file_name().unwrap_or("file.bin").to_string();
            let dest = temp_dir.join(&filename);
            let mut f = File::create(&dest)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let data = field
                .bytes()
                .await
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            f.write_all(&data)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            temp_files.push((dest, filename, data.len() as u64));
        }
    }

    if target.is_empty() || temp_files.is_empty() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err((StatusCode::BAD_REQUEST, "Target and files required".into()));
    }

    // Initiate transfer to target device via DropItClient
    let client = DropItClient::new();
    let (local_device_id, local_name) = {
        let cfg = state.config.read().await;
        (cfg.device_id.clone(), cfg.device_name.clone())
    };

    let session_id = hex::encode(rand::random::<[u8; 16]>());
    let mut file_metas = Vec::new();

    for (idx, (path, name, size)) in temp_files.iter().enumerate() {
        let hash = blake3::hash(&tokio::fs::read(path).await.unwrap_or_default())
            .to_hex()
            .to_string();
        file_metas.push(FileMetadata {
            file_id: idx.to_string(),
            filename: name.clone(),
            size_bytes: *size,
            blake3_hash: Some(hash),
        });
    }

    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: local_name,
        sender_device_id: local_device_id,
        sender_fingerprint: state.identity.fingerprint.clone(),
        files: file_metas.clone(),
    };

    let offer_resp = client
        .send_offer(&target, &offer_req)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !offer_resp.accepted {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err((
            StatusCode::FORBIDDEN,
            "Transfer declined by target device".into(),
        ));
    }

    let token = offer_resp
        .session_token
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "Missing session token".into()))?;

    for (idx, meta) in file_metas.iter().enumerate() {
        let (file_path, _, _) = &temp_files[idx];
        client
            .stream_file(&target, &session_id, &token, meta, file_path, |_p| {})
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "files_sent": file_metas.len(),
    })))
}

async fn handle_post_offer(
    State(state): State<ServerState>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<SocketAddr>,
    Json(offer): Json<TransferOfferRequest>,
) -> Result<Json<TransferOfferResponse>, (StatusCode, String)> {
    // 1. Boundary check: Private IP only
    if !is_private_or_loopback_ip(&peer_addr.ip()) {
        warn!(
            "Rejected transfer offer from public/unauthorized IP: {}",
            peer_addr
        );
        return Err((
            StatusCode::FORBIDDEN,
            "Access denied: Private network only".into(),
        ));
    }

    // 2. Rate-limit check: at most 1 offer every 200ms per IP
    {
        let mut recent = state.recent_offers.write().await;
        let now = Instant::now();
        if let Some(prev) = recent.get(&peer_addr.ip())
            && now.duration_since(*prev) < Duration::from_millis(200)
        {
            return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".into()));
        }
        recent.insert(peer_addr.ip(), now);
    }

    // 3. Concurrent sessions cap & expired cleanup
    {
        let mut sessions = state.active_sessions.write().await;
        sessions.retain(|_, s| s.created_at.elapsed() < SESSION_TTL);
        if sessions.len() >= MAX_CONCURRENT_SESSIONS {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Server busy: too many active transfer sessions".into(),
            ));
        }
    }

    // 4. Validate offer payload & security limits
    if offer.files.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "File list cannot be empty".into()));
    }
    if offer.files.len() > MAX_FILES_PER_OFFER {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Too many files in offer (maximum {MAX_FILES_PER_OFFER})"),
        ));
    }
    let mut total_bytes = 0u64;
    for file in &offer.files {
        if file.filename.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Filename cannot be empty".into()));
        }
        if file.size_bytes > MAX_FILE_SIZE {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("File '{}' exceeds maximum allowed size", file.filename),
            ));
        }
        total_bytes = total_bytes.saturating_add(file.size_bytes);
    }
    if total_bytes > MAX_OFFER_TOTAL_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Total offer size exceeds maximum allowed transfer size".into(),
        ));
    }

    // 5. Pairing code & trust verification
    let pairing_code =
        crypto::generate_pairing_code(&state.identity.fingerprint, &offer.sender_fingerprint);
    let config = state.config.read().await;
    let trust_status = config.is_peer_trusted(&offer.sender_device_id, &offer.sender_fingerprint);

    let is_trusted = match trust_status {
        PeerTrustStatus::Trusted => true,
        PeerTrustStatus::FingerprintMismatch {
            ref expected,
            ref actual,
        } => {
            warn!(
                "🚨 SECURITY WARNING: Pinned fingerprint mismatch for device '{}'! Expected {}, got {}",
                offer.sender_name, expected, actual
            );
            return Err((
                StatusCode::CONFLICT,
                format!("Security alert: Pinned device fingerprint mismatch! Expected {expected}"),
            ));
        }
        PeerTrustStatus::Untrusted => false,
    };

    // 6. Acceptance decision: Auto-accept > Web UI prompt > CLI prompt
    let has_web_client = state.sse_broadcast_tx.receiver_count() > 0;
    let has_cli_prompt = state.offer_prompt_tx.is_some();

    let accepted = if is_trusted && config.auto_accept_trusted {
        info!(
            "Auto-accepted transfer offer from trusted peer '{}'",
            offer.sender_name
        );
        true
    } else if !has_web_client && !has_cli_prompt {
        config.auto_accept_trusted
    } else {
        // Broadcast offer to Web UI via SSE
        let decision_id = hex::encode(rand::random::<[u8; 8]>());
        let (web_tx, web_rx) = oneshot::channel();
        let total_bytes: u64 = offer.files.iter().map(|f| f.size_bytes).sum();

        {
            let mut pending = state.pending_web_decisions.write().await;
            pending.insert(decision_id.clone(), web_tx);
        }

        let _ = state.sse_broadcast_tx.send(WebSseEvent::Offer {
            decision_id: decision_id.clone(),
            sender_name: offer.sender_name.clone(),
            sender_device_id: offer.sender_device_id.clone(),
            sender_fingerprint: offer.sender_fingerprint.clone(),
            pairing_code: pairing_code.clone(),
            files: offer.files.clone(),
            total_bytes,
        });

        // If CLI receiver prompt channel is registered, run both in select
        if let Some(ref prompt_tx) = state.offer_prompt_tx {
            let (cli_tx, cli_rx) = oneshot::channel();
            let prompt_req = OfferDecisionRequest {
                offer: offer.clone(),
                pairing_code: pairing_code.clone(),
                is_trusted,
                response_tx: cli_tx,
            };
            let _ = prompt_tx.send(prompt_req).await;

            tokio::select! {
                web_res = web_rx => {
                    web_res.unwrap_or(false)
                }
                cli_res = cli_rx => {
                    cli_res.unwrap_or(false)
                }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    false
                }
            }
        } else {
            tokio::time::timeout(Duration::from_secs(60), web_rx)
                .await
                .unwrap_or(Ok(false))
                .unwrap_or(false)
        }
    };

    if !accepted {
        return Ok(Json(TransferOfferResponse {
            accepted: false,
            session_token: None,
            pairing_code,
            reason: Some("Transfer declined by receiver".into()),
        }));
    }

    // 7. Initialize active session & destination paths
    let session_token = crypto::generate_session_token();
    let download_dir = config.download_dir.clone();
    drop(config); // release lock

    let mut dest_paths = HashMap::new();
    for file in &offer.files {
        let safe_dest = resolve_destination_path(&download_dir, &file.filename, file.size_bytes);
        dest_paths.insert(file.file_id.clone(), safe_dest);
    }

    let total_offer_bytes: u64 = offer.files.iter().map(|f| f.size_bytes).sum();
    let active_session = ActiveSession {
        session_id: offer.session_id.clone(),
        session_token: session_token.clone(),
        sender_name: offer.sender_name.clone(),
        sender_device_id: offer.sender_device_id.clone(),
        sender_fingerprint: offer.sender_fingerprint.clone(),
        files: offer.files.clone(),
        dest_paths,
        completed_files: HashSet::new(),
        total_bytes: total_offer_bytes,
        received_bytes: 0,
        created_at: Instant::now(),
    };

    {
        let mut sessions = state.active_sessions.write().await;
        sessions.insert(offer.session_id.clone(), active_session);
    }

    Ok(Json(TransferOfferResponse {
        accepted: true,
        session_token: Some(session_token),
        pairing_code,
        reason: None,
    }))
}

fn parse_range_start(headers: &HeaderMap) -> Option<u64> {
    for header_name in &[&header::CONTENT_RANGE, &header::RANGE] {
        if let Some(val) = headers.get(*header_name).and_then(|h| h.to_str().ok()) {
            let trimmed = val.trim();
            let after_bytes = if let Some(s) = trimmed.strip_prefix("bytes=") {
                Some(s)
            } else {
                trimmed.strip_prefix("bytes ")
            };
            if let Some(range_part) = after_bytes {
                let start_str = range_part.split('-').next()?;
                if let Ok(start) = start_str.trim().parse::<u64>() {
                    return Some(start);
                }
            }
        }
    }
    None
}

async fn handle_get_upload_status(
    State(state): State<ServerState>,
    AxumPath((session_id, file_id)): AxumPath<(String, String)>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !is_private_or_loopback_ip(&peer_addr.ip()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Access denied: Private network only".into(),
        ));
    }

    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Missing or invalid Authorization header".into(),
            )
        })?;

    let dest_path = {
        let sessions = state.active_sessions.read().await;
        let session = sessions.get(&session_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Session '{session_id}' not found or expired"),
            )
        })?;

        if session.created_at.elapsed() >= SESSION_TTL {
            return Err((
                StatusCode::UNAUTHORIZED,
                format!("Session '{session_id}' has expired"),
            ));
        }

        if session.session_token != auth_header {
            return Err((StatusCode::FORBIDDEN, "Invalid session token".into()));
        }

        session.dest_paths.get(&file_id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("File id '{file_id}' not found in session"),
            )
        })?
    };

    let existing_bytes = if dest_path.exists() {
        tokio::fs::metadata(&dest_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    resp_headers.insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from(existing_bytes),
    );

    Ok((resp_headers, ""))
}

async fn handle_put_upload(
    State(state): State<ServerState>,
    AxumPath((session_id, file_id)): AxumPath<(String, String)>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<UploadResult>, (StatusCode, String)> {
    // 1. Boundary check: Private IP only
    if !is_private_or_loopback_ip(&peer_addr.ip()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Access denied: Private network only".into(),
        ));
    }

    // 2. Authorization Bearer check
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Missing or invalid Authorization header".into(),
            )
        })?;

    // 3. Session lookup & verification
    let (dest_path, expected_hash, sender_device_id, sender_fingerprint, session_total_bytes) = {
        let sessions = state.active_sessions.read().await;
        let session = sessions.get(&session_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Session '{session_id}' not found or expired"),
            )
        })?;

        if session.created_at.elapsed() >= SESSION_TTL {
            return Err((
                StatusCode::UNAUTHORIZED,
                format!("Session '{session_id}' has expired"),
            ));
        }

        if session.session_token != auth_header {
            return Err((StatusCode::FORBIDDEN, "Invalid session token".into()));
        }

        let path = session.dest_paths.get(&file_id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("File id '{file_id}' not found in session"),
            )
        })?;

        let meta = session
            .files
            .iter()
            .find(|f| f.file_id == file_id)
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("File metadata for '{file_id}' not found"),
                )
            })?;

        (
            path,
            meta.blake3_hash,
            session.sender_device_id.clone(),
            session.sender_fingerprint.clone(),
            session.total_bytes,
        )
    };

    // 4. Check for HTTP Range resume
    let range_start = parse_range_start(&headers).unwrap_or(0);

    let mut file = if range_start > 0 {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&dest_path)
            .await
            .map_err(|err| {
                error!(
                    "Failed to open destination file for resume {:?}: {}",
                    dest_path, err
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to open file for resume: {err}"),
                )
            })?;

        let current_len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
        if range_start > current_len {
            return Err((
                StatusCode::RANGE_NOT_SATISFIABLE,
                format!(
                    "Requested range start {range_start} exceeds existing file size {current_len}"
                ),
            ));
        }

        f.seek(SeekFrom::Start(range_start)).await.map_err(|err| {
            error!("Failed to seek destination file {:?}: {}", dest_path, err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to seek file: {err}"),
            )
        })?;
        f
    } else {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&dest_path)
            .await
            .map_err(|err| {
                error!("Failed to create destination file {:?}: {}", dest_path, err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to create file: {err}"),
                )
            })?
    };

    // 5. Stream socket -> disk in 1 MB buffer chunks (NEVER load whole files into RAM!)
    let mut body_stream = body.into_data_stream();
    let mut hasher = blake3::Hasher::new();
    let mut bytes_written = 0u64;
    let stream_start = Instant::now();
    let mut last_progress_time = Instant::now();
    let mut session_current_loaded = {
        let sessions = state.active_sessions.read().await;
        sessions
            .get(&session_id)
            .map(|s| s.received_bytes)
            .unwrap_or(0)
    };

    while let Some(chunk_res) = body_stream.next().await {
        let chunk = chunk_res.map_err(|err| {
            error!(
                "Streaming error while receiving file {:?}: {}",
                dest_path, err
            );
            (StatusCode::BAD_REQUEST, format!("Streaming error: {err}"))
        })?;

        if range_start == 0 {
            hasher.update(&chunk);
        }
        file.write_all(&chunk).await.map_err(|err| {
            error!("Disk write error on {:?}: {}", dest_path, err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Disk write error: {err}"),
            )
        })?;

        let chunk_len = chunk.len() as u64;
        bytes_written += chunk_len;
        session_current_loaded += chunk_len;

        if last_progress_time.elapsed() >= Duration::from_millis(80) {
            last_progress_time = Instant::now();
            let elapsed_secs = stream_start.elapsed().as_secs_f64();
            let speed_mb_s = if elapsed_secs > 0.05 {
                (bytes_written as f64 / (1024.0 * 1024.0)) / elapsed_secs
            } else {
                0.0
            };
            let percent = if session_total_bytes > 0 {
                ((session_current_loaded as f64 / session_total_bytes as f64) * 100.0)
                    .clamp(0.0, 99.9)
            } else {
                50.0
            };

            let _ = state.sse_broadcast_tx.send(WebSseEvent::Progress {
                transfer_id: session_id.clone(),
                percent,
                loaded: session_current_loaded,
                total: session_total_bytes,
                speed_mb_s,
            });
        }
    }

    file.flush().await.map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Disk flush error: {err}"),
        )
    })?;
    drop(file);

    // 6. BLAKE3 hash computation & verification
    let computed_hash = if range_start > 0 {
        // Stream file on disk in 1 MB chunks to compute final BLAKE3 hash without buffering in RAM
        crypto::hash_file_streaming(&dest_path)
            .await
            .map_err(|err| {
                error!("Failed to hash resumed file {:?}: {}", dest_path, err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to compute file hash: {err}"),
                )
            })?
    } else {
        hasher.finalize().to_hex().to_string()
    };

    let verified = if let Some(ref expected) = expected_hash {
        expected.eq_ignore_ascii_case(&computed_hash)
    } else {
        true
    };

    if !verified {
        warn!("BLAKE3 hash verification failed for file {:?}", dest_path);
        let _ = tokio::fs::remove_file(&dest_path).await;
        return Err((StatusCode::BAD_REQUEST, "BLAKE3 hash mismatch".into()));
    }

    info!(
        "Successfully received {:?} ({} bytes streamed, total: {}, blake3: {})",
        dest_path,
        bytes_written,
        range_start + bytes_written,
        computed_hash
    );

    // 7. Update session & check completion
    let (session_all_done, final_loaded, final_total) = {
        let mut sessions = state.active_sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.received_bytes += bytes_written;
            session.completed_files.insert(file_id.clone());
            let all_done = session.is_completed();
            (all_done, session.received_bytes, session.total_bytes)
        } else {
            (true, bytes_written, session_total_bytes)
        }
    };

    // Emit final progress and file completion events
    let _ = state.sse_broadcast_tx.send(WebSseEvent::Progress {
        transfer_id: session_id.clone(),
        percent: if session_all_done {
            100.0
        } else {
            ((final_loaded as f64 / final_total.max(1) as f64) * 100.0).clamp(0.0, 99.9)
        },
        loaded: final_loaded,
        total: final_total,
        speed_mb_s: 0.0,
    });

    let _ = state.sse_broadcast_tx.send(WebSseEvent::FileCompleted {
        session_id: session_id.clone(),
        file_id: file_id.clone(),
        dest_path: dest_path.to_string_lossy().to_string(),
    });

    if session_all_done {
        // Pin fingerprint on first completed transfer (TOFU)
        let mut config = state.config.write().await;
        let _ = config.trust_peer(&sender_device_id, &sender_fingerprint);
        info!(
            "All files received! Pinned peer '{}' ({})",
            sender_device_id, sender_fingerprint
        );

        let mut sessions = state.active_sessions.write().await;
        sessions.remove(&session_id);
    }

    Ok(Json(UploadResult {
        file_id,
        bytes_written: range_start + bytes_written,
        blake3_hash: computed_hash,
        verified,
    }))
}
