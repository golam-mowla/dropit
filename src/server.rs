use crate::messenger::{ChatMessage, ChatSessionManager};
use crate::pairing::{DeviceTrustStore, PairingError, SessionTokenStore, TrustStatus};
use crate::sanitize::safe_destination_path;
use crate::tls::TlsIdentity;
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Multipart, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Clone)]
pub struct ServerState {
    pub device_id: String,
    pub name: String,
    pub download_dir: PathBuf,
    pub token_store: SessionTokenStore,
    pub cert_fingerprint: String,
    pub auto_accept: bool,
    pub trust_store: DeviceTrustStore,
    pub pending_transfers: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    pub cancelled_sessions: Arc<RwLock<HashSet<String>>>,
    pub chat_manager: ChatSessionManager,
}

impl ServerState {
    pub fn new(
        device_id: String,
        name: String,
        download_dir: PathBuf,
        token_store: SessionTokenStore,
        cert_fingerprint: String,
        auto_accept: bool,
    ) -> Self {
        Self {
            device_id,
            name,
            download_dir,
            token_store,
            cert_fingerprint,
            auto_accept,
            trust_store: DeviceTrustStore::new(),
            pending_transfers: Arc::new(Mutex::new(HashMap::new())),
            cancelled_sessions: Arc::new(RwLock::new(HashSet::new())),
            chat_manager: ChatSessionManager::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct DeviceInfoResponse {
    pub device_id: String,
    pub name: String,
    pub cert_fingerprint: String,
}

#[derive(Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairingCodeRequest {
    pub code: String,
    pub client_id: String,
    pub client_fingerprint: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairingCodeResponse {
    pub token: String,
    pub server_fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub blake3_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferIntentRequest {
    pub session_id: String,
    pub sender_id: String,
    pub sender_alias: String,
    pub files: Vec<FileMetadata>,
    pub total_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferIntentResponse {
    pub session_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferDecisionRequest {
    pub session_id: String,
    pub accept: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancelTransferRequest {
    pub session_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct TransferSuccessResponse {
    pub files: Vec<TransferredFileInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct TransferredFileInfo {
    pub filename: String,
    pub size: u64,
    pub blake3_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct ChatSendResponse {
    pub status: String,
    pub id: String,
}

#[derive(Deserialize)]
pub struct ChatQuery {
    pub session_id: String,
}

pub fn create_router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(web_ui_handler))
        .route("/web", get(web_ui_handler))
        .route("/health", get(health_handler))
        .route("/api/info", get(info_handler))
        .route("/api/token", post(token_handler))
        .route("/api/pairing/code", post(pairing_code_handler))
        .route("/api/transfer/prepare", post(prepare_transfer_handler))
        .route("/api/transfer/decision", post(decision_handler))
        .route("/api/transfer/cancel", post(cancel_handler))
        .route("/api/transfer", post(transfer_handler))
        .route("/api/chat/message", post(chat_send_handler))
        .route("/api/chat/messages", get(chat_get_handler))
        .with_state(Arc::new(state))
}

async fn web_ui_handler() -> Html<&'static str> {
    Html(crate::web::INDEX_HTML)
}

async fn health_handler() -> &'static str {
    "OK"
}

async fn info_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    Json(DeviceInfoResponse {
        device_id: state.device_id.clone(),
        name: state.name.clone(),
        cert_fingerprint: state.cert_fingerprint.clone(),
    })
}

async fn token_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let token = state.token_store.generate_token();
    Json(TokenResponse { token })
}

async fn pairing_code_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(payload): Json<PairingCodeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    // 1. Verify pairing code with IP rate limiting and 3 strikes lockout
    let pairing_store = state.token_store.pairing_session();
    pairing_store
        .verify_code(&payload.code, client_ip)
        .map_err(|e| match e {
            PairingError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, e.to_string()),
            PairingError::LockedOut => (StatusCode::LOCKED, e.to_string()),
            PairingError::InvalidCode { .. } => (StatusCode::UNAUTHORIZED, e.to_string()),
            PairingError::ExpiredOrNotFound => (StatusCode::NOT_FOUND, e.to_string()),
        })?;

    // 2. TOFU certificate pinning check
    match state
        .trust_store
        .verify_or_pin(&payload.client_id, &payload.client_fingerprint)
    {
        TrustStatus::Trusted | TrustStatus::NewDevicePinned => {}
        TrustStatus::FingerprintMismatch { expected, actual } => {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "Certificate fingerprint mismatch: expected {}, got {}",
                    expected, actual
                ),
            ));
        }
    }

    // 3. Issue single-use session token
    let token = state.token_store.generate_token();
    Ok(Json(PairingCodeResponse {
        token,
        server_fingerprint: state.cert_fingerprint.clone(),
    }))
}

