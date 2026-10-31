use anyhow::{Result, bail};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Checks whether an incoming connection comes from a valid private or loopback IP.
pub fn is_private_or_loopback_ip(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(ipv4) => {
            // Loopback 127.0.0.0/8
            if ipv4.is_loopback() {
                return true;
            }
            // RFC 1918 Private ranges:
            // 10.0.0.0/8
            // 172.16.0.0/12
            // 192.168.0.0/16
            // Link-local 169.254.0.0/16
            let octets = ipv4.octets();
            match octets[0] {
                10 => true,
                172 if (16..=31).contains(&octets[1]) => true,
                192 if octets[1] == 168 => true,
                169 if octets[1] == 254 => true,
                _ => false,
            }
        }
        IpAddr::V6(ipv6) => {
            if ipv6.is_loopback() {
                return true;
            }
            // Check for IPv4-mapped IPv6 address (::ffff:x.x.x.x)
            if let Some(ipv4) = ipv6.to_ipv4_mapped() {
                return is_private_or_loopback_ip(&IpAddr::V4(ipv4));
            }
            // IPv6 Unique Local Address (fc00::/7) or Link-Local (fe80::/10)
            let segments = ipv6.segments();
            let first = segments[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

pub fn validate_client_address(peer_addr: SocketAddr) -> Result<()> {
    if !is_private_or_loopback_ip(&peer_addr.ip()) {
        bail!(
            "Access denied: Connection from non-private IP {}",
            peer_addr.ip()
        );
    }
    Ok(())
}

/// Sanitizes untrusted remote filename to prevent directory traversal and attacks:
/// - Strips path components, directory separators, and null bytes
/// - Strips control characters
/// - Replaces forbidden characters (< > : " / \ | ? *) with underscores
/// - Guards against Windows reserved names (CON, NUL, COM1, etc.)
/// - Fallbacks to "unnamed_file" if empty
pub fn sanitize_filename(raw_name: &str) -> String {
    // Basename extraction: split by both '/' and '\'
    let basename = raw_name
        .split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .unwrap_or("unnamed_file");

    let mut clean: String = basename
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();

    let trimmed = clean.trim_matches(|c| c == ' ' || c == '.');
    if trimmed.is_empty() {
        clean = "unnamed_file".to_string();
    } else {
        clean = trimmed.to_string();
    }

    // Check Windows reserved names
    let stem = clean.split('.').next().unwrap_or(&clean);
    for reserved in WINDOWS_RESERVED_NAMES {
        if stem.eq_ignore_ascii_case(reserved) {
            clean = format!("_{clean}");
            break;
        }
    }

    clean
}

/// Resolves destination path in target directory:
/// - Reuses existing file if it is an incomplete partial transfer (< expected_size) to allow resume.
/// - Otherwise resolves duplicate files by appending ` (1)`, ` (2)`, etc.
pub fn resolve_destination_path(dir: &Path, filename: &str, expected_size: u64) -> PathBuf {
    let clean_name = sanitize_filename(filename);
    let target = dir.join(&clean_name);
    if !target.exists() {
        return target;
    }

    // If existing file is partial (< expected_size), reuse it to allow resume
    if let Ok(m) = target.metadata()
        && expected_size > 0
        && m.len() < expected_size
    {
        return target;
    }

    let p = Path::new(&clean_name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let mut counter = 1;
    loop {
        let candidate = dir.join(format!("{stem} ({counter}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(windows)]
pub fn get_available_disk_space(dir: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut path_wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    path_wide.push(0);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let mut free_bytes_to_caller = 0u64;
    let mut total_bytes = 0u64;
    let mut total_free = 0u64;
    let ret = unsafe {
        GetDiskFreeSpaceExW(
            path_wide.as_ptr(),
            &mut free_bytes_to_caller,
            &mut total_bytes,
            &mut total_free,
        )
    };
    if ret == 0 {
        if let Some(parent) = dir.parent() {
            let mut parent_wide: Vec<u16> = parent.as_os_str().encode_wide().collect();
            parent_wide.push(0);
            let ret2 = unsafe {
                GetDiskFreeSpaceExW(
                    parent_wide.as_ptr(),
                    &mut free_bytes_to_caller,
                    &mut total_bytes,
                    &mut total_free,
                )
            };
            if ret2 != 0 {
                return Ok(free_bytes_to_caller);
            }
        }
        bail!("Failed to query disk free space");
    }
    Ok(free_bytes_to_caller)
}

#[cfg(not(windows))]
pub fn get_available_disk_space(_dir: &Path) -> Result<u64> {
    Ok(u64::MAX)
}

pub fn check_free_space(dir: &Path, required_bytes: u64) -> Result<()> {
    let available = get_available_disk_space(dir)?;
    let safety_margin = 50 * 1024 * 1024u64; // 50 MB safety margin
    if available < required_bytes.saturating_add(safety_margin) {
        bail!(
            "Insufficient free disk space (available: {} MB, required: {} MB)",
            available / (1024 * 1024),
            required_bytes / (1024 * 1024)
        );
    }
    Ok(())
}

pub fn is_dangerous_executable(filename: &str) -> bool {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "exe"
            | "bat"
            | "cmd"
            | "msi"
            | "apk"
            | "ps1"
            | "sh"
            | "vbs"
            | "com"
            | "scr"
            | "reg"
            | "jar"
            | "pif"
    )
}

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct PairingRateLimiter {
    attempts: Arc<RwLock<HashMap<IpAddr, (u32, Instant)>>>,
}

impl PairingRateLimiter {
    pub async fn check_and_record_failure(&self, ip: IpAddr) -> Result<(), &'static str> {
        let mut map = self.attempts.write().await;
        let now = Instant::now();
        if let Some((count, last_try)) = map.get_mut(&ip) {
            if now.duration_since(*last_try) > Duration::from_secs(300) {
                *count = 1;
                *last_try = now;
                Ok(())
            } else if *count >= 5 {
                Err("Too many failed pairing attempts. Locked for 5 minutes.")
            } else {
                *count += 1;
                *last_try = now;
                Ok(())
            }
        } else {
            map.insert(ip, (1, now));
            Ok(())
        }
    }

    pub async fn is_locked(&self, ip: &IpAddr) -> bool {
        let map = self.attempts.read().await;
        if let Some((count, last_try)) = map.get(ip) {
            *count >= 5 && last_try.elapsed() < Duration::from_secs(300)
        } else {
            false
        }
    }

    pub async fn reset(&self, ip: &IpAddr) {
        let mut map = self.attempts.write().await;
        map.remove(ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_private_ip_detection() {
        assert!(is_private_or_loopback_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_or_loopback_ip(&"10.0.1.50".parse().unwrap()));
        assert!(is_private_or_loopback_ip(&"192.168.1.100".parse().unwrap()));
        assert!(is_private_or_loopback_ip(&"172.20.5.10".parse().unwrap()));
        assert!(is_private_or_loopback_ip(&"::1".parse().unwrap()));

        // Public IPs should be rejected
        assert!(!is_private_or_loopback_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_or_loopback_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_or_loopback_ip(&"172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(r"..\..\Windows\System32"), "System32");
        assert_eq!(sanitize_filename("valid_photo.jpg"), "valid_photo.jpg");
        assert_eq!(sanitize_filename("bad:name?.txt"), "bad_name_.txt");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("nul"), "_nul");
        assert_eq!(sanitize_filename("..."), "unnamed_file");
        assert_eq!(sanitize_filename(""), "unnamed_file");
    }

    #[test]
    fn test_conflict_rename() {
        let temp_dir = std::env::temp_dir().join("dropit_test_rename");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let initial_file = temp_dir.join("test.txt");
        std::fs::write(&initial_file, "hello").unwrap();

        // When expected_size <= existing size (5 bytes), it renames to test (1).txt
        let candidate = resolve_destination_path(&temp_dir, "test.txt", 5);
        assert_eq!(candidate, temp_dir.join("test (1).txt"));

        // When expected_size > existing size (e.g. 100 bytes), it reuses test.txt for resume
        let resumable = resolve_destination_path(&temp_dir, "test.txt", 100);
        assert_eq!(resumable, temp_dir.join("test.txt"));

        std::fs::write(&candidate, "hello 2").unwrap();
        let candidate2 = resolve_destination_path(&temp_dir, "test.txt", 5);
        assert_eq!(candidate2, temp_dir.join("test (2).txt"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
