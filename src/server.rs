use crate::config::{DropItConfig, PeerTrustStatus};
use crate::crypto::{self, TlsIdentity};
use crate::security::{is_private_or_loopback_ip, resolve_destination_path};
use crate::transfer::{ActiveSession, TransferOfferRequest, TransferOfferResponse, UploadResult};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info, warn};

pub const BUFFER_SIZE: usize = 1024 * 1024; // 1 MB buffer chunk
const MAX_CONCURRENT_SESSIONS: usize = 8;
const SESSION_TTL: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Clone)]
pub struct ServerState {
    pub config: Arc<RwLock<DropItConfig>>,
    pub identity: TlsIdentity,
    pub active_sessions: Arc<RwLock<HashMap<String, ActiveSession>>>,
    pub recent_offers: Arc<RwLock<HashMap<IpAddr, Instant>>>,
    pub offer_prompt_tx: Option<mpsc::Sender<OfferDecisionRequest>>,
}

pub struct OfferDecisionRequest {
    pub offer: TransferOfferRequest,
    pub pairing_code: String,
    pub is_trusted: bool,
    pub response_tx: tokio::sync::oneshot::Sender<bool>,
}

impl ServerState {
    pub fn new(
        config: Arc<RwLock<DropItConfig>>,
        identity: TlsIdentity,
        offer_prompt_tx: Option<mpsc::Sender<OfferDecisionRequest>>,
    ) -> Self {
        Self {
            config,
            identity,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            recent_offers: Arc::new(RwLock::new(HashMap::new())),
            offer_prompt_tx,
        }
    }
}

pub async fn build_router(state: ServerState) -> Router {
    Router::new()
        .route("/api/info", get(handle_get_info))
        .route("/api/offer", post(handle_post_offer))
        .route("/api/upload/{session_id}/{file_id}", put(handle_put_upload))
        .with_state(state)
}

pub async fn run_server(
    state: ServerState,
    bind_addr: SocketAddr,
    shutdown_rx: Option<tokio::sync::broadcast::Receiver<()>>,
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
        "version": env!("CARGO_PKG_VERSION"),
    });
    Json(payload)
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

    // 4. Validate offer payload
    if offer.files.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "File list cannot be empty".into()));
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

    // 6. Acceptance decision
    let accepted = if is_trusted && config.auto_accept_trusted {
        info!(
            "Auto-accepted transfer offer from trusted peer '{}'",
            offer.sender_name
        );
        true
    } else if let Some(ref prompt_tx) = state.offer_prompt_tx {
        let (res_tx, res_rx) = tokio::sync::oneshot::channel();
        let prompt_req = OfferDecisionRequest {
            offer: offer.clone(),
            pairing_code: pairing_code.clone(),
            is_trusted,
            response_tx: res_tx,
        };
        if prompt_tx.send(prompt_req).await.is_err() {
            false
        } else {
            tokio::time::timeout(Duration::from_secs(60), res_rx)
                .await
                .unwrap_or(Ok(false))
                .unwrap_or(false)
        }
    } else {
        // Fallback default: accept if receiver mode was started with auto-accept
        config.auto_accept_trusted
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
        let safe_dest = resolve_destination_path(&download_dir, &file.filename);
        dest_paths.insert(file.file_id.clone(), safe_dest);
    }

    let active_session = ActiveSession {
        session_id: offer.session_id.clone(),
        session_token: session_token.clone(),
        sender_name: offer.sender_name.clone(),
        sender_device_id: offer.sender_device_id.clone(),
        sender_fingerprint: offer.sender_fingerprint.clone(),
        files: offer.files.clone(),
        dest_paths,
        completed_files: HashSet::new(),
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
    let (dest_path, expected_hash, sender_device_id, sender_fingerprint) = {
        let sessions = state.active_sessions.read().await;
        let session = sessions.get(&session_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Session '{session_id}' not found or expired"),
            )
        })?;

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
        )
    };

    // 4. Stream socket -> disk in 1 MB buffer chunks (NEVER load whole files into RAM!)
    let mut file = File::create(&dest_path).await.map_err(|err| {
        error!("Failed to create destination file {:?}: {}", dest_path, err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create file: {err}"),
        )
    })?;

    let mut body_stream = body.into_data_stream();
    let mut hasher = blake3::Hasher::new();
    let mut bytes_written = 0u64;

    while let Some(chunk_res) = body_stream.next().await {
        let chunk = chunk_res.map_err(|err| {
            error!(
                "Streaming error while receiving file {:?}: {}",
                dest_path, err
            );
            (StatusCode::BAD_REQUEST, format!("Streaming error: {err}"))
        })?;

        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|err| {
            error!("Disk write error on {:?}: {}", dest_path, err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Disk write error: {err}"),
            )
        })?;

        bytes_written += chunk.len() as u64;
    }

    file.flush().await.map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Disk flush error: {err}"),
        )
    })?;

    let computed_hash = hasher.finalize().to_hex().to_string();
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
        "Successfully received {:?} ({} bytes, blake3: {})",
        dest_path, bytes_written, computed_hash
    );

    // 5. Update session & check completion
    let session_all_done = {
        let mut sessions = state.active_sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_id) {
            session.completed_files.insert(file_id.clone());
            session.is_completed()
        } else {
            false
        }
    };

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
        bytes_written,
        blake3_hash: computed_hash,
        verified,
    }))
}