async fn prepare_transfer_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<TransferIntentRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 1. Check disk space
    if let Err(e) = crate::pairing::verify_disk_space(&state.download_dir, payload.total_size) {
        return Ok(Json(TransferIntentResponse {
            session_id: payload.session_id,
            status: "rejected".to_string(),
            reason: Some(format!("Insufficient disk space: {}", e)),
            token: None,
        }));
    }

    // 2. If auto_accept is enabled, approve immediately
    if state.auto_accept {
        let token = state.token_store.generate_token();
        return Ok(Json(TransferIntentResponse {
            session_id: payload.session_id,
            status: "accepted".to_string(),
            reason: None,
            token: Some(token),
        }));
    }

    // 3. Queue for decision with 30s timeout
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut map = state.pending_transfers.lock().await;
        map.insert(payload.session_id.clone(), tx);
    }

    info!(
        "Pending transfer approval for {} files from '{}' (session: {})",
        payload.files.len(),
        payload.sender_alias,
        payload.session_id
    );

    match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(true)) => {
            let token = state.token_store.generate_token();
            Ok(Json(TransferIntentResponse {
                session_id: payload.session_id,
                status: "accepted".to_string(),
                reason: None,
                token: Some(token),
            }))
        }
        Ok(Ok(false)) => Ok(Json(TransferIntentResponse {
            session_id: payload.session_id,
            status: "rejected".to_string(),
            reason: Some("Transfer declined by receiver".to_string()),
            token: None,
        })),
        _ => {
            let mut map = state.pending_transfers.lock().await;
            map.remove(&payload.session_id);
            Ok(Json(TransferIntentResponse {
                session_id: payload.session_id,
                status: "rejected".to_string(),
                reason: Some("Transfer request timed out".to_string()),
                token: None,
            }))
        }
    }
}

async fn decision_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<TransferDecisionRequest>,
) -> impl IntoResponse {
    let mut map = state.pending_transfers.lock().await;
    if let Some(sender) = map.remove(&payload.session_id) {
        let _ = sender.send(payload.accept);
        (StatusCode::OK, "Decision applied")
    } else {
        (
            StatusCode::NOT_FOUND,
            "No pending transfer found for session_id",
        )
    }
}

async fn cancel_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<CancelTransferRequest>,
) -> impl IntoResponse {
    let mut set = state.cancelled_sessions.write().unwrap();
    set.insert(payload.session_id);
    (StatusCode::OK, "Session marked cancelled")
}

async fn chat_send_handler(
    State(state): State<Arc<ServerState>>,
    Json(message): Json<ChatMessage>,
) -> impl IntoResponse {
    let id = message.id.clone();
    state.chat_manager.append_message(message);
    Json(ChatSendResponse {
        status: "delivered".to_string(),
        id,
    })
}

async fn chat_get_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ChatQuery>,
) -> impl IntoResponse {
    let messages = state.chat_manager.get_messages(&query.session_id);
    Json(messages)
}

