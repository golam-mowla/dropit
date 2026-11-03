use crate::pairing::{DeviceTrustStore, PairingError, SessionTokenStore, TrustStatus};
use crate::sanitize::safe_destination_path;
use crate::tls::TlsIdentity;
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Multipart, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
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

pub fn create_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/info", get(info_handler))
        .route("/api/token", post(token_handler))
        .route("/api/pairing/code", post(pairing_code_handler))
        .route("/api/transfer", post(transfer_handler))
        .with_state(Arc::new(state))
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
    connect_info: Option<ConnectInfo<SocketAddr>>,
    Json(payload): Json<PairingCodeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let client_ip = connect_info
        .map(|ci| ci.0.ip())
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
            // Verify disk space before writing chunk
            if let Err(e) =
                crate::pairing::verify_disk_space(&state.download_dir, chunk.len() as u64)
            {
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
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("Server error")?;

    Ok(())
}
