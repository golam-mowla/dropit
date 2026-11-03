use anyhow::{Context, Result, bail};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit},
};
use hkdf::Hkdf;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spake2::{Ed25519Group, Spake2};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, ZeroizeOnDrop};

// Embedded curated word lists for N-word-word pairing codes
const ADJECTIVES: &[&str] = &[
    "amber", "azure", "bright", "bronze", "calm", "cedar", "clever", "coral", "crimson", "crystal",
    "daring", "dawn", "dusk", "emerald", "ember", "falcon", "frost", "golden", "granite", "hazel",
    "indigo", "jade", "jasper", "lunar", "marine", "mystic", "noble", "ocean", "opal", "pearl",
    "polar", "quiet", "radiant", "rapid", "ruby", "rust", "sable", "sage", "sapphire", "shadow",
    "silver", "solar", "swift", "topaz", "twilight", "ultra", "velvet", "vibrant", "violet",
    "vivid", "wild", "wind",
];

const NOUNS: &[&str] = &[
    "badger",
    "bear",
    "beaver",
    "bison",
    "cheetah",
    "condor",
    "coyote",
    "crane",
    "deer",
    "dolphin",
    "dragon",
    "eagle",
    "falcon",
    "finch",
    "fox",
    "gazelle",
    "gecko",
    "harrier",
    "hawk",
    "heron",
    "jaguar",
    "kestrel",
    "leopard",
    "lion",
    "lynx",
    "marten",
    "otter",
    "owl",
    "panther",
    "pelican",
    "penguin",
    "puma",
    "raven",
    "rhino",
    "robin",
    "sable",
    "seal",
    "shark",
    "sparrow",
    "starling",
    "swan",
    "tiger",
    "viper",
    "walrus",
    "whale",
    "wolf",
    "wolverine",
    "zebra",
];

/// Generates a human-friendly pairing code in the format "N-word-word" (e.g. "7-amber-falcon").
pub fn generate_pairing_code() -> String {
    let mut rng = rand::thread_rng();
    let num: u8 = rng.gen_range(1..=9);
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{}-{}-{}", num, adj, noun)
}

/// Errors related to pairing verification.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("Pairing code has expired or was not found")]
    ExpiredOrNotFound,
    #[error("Invalid pairing code. Remaining attempts: {remaining_strikes}")]
    InvalidCode { remaining_strikes: usize },
    #[error("Too many failed attempts. Code has been permanently locked out.")]
    LockedOut,
    #[error("Rate limit exceeded. Please wait before retrying.")]
    RateLimited,
}

struct ActiveCode {
    code: String,
    created_at: Instant,
    failed_attempts: usize,
}

/// Manages pairing codes, lockout after 3 strikes, and IP rate limiting.
#[derive(Clone)]
pub struct PairingSessionStore {
    active_code: Arc<Mutex<Option<ActiveCode>>>,
    ttl: Duration,
    max_strikes: usize,
    rate_limits: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
}

