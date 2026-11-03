use crate::server::{
    CancelTransferRequest, DeviceInfoResponse, FileMetadata, TokenResponse,
    TransferDecisionRequest, TransferIntentRequest, TransferIntentResponse,
    TransferSuccessResponse,
};
use anyhow::{Context, Result, bail};
use reqwest::multipart::{Form, Part};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

#[derive(Clone, Debug)]
pub struct ProgressUpdate {
    pub files_completed: usize,
    pub total_files: usize,
    pub bytes_sent: u64,
    pub total_bytes: u64,
}

#[derive(Clone)]
pub struct DropItClient {
    client: reqwest::Client,
    base_url: String,
}

impl DropItClient {
    /// Creates a client connecting to target (host/ip and port).
    pub fn new(host: &str, port: u16) -> Result<Self> {
        crate::tls::init_crypto_provider();
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true) // baseline self-signed cert support
            .build()
            .context("Failed to build HTTP client")?;

        let base_url = format!("https://{}:{}", host, port);
        Ok(Self { client, base_url })
    }

    /// Fetches remote peer device info.
    pub async fn get_info(&self) -> Result<DeviceInfoResponse> {
        let url = format!("{}/api/info", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to peer /api/info")?;

        if !resp.status().is_success() {
            bail!("Peer returned error status: {}", resp.status());
        }

        resp.json::<DeviceInfoResponse>()
            .await
            .context("Failed to parse device info response")
    }

    /// Obtains a single-use session token from the receiver.
    pub async fn request_session_token(&self) -> Result<String> {
        let url = format!("{}/api/token", self.base_url);
        let resp = self
            .client
            .post(&url)
            .send()
            .await
            .context("Failed to request session token")?;

        if !resp.status().is_success() {
            bail!("Failed to acquire session token: {}", resp.status());
        }

        let token_resp = resp
            .json::<TokenResponse>()
            .await
            .context("Failed to parse token response")?;
        Ok(token_resp.token)
    }

    /// Pairs using an N-word-word code and returns session token and server fingerprint.
    pub async fn pair_with_code(
        &self,
        code: &str,
        client_id: &str,
        client_fingerprint: &str,
    ) -> Result<crate::server::PairingCodeResponse> {
        let url = format!("{}/api/pairing/code", self.base_url);
        let req = crate::server::PairingCodeRequest {
            code: code.to_string(),
            client_id: client_id.to_string(),
            client_fingerprint: client_fingerprint.to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to send pairing code")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Pairing failed ({}): {}", status, body);
        }

        resp.json::<crate::server::PairingCodeResponse>()
            .await
            .context("Failed to parse pairing response")
    }

    /// Initiates transfer intent (Accept/Reject negotiation).
    pub async fn prepare_transfer(
        &self,
        intent: &TransferIntentRequest,
    ) -> Result<TransferIntentResponse> {
        let url = format!("{}/api/transfer/prepare", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(intent)
            .send()
            .await
            .context("Failed to send transfer intent")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Transfer prepare failed ({}): {}", status, body);
        }

        resp.json::<TransferIntentResponse>()
            .await
            .context("Failed to parse transfer prepare response")
    }

    /// Sends user decision to accept or reject a pending transfer.
    pub async fn decide_transfer(
        &self,
        session_id: &str,
        accept: bool,
        reason: Option<String>,
    ) -> Result<()> {
        let url = format!("{}/api/transfer/decision", self.base_url);
        let req = TransferDecisionRequest {
            session_id: session_id.to_string(),
            accept,
            reason,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to send transfer decision")?;

        if !resp.status().is_success() {
            bail!("Decision request failed with status: {}", resp.status());
        }

        Ok(())
    }

    /// Sends cancellation signal for an ongoing transfer.
    pub async fn cancel_transfer(&self, session_id: &str) -> Result<()> {
        let url = format!("{}/api/transfer/cancel", self.base_url);
        let req = CancelTransferRequest {
            session_id: session_id.to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to send transfer cancel")?;

        if !resp.status().is_success() {
            bail!("Cancel request failed with status: {}", resp.status());
        }

        Ok(())
    }

    /// Sends files to receiver and optionally verifies Blake3 hashes.
    pub async fn send_files<P: AsRef<Path>>(
        &self,
        file_paths: &[P],
        verify_hash: bool,
    ) -> Result<()> {
        let token = self.request_session_token().await?;
        self.send_files_internal(file_paths, &token, None, verify_hash, None)
            .await
    }

    /// Sends files with intent handshake, cancellation token, and progress reporting.
    pub async fn send_files_with_intent<P: AsRef<Path>>(
        &self,
        file_paths: &[P],
        sender_id: &str,
        sender_alias: &str,
        verify_hash: bool,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        self.send_files_with_intent_and_session(
            None,
            file_paths,
            sender_id,
            sender_alias,
            verify_hash,
            cancel_flag,
        )
        .await
    }

    pub async fn send_files_with_intent_and_session<P: AsRef<Path>>(
        &self,
        custom_session_id: Option<&str>,
        file_paths: &[P],
        sender_id: &str,
        sender_alias: &str,
        verify_hash: bool,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        let session_id = custom_session_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{:016x}", rand::random::<u64>()));

        // 1. Collect file metadata
        let mut file_metas = Vec::new();
        let mut total_size = 0u64;

        for (idx, p) in file_paths.iter().enumerate() {
            let path = p.as_ref();
            let metadata = tokio::fs::metadata(path)
                .await
                .with_context(|| format!("Failed to read metadata for '{}'", path.display()))?;
            let size = metadata.len();
            total_size += size;

            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();

            file_metas.push(FileMetadata {
                id: idx.to_string(),
                filename,
                size,
                blake3_hash: None,
            });
        }

        // 2. Prepare transfer (Accept/Reject handshake)
        let intent = TransferIntentRequest {
            session_id: session_id.clone(),
            sender_id: sender_id.to_string(),
            sender_alias: sender_alias.to_string(),
            files: file_metas,
            total_size,
        };

        let resp = self.prepare_transfer(&intent).await?;
        if resp.status != "accepted" {
            bail!(
                "Transfer rejected: {}",
                resp.reason
                    .unwrap_or_else(|| "Declined by receiver".to_string())
            );
        }

        let token = resp
            .token
            .ok_or_else(|| anyhow::anyhow!("Missing token in accepted response"))?;

        // 3. Stream files
        self.send_files_internal(
            file_paths,
            &token,
            Some(&session_id),
            verify_hash,
            cancel_flag,
        )
        .await
    }

    /// Recursively sends an entire directory tree.
    pub async fn send_directory<P: AsRef<Path>>(
        &self,
        dir_path: P,
        verify_hash: bool,
    ) -> Result<()> {
        let root = dir_path.as_ref();
        if !root.is_dir() {
            bail!("Provided path is not a directory: {}", root.display());
        }

        let mut collected_files = Vec::new();
        Self::collect_files_recursive(root, &mut collected_files)?;

        self.send_files(&collected_files, verify_hash).await
    }

    fn collect_files_recursive(dir: &Path, list: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files_recursive(&path, list)?;
            } else if path.is_file() {
                list.push(path);
            }
        }
        Ok(())
    }

    async fn send_files_internal<P: AsRef<Path>>(
        &self,
        file_paths: &[P],
        token: &str,
        session_id: Option<&str>,
        verify_hash: bool,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        let mut form = Form::new();
        let mut local_hashes = Vec::new();

        for p in file_paths {
            if let Some(ref cf) = cancel_flag
                && cf.load(Ordering::SeqCst)
            {
                bail!("Transfer cancelled before uploading");
            }

            let path = p.as_ref();
            let file_bytes = tokio::fs::read(path)
                .await
                .with_context(|| format!("Failed to read file '{}'", path.display()))?;

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();

            if verify_hash {
                let hash = blake3::hash(&file_bytes).to_hex().to_string();
                local_hashes.push((file_name.clone(), hash));
            }

            let part = Part::bytes(file_bytes).file_name(file_name);
            form = form.part("file", part);
        }

        let url = format!("{}/api/transfer", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("X-Session-Token", token)
            .multipart(form);

        if let Some(sid) = session_id {
            req = req.header("X-Session-ID", sid);
        }

        let resp = req
            .send()
            .await
            .context("Failed to send multipart transfer")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Transfer failed with status {}: {}", status, body);
        }

        let transfer_resp = resp
            .json::<TransferSuccessResponse>()
            .await
            .context("Failed to parse transfer response")?;

        if verify_hash {
            for (filename, local_hash) in local_hashes {
                if let Some(remote_file) =
                    transfer_resp.files.iter().find(|f| f.filename == filename)
                    && remote_file.blake3_hash != local_hash
                {
                    bail!(
                        "Hash verification mismatch for '{}': local={}, remote={}",
                        filename,
                        local_hash,
                        remote_file.blake3_hash
                    );
                }
            }
            info!("All file Blake3 hashes verified successfully against remote receiver");
        }

        Ok(())
    }

    /// Sends an encrypted or plaintext chat message / event into a session thread.
    pub async fn send_chat_message(
        &self,
        message: &crate::messenger::ChatMessage,
    ) -> Result<String> {
        let url = format!("{}/api/chat/message", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(message)
            .send()
            .await
            .context("Failed to send chat message")?;

        if !resp.status().is_success() {
            bail!("Failed to send chat message: {}", resp.status());
        }

        let res: crate::server::ChatSendResponse = resp
            .json()
            .await
            .context("Failed to parse chat send response")?;
        Ok(res.id)
    }

    /// Fetches all messages and events in a chat session thread.
    pub async fn get_chat_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::messenger::ChatMessage>> {
        let url = format!(
            "{}/api/chat/messages?session_id={}",
            self.base_url, session_id
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch chat messages")?;

        if !resp.status().is_success() {
            bail!("Failed to fetch chat messages: {}", resp.status());
        }

        resp.json::<Vec<crate::messenger::ChatMessage>>()
            .await
            .context("Failed to parse chat messages")
    }
}
