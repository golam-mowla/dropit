use crate::server::{DeviceInfoResponse, TokenResponse, TransferSuccessResponse};
use anyhow::{Context, Result, bail};
use reqwest::multipart::{Form, Part};
use std::path::Path;
use tracing::info;

pub struct DropItClient {
    client: reqwest::Client,
    base_url: String,
}

impl DropItClient {
    /// Creates a client connecting to target (host/ip and port).
    pub fn new(host: &str, port: u16) -> Result<Self> {
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

    /// Sends files to receiver and optionally verifies Blake3 hashes.
    pub async fn send_files<P: AsRef<Path>>(
        &self,
        file_paths: &[P],
        verify_hash: bool,
    ) -> Result<()> {
        let token = self.request_session_token().await?;
        let mut form = Form::new();
        let mut local_hashes = Vec::new();

        for p in file_paths {
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
        let resp = self
            .client
            .post(&url)
            .header("X-Session-Token", token)
            .multipart(form)
            .send()
            .await
            .context("Failed to send multipart transfer")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Transfer rejected with status {}: {}", status, body);
        }

        let transfer_resp = resp
            .json::<TransferSuccessResponse>()
            .await
            .context("Failed to parse transfer response")?;

        if verify_hash {
            for (name, expected_hash) in &local_hashes {
                if let Some(remote) = transfer_resp.files.iter().find(|f| f.filename == *name) {
                    if remote.blake3_hash != *expected_hash {
                        bail!(
                            "Hash verification mismatch for {}: expected {}, remote {}",
                            name,
                            expected_hash,
                            remote.blake3_hash
                        );
                    }
                    info!("Verified Blake3 hash for '{}' ({})", name, expected_hash);
                } else {
                    bail!("Remote server did not confirm transfer of '{}'", name);
                }
            }
        }

        info!("All files successfully transferred!");
        Ok(())
    }
}
