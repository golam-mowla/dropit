use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub file_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub blake3_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOfferRequest {
    pub session_id: String,
    pub sender_name: String,
    pub sender_fingerprint: String,
    pub files: Vec<FileMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOfferResponse {
    pub accepted: bool,
    pub session_token: Option<String>,
    pub pairing_code: String,
    pub reason: Option<String>,
}
