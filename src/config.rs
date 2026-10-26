use anyhow::{Context, Result};
use directories::ProjectDirs;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const ADJECTIVES: &[&str] = &[
    "Amber", "Azure", "Brisk", "Cobalt", "Crimson", "Emerald", "Frosty", "Golden", "Indigo",
    "Jade", "Lunar", "Nimble", "Opal", "Polar", "Quick", "Ruby", "Silver", "Solar", "Swift",
    "Velvet",
];

const NOUNS: &[&str] = &[
    "Badger", "Cheetah", "Dolphin", "Eagle", "Falcon", "Fox", "Gazelle", "Hawk", "Jaguar", "Lynx",
    "Otter", "Panther", "Raven", "Robin", "Shadow", "Sparrow", "Tiger", "Viper", "Wolf", "Zephyr",
];

pub fn generate_random_device_name() -> String {
    let mut rng = rand::rng();
    let adj = ADJECTIVES.choose(&mut rng).unwrap_or(&"Swift");
    let noun = NOUNS.choose(&mut rng).unwrap_or(&"Falcon");
    format!("{adj} {noun}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropItConfig {
    pub device_id: String,
    pub device_name: String,
    pub port: u16,
    pub download_dir: PathBuf,
    pub auto_accept_trusted: bool,
    pub verify_hash: bool,
    pub trusted_devices: HashMap<String, String>, // device_id -> sha256 fingerprint
}

impl Default for DropItConfig {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.join("DropIt")))
            .unwrap_or_else(|| PathBuf::from("Downloads").join("DropIt"));

        let random_id = hex::encode(rand::random::<[u8; 16]>());

        Self {
            device_id: random_id,
            device_name: generate_random_device_name(),
            port: 53420,
            download_dir,
            auto_accept_trusted: false,
            verify_hash: true,
            trusted_devices: HashMap::new(),
        }
    }
}

impl DropItConfig {
    pub fn config_dir() -> Result<PathBuf> {
        let proj = ProjectDirs::from("com", "dropit", "dropit")
            .context("Failed to determine application config directory")?;
        let dir = proj.config_dir().to_path_buf();
        if !dir.exists() {
            fs::create_dir_all(&dir).context("Failed to create config directory")?;
        }
        Ok(dir)
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.json"))
    }

    pub fn load_or_create() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file from {:?}", path))?;
            match serde_json::from_str::<Self>(&content) {
                Ok(mut cfg) => {
                    // Ensure download directory exists
                    if !cfg.download_dir.exists() {
                        let _ = fs::create_dir_all(&cfg.download_dir);
                    }
                    if cfg.port == 0 {
                        cfg.port = 53420;
                    }
                    Ok(cfg)
                }
                Err(err) => {
                    tracing::warn!("Failed to parse config, regenerating defaults: {}", err);
                    let cfg = Self::default();
                    cfg.save()?;
                    Ok(cfg)
                }
            }
        } else {
            let cfg = Self::default();
            cfg.save()?;
            Ok(cfg)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)?;
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content)?;
        fs::rename(&temp_path, &path)?;

        if !self.download_dir.exists() {
            let _ = fs::create_dir_all(&self.download_dir);
        }
        Ok(())
    }

    pub fn trust_peer(&mut self, device_id: &str, fingerprint: &str) -> Result<()> {
        self.trusted_devices
            .insert(device_id.to_string(), fingerprint.to_string());
        self.save()
    }

    pub fn is_peer_trusted(&self, device_id: &str, fingerprint: &str) -> PeerTrustStatus {
        match self.trusted_devices.get(device_id) {
            Some(pinned_fp) => {
                if pinned_fp.eq_ignore_ascii_case(fingerprint) {
                    PeerTrustStatus::Trusted
                } else {
                    PeerTrustStatus::FingerprintMismatch {
                        expected: pinned_fp.clone(),
                        actual: fingerprint.to_string(),
                    }
                }
            }
            None => PeerTrustStatus::Untrusted,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeerTrustStatus {
    Trusted,
    Untrusted,
    FingerprintMismatch { expected: String, actual: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_name_generation() {
        let name = generate_random_device_name();
        assert!(!name.is_empty());
        let parts: Vec<&str> = name.split_whitespace().collect();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn test_peer_trust_status() {
        let mut cfg = DropItConfig::default();
        let peer_id = "test-device-1";
        let fp1 = "AA:BB:CC:DD";
        let fp2 = "11:22:33:44";

        assert_eq!(
            cfg.is_peer_trusted(peer_id, fp1),
            PeerTrustStatus::Untrusted
        );

        cfg.trusted_devices
            .insert(peer_id.to_string(), fp1.to_string());
        assert_eq!(cfg.is_peer_trusted(peer_id, fp1), PeerTrustStatus::Trusted);
        assert_eq!(
            cfg.is_peer_trusted(peer_id, fp2),
            PeerTrustStatus::FingerprintMismatch {
                expected: fp1.to_string(),
                actual: fp2.to_string()
            }
        );
    }
}
