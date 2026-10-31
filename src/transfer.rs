use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub file_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub blake3_hash: Option<String>,
    #[serde(default)]
    pub sha256_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferControlCommand {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferControlPayload {
    pub session_id: String,
    pub command: TransferControlCommand,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOfferRequest {
    pub session_id: String,
    pub sender_name: String,
    pub sender_device_id: String,
    pub sender_fingerprint: String,
    pub files: Vec<FileMetadata>,
    #[serde(default = "default_port")]
    pub sender_port: u16,
}

fn default_port() -> u16 {
    53420
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOfferResponse {
    pub accepted: bool,
    pub session_token: Option<String>,
    pub pairing_code: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResult {
    pub file_id: String,
    pub bytes_written: u64,
    pub blake3_hash: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferProgress {
    pub file_id: String,
    pub filename: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_mb_s: f64,
    pub eta_secs: Option<u64>,
    pub completed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub session_id: String,
    pub session_token: String,
    pub sender_name: String,
    pub sender_device_id: String,
    pub sender_fingerprint: String,
    pub files: Vec<FileMetadata>,
    pub dest_paths: HashMap<String, PathBuf>, // file_id -> destination path
    pub completed_files: HashSet<String>,
    pub total_bytes: u64,
    pub received_bytes: u64,
    pub created_at: Instant,
    pub is_paused: bool,
    pub is_cancelled: bool,
}

impl ActiveSession {
    pub fn is_completed(&self) -> bool {
        self.completed_files.len() >= self.files.len()
    }
}
