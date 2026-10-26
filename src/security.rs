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
    let basename = Path::new(raw_name)
        .file_name()
        .and_then(|n| n.to_str())
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
