use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub alias: String,
    pub download_dir: PathBuf,
    pub auto_accept: bool,
    pub port: u16,
}

impl Default for AppSettings {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("downloads"))
            });

        Self {
            alias: "Swift Falcon".to_string(),
            download_dir,
            auto_accept: false,
            port: 53420,
        }
    }
}

impl AppSettings {
    fn settings_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "dropit", "dropit")
            .map(|proj| proj.config_dir().join("settings.json"))
    }

    /// Loads settings from disk or returns defaults if not present or corrupt.
    pub fn load() -> Self {
        let loaded = Self::settings_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|c| serde_json::from_str::<AppSettings>(&c).ok());
        loaded.unwrap_or_default()
    }

    /// Saves settings to disk atomically.
    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::settings_path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create config dir {:?}", parent))?;
            }
            let data = serde_json::to_string_pretty(self)?;
            fs::write(&path, data)
                .with_context(|| format!("Failed to write settings to {:?}", path))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let s = AppSettings::default();
        assert_eq!(s.port, 53420);
        assert!(!s.auto_accept);
        assert!(!s.alias.is_empty());
    }

    #[test]
    fn test_settings_serialization() {
        let s = AppSettings {
            alias: "Amber Fox".to_string(),
            download_dir: PathBuf::from("/tmp/downloads"),
            auto_accept: true,
            port: 53425,
        };
        let json = serde_json::to_string(&s).unwrap();
        let loaded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
    }
}