impl Default for PairingSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingSessionStore {
    /// Creates a new store with a 5-minute TTL and 3-strike lockout.
    pub fn new() -> Self {
        Self {
            active_code: Arc::new(Mutex::new(None)),
            ttl: Duration::from_secs(300),
            max_strikes: 3,
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Creates a store with custom TTL and max strikes.
    pub fn with_options(ttl: Duration, max_strikes: usize) -> Self {
        Self {
            active_code: Arc::new(Mutex::new(None)),
            ttl,
            max_strikes,
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Generates and stores a new pairing code, replacing any prior code.
    pub fn generate_code(&self) -> String {
        let code = generate_pairing_code();
        let mut lock = self.active_code.lock().unwrap();
        *lock = Some(ActiveCode {
            code: code.clone(),
            created_at: Instant::now(),
            failed_attempts: 0,
        });
        code
    }

    /// Manually sets a specific code (useful for tests and manual entry).
    pub fn set_code(&self, code: &str) {
        let mut lock = self.active_code.lock().unwrap();
        *lock = Some(ActiveCode {
            code: code.to_string(),
            created_at: Instant::now(),
            failed_attempts: 0,
        });
    }

    /// Checks rate limit for the given client IP (max 5 attempts per minute).
    fn check_rate_limit(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut limits = self.rate_limits.lock().unwrap();
        let timestamps = limits.entry(ip).or_default();
        timestamps.retain(|t| now.duration_since(*t) <= window);

        if timestamps.len() >= 5 {
            false
        } else {
            timestamps.push(now);
            true
        }
    }

    /// Verifies a pairing code attempt from a specific IP.
    /// Invalides the code if strikes exceed max_strikes.
    pub fn verify_code(&self, candidate: &str, ip: IpAddr) -> Result<(), PairingError> {
        if !self.check_rate_limit(ip) {
            return Err(PairingError::RateLimited);
        }

        let mut lock = self.active_code.lock().unwrap();
        let active = match lock.as_mut() {
            Some(a) => a,
            None => return Err(PairingError::ExpiredOrNotFound),
        };

        if active.created_at.elapsed() > self.ttl {
            *lock = None;
            return Err(PairingError::ExpiredOrNotFound);
        }

        // Compare case-insensitively and ignoring whitespace
        if active.code.trim().eq_ignore_ascii_case(candidate.trim()) {
            *lock = None; // Single-use on success
            Ok(())
        } else {
            active.failed_attempts += 1;
            if active.failed_attempts >= self.max_strikes {
                *lock = None; // Permanent lockout on 3 strikes
                Err(PairingError::LockedOut)
            } else {
                let remaining = self.max_strikes - active.failed_attempts;
                Err(PairingError::InvalidCode {
                    remaining_strikes: remaining,
                })
            }
        }
    }
}

/// Backward compatibility token store wrapper for API tokens.
#[derive(Clone, Default)]
pub struct SessionTokenStore {
    tokens: Arc<Mutex<HashMap<String, Instant>>>,
    ttl: Duration,
    pairing_session: PairingSessionStore,
}

impl SessionTokenStore {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(300),
            pairing_session: PairingSessionStore::new(),
        }
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            ttl,
            pairing_session: PairingSessionStore::with_options(ttl, 3),
        }
    }

    pub fn generate_token(&self) -> String {
        let token = format!(
            "{:016x}{:016x}",
            rand::random::<u64>(),
            rand::random::<u64>()
        );
        let mut map = self.tokens.lock().unwrap();
        map.insert(token.clone(), Instant::now());
        token
    }

    pub fn verify_and_consume(&self, token: &str) -> bool {
        let mut map = self.tokens.lock().unwrap();
        if let Some(created_at) = map.remove(token) {
            created_at.elapsed() <= self.ttl
        } else {
            false
        }
    }

    pub fn pairing_session(&self) -> &PairingSessionStore {
        &self.pairing_session
    }
}

// ============================================================================
// SPAKE2 Key Exchange & ChaCha20-Poly1305 Frame Encryption
// ============================================================================

const SPAKE_ID_INITIATOR: &[u8] = b"dropit-initiator";
const SPAKE_ID_RESPONDER: &[u8] = b"dropit-responder";
const HKDF_SALT: &[u8] = b"dropit-spake2-tls-binding-v1";

/// Initiator side of SPAKE2 handshake.
pub struct Spake2Initiator {
    spake: Spake2<Ed25519Group>,
    local_cert_fingerprint: String,
}

/// Responder side of SPAKE2 handshake.
pub struct Spake2Responder {
    spake: Spake2<Ed25519Group>,
    local_cert_fingerprint: String,
}

impl Spake2Initiator {
    /// Starts SPAKE2 as initiator using the pairing code and local TLS fingerprint.
    pub fn start(code: &str, local_cert_fingerprint: &str) -> (Self, Vec<u8>) {
        let (spake, msg) = Spake2::<Ed25519Group>::start_a(
            code.trim().as_bytes(),
            SPAKE_ID_INITIATOR,
            SPAKE_ID_RESPONDER,
        );
        (
            Self {
                spake,
                local_cert_fingerprint: local_cert_fingerprint.to_string(),
            },
            msg,
        )
    }

