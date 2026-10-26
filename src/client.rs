use crate::transfer::{
    FileMetadata, FileTransferProgress, TransferOfferRequest, TransferOfferResponse, UploadResult,
};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, HeaderMap, HeaderValue};
use reqwest::{Body, Client};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB chunk buffer

pub struct DropItClient {
    client: Client,
}

impl Default for DropItClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DropItClient {
    pub fn new() -> Self {
        crate::crypto::init_crypto_provider();
        let client = Client::builder()
            .danger_accept_invalid_certs(true) // Self-signed TLS 1.3 certs
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    pub async fn get_device_info(&self, host_port: &str) -> Result<serde_json::Value> {
        let url = format!("https://{host_port}/api/info");
        let res = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to connect to https://{host_port}"))?;

        if !res.status().is_success() {
            bail!("Failed to get device info: HTTP {}", res.status());
        }

        let info = res.json::<serde_json::Value>().await?;
        Ok(info)
    }

    pub async fn send_offer(
        &self,
        host_port: &str,
        offer: &TransferOfferRequest,
    ) -> Result<TransferOfferResponse> {
        let url = format!("https://{host_port}/api/offer");
        let res = self
            .client
            .post(&url)
            .json(offer)
            .send()
            .await
            .with_context(|| format!("Failed to send transfer offer to {host_port}"))?;

        if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!("Rate limit exceeded on target device");
        } else if res.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            bail!("Target device is busy with too many active transfers");
        } else if !res.status().is_success() {
            let error_text = res.text().await.unwrap_or_default();
            bail!("Offer rejected by target device: {error_text}");
        }

        let resp = res.json::<TransferOfferResponse>().await?;
        Ok(resp)
    }

    /// Streams a file directly from disk to socket with 1 MB chunks and live progress updates.
    /// NEVER loads the whole file into RAM!
    pub async fn stream_file<F>(
        &self,
        host_port: &str,
        session_id: &str,
        session_token: &str,
        meta: &FileMetadata,
        file_path: &Path,
        on_progress: F,
    ) -> Result<UploadResult>
    where
        F: FnMut(FileTransferProgress) + Send + 'static,
    {
        let file = File::open(file_path)
            .await
            .with_context(|| format!("Failed to open file {:?}", file_path))?;
        let total_bytes = meta.size_bytes;

        let url = format!(
            "https://{host_port}/api/upload/{session_id}/{}",
            meta.file_id
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {session_token}"))?,
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from(total_bytes));

        let start_time = Instant::now();
        let bytes_transferred = Arc::new(AtomicU64::new(0));

        let stream = ReaderStream::with_capacity(file, CHUNK_SIZE);
        let bytes_counter = Arc::clone(&bytes_transferred);
        let file_id_for_stream = meta.file_id.clone();
        let filename_for_stream = meta.filename.clone();
        let progress_callback = Arc::new(Mutex::new(on_progress));

        // Wrap stream in a progress-tracking stream
        let progress_stream = stream.map(move |chunk_res| {
            if let Ok(ref chunk) = chunk_res {
                let current = bytes_counter.fetch_add(chunk.len() as u64, Ordering::Relaxed)
                    + chunk.len() as u64;

                let elapsed_secs = start_time.elapsed().as_secs_f64();
                let speed_mb_s = if elapsed_secs > 0.05 {
                    (current as f64 / (1024.0 * 1024.0)) / elapsed_secs
                } else {
                    0.0
                };

                let remaining_bytes = total_bytes.saturating_sub(current);
                let bytes_per_sec = if elapsed_secs > 0.05 {
                    current as f64 / elapsed_secs
                } else {
                    1.0
                };
                let eta_secs = if bytes_per_sec > 0.0 {
                    Some((remaining_bytes as f64 / bytes_per_sec) as u64)
                } else {
                    None
                };

                if let Ok(mut cb) = progress_callback.lock() {
                    cb(FileTransferProgress {
                        file_id: file_id_for_stream.clone(),
                        filename: filename_for_stream.clone(),
                        bytes_transferred: current,
                        total_bytes,
                        speed_mb_s,
                        eta_secs,
                        completed: current >= total_bytes,
                        error: None,
                    });
                }
            }
            chunk_res
        });

        let body = Body::wrap_stream(progress_stream);

        let res = self
            .client
            .put(&url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .with_context(|| format!("Streaming upload failed to {url}"))?;

        if !res.status().is_success() {
            let status = res.status();
            let error_text = res.text().await.unwrap_or_default();
            bail!("Upload failed (HTTP {}): {error_text}", status);
        }

        let upload_result = res.json::<UploadResult>().await?;
        Ok(upload_result)
    }
}
