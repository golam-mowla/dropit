use anyhow::{Context, Result, bail};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit},
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, ZeroizeOnDrop};

// ============================================================================
// Chat Protocol Models (Shared Thread for Text, Files, Receipts, Typing)
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReceiptStatus {
    Delivered,
    Read,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum MessagePayload {
    Text(String),
    FileBubble {
        file_id: String,
        filename: String,
        size: u64,
        blake3_hash: String,
    },
    TypingIndicator {
        is_typing: bool,
    },
    Receipt {
        target_message_id: String,
        status: ReceiptStatus,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub sender_id: String,
    pub sender_alias: String,
    pub timestamp_ms: u64,
    pub payload: MessagePayload,
}

impl ChatMessage {
    pub fn new(
        session_id: &str,
        sender_id: &str,
        sender_alias: &str,
        payload: MessagePayload,
    ) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            id: format!("{:016x}", rand::random::<u64>()),
            session_id: session_id.to_string(),
            sender_id: sender_id.to_string(),
            sender_alias: sender_alias.to_string(),
            timestamp_ms,
            payload,
        }
    }
}

// ============================================================================
// Symmetric Hash Ratchet (Forward Secrecy per message)
// ============================================================================

/// Symmetric hash ratchet providing forward secrecy for an active session.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SymmetricRatchet {
    chain_key: [u8; 32],
    #[zeroize(skip)]
    step_count: u64,
}

impl SymmetricRatchet {
    /// Initializes ratchet from a 32-byte shared root/session key.
    pub fn new(initial_key: [u8; 32]) -> Self {
        Self {
            chain_key: initial_key,
            step_count: 0,
        }
    }

    /// Steps the ratchet forward: computes (next_chain_key, message_key),
    /// replaces chain_key, and returns the message key.
    pub fn step(&mut self) -> Result<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(Some(b"dropit-ratchet-salt-v1"), &self.chain_key);
        let mut okm = [0u8; 64];
        hk.expand(b"dropit-ratchet-step", &mut okm)
            .map_err(|e| anyhow::anyhow!("Ratchet HKDF expand failed: {:?}", e))?;

        self.chain_key.copy_from_slice(&okm[0..32]);
        self.step_count += 1;

        let mut msg_key = [0u8; 32];
        msg_key.copy_from_slice(&okm[32..64]);
        okm.zeroize();
        Ok(msg_key)
    }

    pub fn current_step(&self) -> u64 {
        self.step_count
    }
}

// ============================================================================
// Length-Prefixed Wire Frame Protocol
// ============================================================================

/// Encrypts and packs a message into a length-prefixed frame:
/// [4 bytes big-endian length][12 bytes nonce][ciphertext + 16 bytes tag]
pub fn pack_frame(key: &[u8; 32], nonce_counter: u64, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[4..12].copy_from_slice(&nonce_counter.to_be_bytes());

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("Frame encryption error: {:?}", e))?;

    let payload_len = (12 + ciphertext.len()) as u32;
    let mut frame = Vec::with_capacity(4 + 12 + ciphertext.len());
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(&nonce_bytes);
    frame.extend_from_slice(&ciphertext);
    Ok(frame)
}

/// Unpacks and decrypts a length-prefixed frame.
pub fn unpack_frame(key: &[u8; 32], frame: &[u8]) -> Result<Vec<u8>> {
    if frame.len() < 4 + 12 + 16 {
        bail!("Frame buffer too short");
    }

    let payload_len = u32::from_be_bytes(frame[0..4].try_into()?) as usize;
    if frame.len() < 4 + payload_len {
        bail!("Incomplete frame buffer");
    }

    let nonce_bytes = &frame[4..16];
    let ciphertext = &frame[16..4 + payload_len];

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Frame decryption failed: {:?}", e))?;

    Ok(plaintext)
}

// ============================================================================
// Windows DPAPI-Protected Chat History Storage
// ============================================================================