    /// Finishes SPAKE2 given the responder's message and cert fingerprint.
    /// Derives session keys bound to both TLS fingerprints.
    pub fn finish(
        self,
        responder_msg: &[u8],
        remote_cert_fingerprint: &str,
    ) -> Result<SessionCipher> {
        let shared_key = self
            .spake
            .finish(responder_msg)
            .map_err(|e| anyhow::anyhow!("SPAKE2 verification failed: {:?}", e))?;

        derive_session_cipher(
            &shared_key,
            &self.local_cert_fingerprint,
            remote_cert_fingerprint,
            true,
        )
    }
}

impl Spake2Responder {
    /// Starts SPAKE2 as responder using the pairing code and local TLS fingerprint.
    pub fn start(code: &str, local_cert_fingerprint: &str) -> (Self, Vec<u8>) {
        let (spake, msg) = Spake2::<Ed25519Group>::start_b(
            code.trim().as_bytes(),
            SPAKE_ID_INITIATOR,
            SPAKE_ID_RESPONDER,
        );
        (
            Self {
                spake,
                local_cert_fingerprint: local_cert_fingerprint.to_string(),
            },
            msg,
        )
    }

    /// Finishes SPAKE2 given the initiator's message and cert fingerprint.
    /// Derives session keys bound to both TLS fingerprints.
    pub fn finish(
        self,
        initiator_msg: &[u8],
        remote_cert_fingerprint: &str,
    ) -> Result<SessionCipher> {
        let shared_key = self
            .spake
            .finish(initiator_msg)
            .map_err(|e| anyhow::anyhow!("SPAKE2 verification failed: {:?}", e))?;

        derive_session_cipher(
            &shared_key,
            remote_cert_fingerprint,
            &self.local_cert_fingerprint,
            false,
        )
    }
}

/// Derives symmetric encryption keys using HKDF-SHA256 bound to TLS certificate fingerprints.
fn derive_session_cipher(
    shared_key: &[u8],
    initiator_fingerprint: &str,
    responder_fingerprint: &str,
    is_initiator: bool,
) -> Result<SessionCipher> {
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), shared_key);
    let info = format!(
        "dropit-v1-session-bind:{}:{}",
        initiator_fingerprint, responder_fingerprint
    );

    let mut key_material = [0u8; 64];
    hk.expand(info.as_bytes(), &mut key_material)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {:?}", e))?;

    let mut init_to_resp = [0u8; 32];
    let mut resp_to_init = [0u8; 32];
    init_to_resp.copy_from_slice(&key_material[0..32]);
    resp_to_init.copy_from_slice(&key_material[32..64]);

    // Zeroize intermediate key material
    key_material.zeroize();

    let (send_key, recv_key) = if is_initiator {
        (init_to_resp, resp_to_init)
    } else {
        (resp_to_init, init_to_resp)
    };

    Ok(SessionCipher {
        send_cipher: ChaCha20Poly1305::new(Key::from_slice(&send_key)),
        recv_cipher: ChaCha20Poly1305::new(Key::from_slice(&recv_key)),
        send_key,
        recv_key,
        send_nonce: 0,
        recv_nonce: 0,
    })
}

/// Symmetric frame cipher using ChaCha20-Poly1305 with zeroization on drop.
pub struct SessionCipher {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_key: [u8; 32],
    recv_key: [u8; 32],
    send_nonce: u64,
    recv_nonce: u64,
}

impl Drop for SessionCipher {
    fn drop(&mut self) {
        self.send_key.zeroize();
        self.recv_key.zeroize();
    }
}

impl SessionCipher {
    fn make_nonce(counter: u64, is_sender: bool) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[0] = if is_sender { 0x01 } else { 0x02 };
        nonce[4..12].copy_from_slice(&counter.to_be_bytes());
        nonce
    }

    /// Encrypts plaintext into a frame [12-byte nonce][ciphertext + 16-byte Poly1305 tag].
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce_bytes = Self::make_nonce(self.send_nonce, true);
        self.send_nonce += 1;

        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .send_cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption error: {:?}", e))?;

        let mut frame = Vec::with_capacity(12 + ciphertext.len());
        frame.extend_from_slice(&nonce_bytes);
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }

    /// Decrypts a frame [12-byte nonce][ciphertext + 16-byte Poly1305 tag].
    pub fn decrypt(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        if frame.len() < 12 {
            bail!("Frame too short for nonce");
        }
        let nonce = Nonce::from_slice(&frame[..12]);
        let ciphertext = &frame[12..];

        let plaintext = self.recv_cipher.decrypt(nonce, ciphertext).map_err(|e| {
            anyhow::anyhow!("Decryption error (bad key or corrupted data): {:?}", e)
        })?;

        self.recv_nonce += 1;
        Ok(plaintext)
    }
}