async fn transfer_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate session token
    let token = headers
        .get("X-Session-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Missing X-Session-Token".to_string(),
            )
        })?;

    if !state.token_store.verify_and_consume(token) {
        return Err((
            StatusCode::FORBIDDEN,
            "Invalid or expired session token".to_string(),
        ));
    }

    // Optional session ID for cancellation checking
    let session_id = headers
        .get("X-Session-ID")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Verify initial disk space
    if let Err(e) = crate::pairing::verify_disk_space(&state.download_dir, 1024) {
        return Err((StatusCode::INSUFFICIENT_STORAGE, e.to_string()));
    }

    tokio::fs::create_dir_all(&state.download_dir)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create download dir: {}", e),
            )
        })?;

    let mut transferred_files = Vec::new();

    while let Ok(Some(mut field)) = multipart.next_field().await {
        // Check for cancellation before processing each file
        if let Some(ref sid) = session_id
            && state.cancelled_sessions.read().unwrap().contains(sid)
        {
            return Err((
                StatusCode::GONE,
                "Transfer cancelled by sender or receiver".to_string(),
            ));
        }

        let raw_filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unnamed_file".to_string());

        let target_path = safe_destination_path(&state.download_dir, &raw_filename);
        let filename = target_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed_file")
            .to_string();

        let mut file = tokio::fs::File::create(&target_path).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create file: {}", e),
            )
        })?;

        let mut hasher = blake3::Hasher::new();
        let mut total_bytes = 0u64;

        while let Ok(Some(chunk)) = field.chunk().await {
            // Check cancellation mid-stream
            if let Some(ref sid) = session_id
                && state.cancelled_sessions.read().unwrap().contains(sid)
            {
                drop(file);
                let _ = tokio::fs::remove_file(&target_path).await;
                return Err((
                    StatusCode::GONE,
                    "Transfer cancelled mid-stream".to_string(),
                ));
            }

            // Verify disk space before writing chunk
            if let Err(e) =
                crate::pairing::verify_disk_space(&state.download_dir, chunk.len() as u64)
            {
                drop(file);
                let _ = tokio::fs::remove_file(&target_path).await;
                return Err((StatusCode::INSUFFICIENT_STORAGE, e.to_string()));
            }

            total_bytes += chunk.len() as u64;
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Write error: {}", e),
                )
            })?;
        }

        file.flush().await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Flush error: {}", e),
            )
        })?;

        let hash_hex = hasher.finalize().to_hex().to_string();
        info!(
            "Successfully received file '{}' ({} bytes, hash: {})",
            filename, total_bytes, hash_hex
        );

        // Record transferred file in chat thread as a FileBubble
        if let Some(ref sid) = session_id {
            let bubble = ChatMessage::new(
                sid,
                &state.device_id,
                &state.name,
                crate::messenger::MessagePayload::FileBubble {
                    file_id: format!("{:016x}", rand::random::<u64>()),
                    filename: filename.clone(),
                    size: total_bytes,
                    blake3_hash: hash_hex.clone(),
                },
            );
            state.chat_manager.append_message(bubble);
        }

        transferred_files.push(TransferredFileInfo {
            filename,
            size: total_bytes,
            blake3_hash: hash_hex,
        });
    }

    Ok(Json(TransferSuccessResponse {
        files: transferred_files,
    }))
}

/// Runs the Drop It TLS server on the given address until cancelled.
pub async fn run_server(
    addr: std::net::SocketAddr,
    state: ServerState,
    tls_identity: &TlsIdentity,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    crate::tls::init_crypto_provider();
    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem(
        tls_identity.cert_pem.as_bytes().to_vec(),
        tls_identity.key_pem.as_bytes().to_vec(),
    )
    .await
    .context("Failed to build RustlsConfig")?;

    let app = create_router(state);
    let handle = axum_server::Handle::new();
    let handle_clone = handle.clone();

    tokio::spawn(async move {
        let _ = shutdown_rx.changed().await;
        info!("Shutting down TLS server");
        handle_clone.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
    });

    info!("Starting Drop It TLS server on {}", addr);
    axum_server::bind_rustls(addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("Server error")?;

    Ok(())
}