#[cfg(windows)]
pub fn dpapi_encrypt(plaintext: &[u8]) -> Result<Vec<u8>> {
    #[allow(non_camel_case_types, non_snake_case)]
    #[repr(C)]
    struct DATA_BLOB {
        cbData: u32,
        pbData: *mut u8,
    }

    unsafe extern "system" {
        fn CryptProtectData(
            pDataIn: *const DATA_BLOB,
            szDataDescr: *const u16,
            pOptionalEntropy: *const DATA_BLOB,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DATA_BLOB,
        ) -> i32;

        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    let in_blob = DATA_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut out_blob = DATA_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ret = unsafe {
        CryptProtectData(
            &in_blob,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0x01, // CRYPTPROTECT_UI_FORBIDDEN
            &mut out_blob,
        )
    };

    if ret != 0 && !out_blob.pbData.is_null() {
        let slice =
            unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) };
        let result = slice.to_vec();
        unsafe { LocalFree(out_blob.pbData as *mut std::ffi::c_void) };
        Ok(result)
    } else {
        bail!(
            "DPAPI CryptProtectData failed with os error: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(windows)]
pub fn dpapi_decrypt(ciphertext: &[u8]) -> Result<Vec<u8>> {
    #[allow(non_camel_case_types, non_snake_case)]
    #[repr(C)]
    struct DATA_BLOB {
        cbData: u32,
        pbData: *mut u8,
    }

    unsafe extern "system" {
        fn CryptUnprotectData(
            pDataIn: *const DATA_BLOB,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *const DATA_BLOB,
            pvReserved: *mut std::ffi::c_void,
            pPromptStruct: *mut std::ffi::c_void,
            dwFlags: u32,
            pDataOut: *mut DATA_BLOB,
        ) -> i32;

        fn LocalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    let in_blob = DATA_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut out_blob = DATA_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ret = unsafe {
        CryptUnprotectData(
            &in_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0x01, // CRYPTPROTECT_UI_FORBIDDEN
            &mut out_blob,
        )
    };

    if ret != 0 && !out_blob.pbData.is_null() {
        let slice =
            unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) };
        let result = slice.to_vec();
        unsafe { LocalFree(out_blob.pbData as *mut std::ffi::c_void) };
        Ok(result)
    } else {
        bail!(
            "DPAPI CryptUnprotectData failed with os error: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(windows))]
pub fn dpapi_encrypt(plaintext: &[u8]) -> Result<Vec<u8>> {
    // Cross-platform mock for CI / non-Windows test environments
    let mut out = Vec::with_capacity(plaintext.len() + 4);
    out.extend_from_slice(b"MOCK");
    for b in plaintext {
        out.push(b ^ 0x42);
    }
    Ok(out)
}

#[cfg(not(windows))]
pub fn dpapi_decrypt(ciphertext: &[u8]) -> Result<Vec<u8>> {
    if !ciphertext.starts_with(b"MOCK") {
        bail!("Invalid mock ciphertext prefix");
    }
    let mut out = Vec::with_capacity(ciphertext.len() - 4);
    for b in &ciphertext[4..] {
        out.push(b ^ 0x42);
    }
    Ok(out)
}

/// Persistent, DPAPI-protected chat history store.
#[derive(Clone)]
pub struct ChatHistoryStore {
    base_dir: PathBuf,
}

impl ChatHistoryStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn thread_file_path(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("thread_{}.bin", session_id))
    }

    /// Saves chat messages encrypted with Windows DPAPI.
    pub fn save_thread(&self, session_id: &str, messages: &[ChatMessage]) -> Result<()> {
        std::fs::create_dir_all(&self.base_dir)
            .with_context(|| format!("Failed to create chat dir {:?}", self.base_dir))?;

        let json = serde_json::to_vec(messages)?;
        let encrypted = dpapi_encrypt(&json)?;
        let path = self.thread_file_path(session_id);
        std::fs::write(&path, encrypted)
            .with_context(|| format!("Failed to write encrypted thread to {:?}", path))?;
        Ok(())
    }

    /// Loads and decrypts chat messages using Windows DPAPI.
    pub fn load_thread(&self, session_id: &str) -> Result<Vec<ChatMessage>> {
        let path = self.thread_file_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let encrypted = std::fs::read(&path)
            .with_context(|| format!("Failed to read thread from {:?}", path))?;
        let decrypted = dpapi_decrypt(&encrypted)?;
        let messages = serde_json::from_slice(&decrypted)?;
        Ok(messages)
    }
}

// ============================================================================
// In-Memory Chat Conversation State
// ============================================================================

#[derive(Clone, Default)]
pub struct ChatSessionManager {
    threads: Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>,
}

impl ChatSessionManager {
    pub fn new() -> Self {
        Self {
            threads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn append_message(&self, message: ChatMessage) {
        let mut map = self.threads.lock().unwrap();
        map.entry(message.session_id.clone())
            .or_default()
            .push(message);
    }

    pub fn get_messages(&self, session_id: &str) -> Vec<ChatMessage> {
        let map = self.threads.lock().unwrap();
        map.get(session_id).cloned().unwrap_or_default()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ratchet_stepping_and_forward_secrecy() {
        let initial_root = [0x5au8; 32];
        let mut ratchet_a = SymmetricRatchet::new(initial_root);
        let mut ratchet_b = SymmetricRatchet::new(initial_root);

        // Step 1: Both ratchets produce identical message key for step 1
        let key_a1 = ratchet_a.step().unwrap();
        let key_b1 = ratchet_b.step().unwrap();
        assert_eq!(key_a1, key_b1);
        assert_eq!(ratchet_a.current_step(), 1);

        // Step 2: Step 2 message key differs from Step 1
        let key_a2 = ratchet_a.step().unwrap();
        let key_b2 = ratchet_b.step().unwrap();
        assert_eq!(key_a2, key_b2);
        assert_ne!(key_a1, key_a2);
        assert_eq!(ratchet_a.current_step(), 2);
    }

    #[test]
    fn test_wire_frame_pack_unpack() {
        let key = [0x42u8; 32];
        let nonce_counter = 12345u64;
        let plaintext = b"Hello end-to-end encrypted messenger!";

        let frame = pack_frame(&key, nonce_counter, plaintext).unwrap();
        assert!(frame.len() > plaintext.len());

        let decrypted = unpack_frame(&key, &frame).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wire_frame_tampering_fails() {
        let key = [0x42u8; 32];
        let nonce_counter = 1u64;
        let plaintext = b"Secure content";

        let mut frame = pack_frame(&key, nonce_counter, plaintext).unwrap();
        // Tamper with last byte (part of Poly1305 tag)
        let last = frame.len() - 1;
        frame[last] ^= 0x01;

        let result = unpack_frame(&key, &frame);
        assert!(result.is_err());
    }

    #[test]
    fn test_dpapi_roundtrip_storage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = ChatHistoryStore::new(temp_dir.path().to_path_buf());

        let session_id = "test-session-123";
        let msg1 = ChatMessage::new(
            session_id,
            "sender-1",
            "Swift Falcon",
            MessagePayload::Text("Hello over DPAPI!".to_string()),
        );
        let msg2 = ChatMessage::new(
            session_id,
            "sender-1",
            "Swift Falcon",
            MessagePayload::FileBubble {
                file_id: "file-abc".to_string(),
                filename: "report.pdf".to_string(),
                size: 2048,
                blake3_hash: "hash123".to_string(),
            },
        );

        let thread = vec![msg1.clone(), msg2.clone()];
        store.save_thread(session_id, &thread).unwrap();

        // Verify the file exists and is encrypted (cannot be parsed as plaintext JSON)
        let raw = std::fs::read(store.thread_file_path(session_id)).unwrap();
        assert!(serde_json::from_slice::<Vec<ChatMessage>>(&raw).is_err());

        // Decrypt and verify
        let loaded = store.load_thread(session_id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], msg1);
        assert_eq!(loaded[1], msg2);
    }

    #[test]
    fn test_chat_message_types_serialization() {
        let typing = MessagePayload::TypingIndicator { is_typing: true };
        let json_typing = serde_json::to_string(&typing).unwrap();
        let decoded_typing: MessagePayload = serde_json::from_str(&json_typing).unwrap();
        assert_eq!(typing, decoded_typing);

        let receipt = MessagePayload::Receipt {
            target_message_id: "msg-999".to_string(),
            status: ReceiptStatus::Read,
        };
        let json_receipt = serde_json::to_string(&receipt).unwrap();
        let decoded_receipt: MessagePayload = serde_json::from_str(&json_receipt).unwrap();
        assert_eq!(receipt, decoded_receipt);
    }
}