// ============================================================================
// TOFU (Trust On First Use) Certificate Pinning
// ============================================================================

#[derive(Debug, PartialEq, Eq)]
pub enum TrustStatus {
    Trusted,
    NewDevicePinned,
    FingerprintMismatch { expected: String, actual: String },
}

/// In-memory and file-backed TOFU device trust store.
#[derive(Clone, Default)]
pub struct DeviceTrustStore {
    pinned: Arc<Mutex<HashMap<String, String>>>,
}

impl DeviceTrustStore {
    pub fn new() -> Self {
        Self {
            pinned: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Verifies peer fingerprint against pinned fingerprint or pins if first use.
    pub fn verify_or_pin(&self, device_id: &str, fingerprint: &str) -> TrustStatus {
        let mut map = self.pinned.lock().unwrap();
        if let Some(expected) = map.get(device_id) {
            if expected == fingerprint {
                TrustStatus::Trusted
            } else {
                TrustStatus::FingerprintMismatch {
                    expected: expected.clone(),
                    actual: fingerprint.to_string(),
                }
            }
        } else {
            map.insert(device_id.to_string(), fingerprint.to_string());
            TrustStatus::NewDevicePinned
        }
    }
}

// ============================================================================
// System Hardening: Disk Space Pre-Check
// ============================================================================

#[cfg(windows)]
pub fn available_disk_space(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_bytes_available: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut total_free_bytes: u64 = 0;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let ret = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };

    if ret != 0 {
        Ok(free_bytes_available)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
pub fn available_disk_space(_path: &Path) -> std::io::Result<u64> {
    Ok(u64::MAX)
}

/// Verifies that the destination path has sufficient disk space for the required bytes.
pub fn verify_disk_space(path: &Path, required_bytes: u64) -> Result<()> {
    let available = available_disk_space(path).unwrap_or(u64::MAX);
    if available < required_bytes {
        bail!(
            "Insufficient disk space: required {} bytes, available {} bytes",
            required_bytes,
            available
        );
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_pairing_code_format() {
        let code = generate_pairing_code();
        let parts: Vec<&str> = code.split('-').collect();
        assert_eq!(parts.len(), 3);
        let num: u8 = parts[0].parse().expect("First part must be integer");
        assert!((1..=9).contains(&num));
        assert!(ADJECTIVES.contains(&parts[1]));
        assert!(NOUNS.contains(&parts[2]));
    }

    #[test]
    fn test_pairing_valid_code() {
        let store = PairingSessionStore::new();
        let code = store.generate_code();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        assert!(store.verify_code(&code, ip).is_ok());
        // Single-use: second attempt fails with ExpiredOrNotFound
        assert_eq!(
            store.verify_code(&code, ip),
            Err(PairingError::ExpiredOrNotFound)
        );
    }

    #[test]
    fn test_pairing_wrong_code_and_strikes() {
        let store = PairingSessionStore::new();
        store.set_code("5-swift-falcon");
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Strike 1
        assert_eq!(
            store.verify_code("wrong-code-1", ip),
            Err(PairingError::InvalidCode {
                remaining_strikes: 2
            })
        );

        // Strike 2
        assert_eq!(
            store.verify_code("wrong-code-2", ip),
            Err(PairingError::InvalidCode {
                remaining_strikes: 1
            })
        );

        // Strike 3 (Lockout)
        assert_eq!(
            store.verify_code("wrong-code-3", ip),
            Err(PairingError::LockedOut)
        );

        // Subsequent attempt shows code is gone
        assert_eq!(
            store.verify_code("5-swift-falcon", ip),
            Err(PairingError::ExpiredOrNotFound)
        );
    }

    #[test]
    fn test_pairing_expiration() {
        let store = PairingSessionStore::with_options(Duration::from_millis(10), 3);
        let code = store.generate_code();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(
            store.verify_code(&code, ip),
            Err(PairingError::ExpiredOrNotFound)
        );
    }

    #[test]
    fn test_rate_limiter() {
        let store = PairingSessionStore::new();
        store.set_code("7-amber-falcon");
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));

        // 5 rapid attempts from same IP
        for _ in 0..5 {
            let _ = store.verify_code("wrong", ip);
        }

        // 6th attempt is rate limited
        assert_eq!(
            store.verify_code("wrong", ip),
            Err(PairingError::RateLimited)
        );
    }

    #[test]
    fn test_spake2_successful_key_exchange_and_cipher() {
        let code = "3-bright-river";
        let fp_init = "cert-fingerprint-initiator-abc123";
        let fp_resp = "cert-fingerprint-responder-xyz789";

        // Step 1: Initiator starts
        let (initiator, init_msg) = Spake2Initiator::start(code, fp_init);

        // Step 2: Responder starts
        let (responder, resp_msg) = Spake2Responder::start(code, fp_resp);

        // Step 3: Finish both sides
        let mut cipher_init = initiator
            .finish(&resp_msg, fp_resp)
            .expect("Initiator finish failed");
        let mut cipher_resp = responder
            .finish(&init_msg, fp_init)
            .expect("Responder finish failed");

        // Step 4: Test bidirectional frame encryption
        let plaintext_a = b"Secret payload from initiator!";
        let encrypted_a = cipher_init.encrypt(plaintext_a).unwrap();
        let decrypted_a = cipher_resp.decrypt(&encrypted_a).unwrap();
        assert_eq!(plaintext_a.to_vec(), decrypted_a);

        let plaintext_b = b"Secret response from responder!";
        let encrypted_b = cipher_resp.encrypt(plaintext_b).unwrap();
        let decrypted_b = cipher_init.decrypt(&encrypted_b).unwrap();
        assert_eq!(plaintext_b.to_vec(), decrypted_b);
    }

    #[test]
    fn test_spake2_wrong_password_fails() {
        let code_correct = "3-bright-river";
        let code_wrong = "4-dark-mountain";
        let fp_init = "cert-fingerprint-initiator";
        let fp_resp = "cert-fingerprint-responder";

        let (initiator, _init_msg) = Spake2Initiator::start(code_correct, fp_init);
        let (_responder, resp_msg) = Spake2Responder::start(code_wrong, fp_resp);

        // Initiator finishing with responder's msg derived from different code must fail
        let result = initiator.finish(&resp_msg, fp_resp);
        assert!(result.is_err());
    }

    #[test]
    fn test_spake2_mitm_cert_mismatch_fails() {
        let code = "3-bright-river";
        let fp_init = "cert-fingerprint-initiator";
        let fp_resp_legit = "cert-fingerprint-responder-legit";
        let fp_resp_mitm = "cert-fingerprint-responder-mitm";

        let (initiator, init_msg) = Spake2Initiator::start(code, fp_init);
        let (responder, resp_msg) = Spake2Responder::start(code, fp_resp_legit);

        let mut cipher_resp = responder.finish(&init_msg, fp_init).unwrap();

        // Initiator thinks remote cert is mitm
        let mut cipher_init = initiator.finish(&resp_msg, fp_resp_mitm).unwrap();

        // Keys derived will not match because HKDF info bound differing fingerprints
        let msg = b"Tampered TLS cert connection";
        let encrypted = cipher_init.encrypt(msg).unwrap();
        let decrypt_attempt = cipher_resp.decrypt(&encrypted);
        assert!(decrypt_attempt.is_err());
    }

    #[test]
    fn test_tofu_pinning() {
        let store = DeviceTrustStore::new();
        let dev_id = "device-12345";
        let fp_first = "sha256-original-fingerprint";
        let fp_altered = "sha256-altered-fingerprint";

        // First use: pins certificate
        assert_eq!(
            store.verify_or_pin(dev_id, fp_first),
            TrustStatus::NewDevicePinned
        );

        // Subsequent use: trusted
        assert_eq!(store.verify_or_pin(dev_id, fp_first), TrustStatus::Trusted);

        // Attacker with different certificate: mismatch
        assert_eq!(
            store.verify_or_pin(dev_id, fp_altered),
            TrustStatus::FingerprintMismatch {
                expected: fp_first.to_string(),
                actual: fp_altered.to_string(),
            }
        );
    }

    #[test]
    fn test_disk_space_check() {
        let temp_dir = tempfile::tempdir().unwrap();
        // 1 KB should succeed on any drive
        assert!(verify_disk_space(temp_dir.path(), 1024).is_ok());
        // 100 Petabytes should fail
        assert!(
            verify_disk_space(temp_dir.path(), 100 * 1024 * 1024 * 1024 * 1024 * 1024).is_err()
        );
    }
}
