use crate::transfer::{
    FileMetadata, FileTransferProgress, TransferOfferRequest, TransferOfferResponse, UploadResult,
};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, HeaderMap, HeaderValue};
use reqwest::{Body, Client};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncSeekExt;
use tokio_util::io::ReaderStream;

pub const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB chunk buffer
pub const BIG_FILE_THRESHOLD: u64 = 16 * 1024 * 1024; // 16 MB: >= 16 MB uses 1 fast stream; < 16 MB uses 4 parallel streams

#[derive(Clone, Debug)]
pub struct UploadSession<'a> {
    pub host_port: &'a str,
    pub session_id: &'a str,
    pub session_token: &'a str,
}

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

    pub async fn send_quick_message(
        &self,
        host_port: &str,
        sender_id: &str,
        sender_name: &str,
        sender_port: u16,
        sender_fingerprint: &str,
        text: &str,
    ) -> Result<()> {
        let url = format!("https://{host_port}/api/message");
        let payload = serde_json::json!({
            "sender_id": sender_id,
            "sender_name": sender_name,
            "sender_port": sender_port,
            "sender_fingerprint": sender_fingerprint,
            "text": text,
        });
        let res = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to send message to https://{host_port}"))?;

        if !res.status().is_success() {
            bail!("Failed to send message: HTTP {}", res.status());
        }
        Ok(())
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

    /// Checks receiver's current file upload offset for resume support via HEAD
    pub async fn get_upload_status(
        &self,
        host_port: &str,
        session_id: &str,
        session_token: &str,
        file_id: &str,
    ) -> Result<u64> {
        let url = format!("https://{host_port}/api/upload/{session_id}/{file_id}");
        let res = self
            .client
            .head(&url)
            .header(AUTHORIZATION, format!("Bearer {session_token}"))
            .send()
            .await
            .with_context(|| format!("Failed to get upload status from {url}"))?;

        if res.status().is_success() {
            let bytes = res
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|val| val.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            return Ok(bytes);
        }
        Ok(0)
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
        let session = UploadSession {
            host_port,
            session_id,
            session_token,
        };
        self.stream_file_resumed(&session, meta, file_path, 0, on_progress)
            .await
    }

    /// Resumes streaming an upload from a byte offset using HTTP Range headers.
    pub async fn stream_file_resumed<F>(
        &self,
        session: &UploadSession<'_>,
        meta: &FileMetadata,
        file_path: &Path,
        resume_from: u64,
        on_progress: F,
    ) -> Result<UploadResult>
    where
        F: FnMut(FileTransferProgress) + Send + 'static,
    {
        let mut file = File::open(file_path)
            .await
            .with_context(|| format!("Failed to open file {:?}", file_path))?;
        let total_bytes = meta.size_bytes;
        let bytes_to_send = total_bytes.saturating_sub(resume_from);

        if resume_from > 0 {
            file.seek(SeekFrom::Start(resume_from))
                .await
                .with_context(|| format!("Failed to seek to {resume_from} in {:?}", file_path))?;
        }

        let url = format!(
            "https://{}/api/upload/{}/{}",
            session.host_port, session.session_id, meta.file_id
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", session.session_token))?,
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from(bytes_to_send));

        if resume_from > 0 {
            let content_range_val = format!(
                "bytes {}-{}/{}",
                resume_from,
                total_bytes.saturating_sub(1),
                total_bytes
            );
            headers.insert(
                reqwest::header::CONTENT_RANGE,
                HeaderValue::from_str(&content_range_val)?,
            );
            let range_val = format!("bytes={}-{}", resume_from, total_bytes.saturating_sub(1));
            headers.insert(reqwest::header::RANGE, HeaderValue::from_str(&range_val)?);
        }

        let start_time = Instant::now();
        let bytes_transferred = Arc::new(AtomicU64::new(resume_from));

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

                let streamed_so_far = current.saturating_sub(resume_from);
                let elapsed_secs = start_time.elapsed().as_secs_f64();
                let speed_mb_s = if elapsed_secs > 0.05 {
                    (streamed_so_far as f64 / (1024.0 * 1024.0)) / elapsed_secs
                } else {
                    0.0
                };

                let remaining_bytes = total_bytes.saturating_sub(current);
                let bytes_per_sec = if elapsed_secs > 0.05 {
                    streamed_so_far as f64 / elapsed_secs
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

    /// Sends a transfer control command (Pause, Resume, Cancel) to the receiver
    pub async fn send_transfer_control(
        &self,
        host_port: &str,
        session_id: &str,
        command: crate::transfer::TransferControlCommand,
    ) -> Result<()> {
        let url = format!("https://{host_port}/api/transfer/control");
        let payload = crate::transfer::TransferControlPayload {
            session_id: session_id.to_string(),
            command,
            reason: None,
        };
        let res = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to send transfer control to {url}"))?;
        if !res.status().is_success() {
            bail!("Transfer control command failed: HTTP {}", res.status());
        }
        Ok(())
    }

    /// Orchestrates file transfers honoring the priorities:
    /// - 4 parallel streams for small files (< 16 MB)
    /// - 1 dedicated stream for big files (>= 16 MB) to saturate the link without contention
    /// - Auto-resumes partially completed files if interrupted
    pub async fn send_files_smart<P>(
        self: Arc<Self>,
        host_port: String,
        session_id: String,
        session_token: String,
        items: Vec<(FileMetadata, PathBuf)>,
        on_progress: P,
    ) -> Result<Vec<UploadResult>>
    where
        P: Fn(FileTransferProgress) + Send + Sync + Clone + 'static,
    {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let mut tasks = Vec::new();

        for (meta, path) in items {
            let client = Arc::clone(&self);
            let sem = Arc::clone(&semaphore);
            let hp = host_port.clone();
            let sid = session_id.clone();
            let tok = session_token.clone();
            let cb = on_progress.clone();

            let is_big = meta.size_bytes >= BIG_FILE_THRESHOLD;
            tasks.push(tokio::spawn(async move {
                // Big files acquire all 4 permits (exclusive line); small files acquire 1 permit (up to 4 in parallel)
                let _permit = if is_big {
                    sem.acquire_many(4).await.context("Semaphore closed")?
                } else {
                    sem.acquire().await.context("Semaphore closed")?
                };

                // Check if receiver already has partial file for resume
                let resume_offset = client
                    .get_upload_status(&hp, &sid, &tok, &meta.file_id)
                    .await
                    .unwrap_or(0);
                let resume_from = if resume_offset < meta.size_bytes {
                    resume_offset
                } else {
                    0
                };

                let session = UploadSession {
                    host_port: &hp,
                    session_id: &sid,
                    session_token: &tok,
                };

                client
                    .stream_file_resumed(&session, &meta, &path, resume_from, move |p| {
                        cb(p);
                    })
                    .await
            }));
        }

        let mut results = Vec::new();
        for task in tasks {
            let res = task.await.context("Transfer task panicked")??;
            results.push(res);
        }

        Ok(results)
    }
}
