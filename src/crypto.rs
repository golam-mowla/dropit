use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use sha2::{Digest, Sha256};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

pub fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[derive(Clone, Debug)]
pub struct TlsIdentity {
    pub cert_pem: String,
    pub key_pem: String,
    pub cert_der: Vec<u8>,
    pub fingerprint: String, // Colon-separated uppercase SHA-256 hex
}

impl TlsIdentity {
    pub fn cert_path(base_dir: &Path) -> PathBuf {
        base_dir.join("cert.pem")
    }

    pub fn key_path(base_dir: &Path) -> PathBuf {
        base_dir.join("key.pem")
    }

    pub fn load_or_generate(base_dir: &Path, device_name: &str) -> Result<Self> {
        let cert_file = Self::cert_path(base_dir);
        let key_file = Self::key_path(base_dir);

        if cert_file.exists() && key_file.exists() {
            let cert_pem = fs::read_to_string(&cert_file)
                .with_context(|| format!("Failed to read cert from {:?}", cert_file))?;
            let key_pem = fs::read_to_string(&key_file)
                .with_context(|| format!("Failed to read key from {:?}", key_file))?;

            // Parse DER from PEM
            if let Ok(der) = pem_to_der(&cert_pem) {
                let fingerprint = compute_sha256_fingerprint(&der);
                return Ok(Self {
                    cert_pem,
                    key_pem,
                    cert_der: der,
                    fingerprint,
                });
            }
        }

        // Generate new self-signed certificate
        let identity = Self::generate_self_signed(device_name)?;
        fs::write(&cert_file, &identity.cert_pem)?;
        fs::write(&key_file, &identity.key_pem)?;
        Ok(identity)
    }

    pub fn generate_self_signed(device_name: &str) -> Result<Self> {
        init_crypto_provider();
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .context("Failed to generate ECDSA P256 key pair")?;

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, format!("DropIt - {device_name}"));
        dn.push(DnType::OrganizationName, "DropIt Local Network");
        params.distinguished_name = dn;

        // Collect SANs: localhost, loopbacks, and all local network interfaces
        let mut subject_alt_names = vec![
            SanType::DnsName("localhost".to_string().try_into()?),
            SanType::IpAddress(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            SanType::IpAddress(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
            SanType::IpAddress(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
            SanType::IpAddress(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)),
        ];

        if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
            for (_name, ip) in interfaces {
                subject_alt_names.push(SanType::IpAddress(ip));
            }
        }

        params.subject_alt_names = subject_alt_names;

        let cert = params
            .self_signed(&key_pair)
            .context("Failed to self-sign certificate")?;
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();
        let cert_der = cert.der().to_vec();
        let fingerprint = compute_sha256_fingerprint(&cert_der);

        Ok(Self {
            cert_pem,
            key_pem,
            cert_der,
            fingerprint,
        })
    }
}

pub fn compute_sha256_fingerprint(der_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(der_bytes);
    let result = hasher.finalize();
    result
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<String>>()
        .join(":")
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let lines: Vec<&str> = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .map(str::trim)
        .collect();
    let b64 = lines.join("");
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    STANDARD
        .decode(b64)
        .context("Failed to decode certificate base64")
}

pub fn generate_session_token() -> String {
    let token_bytes: [u8; 32] = rand::random();
    hex::encode(token_bytes)
}

/// Computes a deterministic 4-digit pairing confirmation code between two peer fingerprints.
/// Sorts the fingerprints so both sides calculate the exact same 4 digits.
pub fn generate_pairing_code(fp1: &str, fp2: &str) -> String {
    let mut sorted = [fp1, fp2];
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(sorted[0].as_bytes());
    hasher.update(b"|DropItPairing|");
    hasher.update(sorted[1].as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    let num = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 10000;
    format!("{num:04}")
}

/// Generates an ASCII/Unicode QR code suitable for printing directly into terminal stdout
pub fn generate_qr_terminal(content: &str) -> Result<String> {
    let code = qrcode::QrCode::new(content.as_bytes()).context("Failed to generate QR code")?;
    let render = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();
    Ok(render)
}

/// Generates an SVG XML string representing a QR code
pub fn generate_qr_svg(content: &str) -> Result<String> {
    let code = qrcode::QrCode::new(content.as_bytes()).context("Failed to generate QR code")?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .build();
    Ok(svg)
}

/// Compute blake3 hash of a byte slice
pub fn hash_bytes(data: &[u8]) -> String {
    let hash = blake3::hash(data);
    hash.to_hex().to_string()
}

/// Compute blake3 hash of a file on disk by streaming in 1 MB chunks without buffering whole file in RAM
pub async fn hash_file_streaming(path: &Path) -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("Failed to open file for streaming hash: {path:?}"))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024]; // 1 MB buffer
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cert_generation_and_fingerprint() {
        let identity = TlsIdentity::generate_self_signed("Test Node").unwrap();
        assert!(!identity.cert_pem.is_empty());
        assert!(!identity.key_pem.is_empty());
        assert!(!identity.fingerprint.is_empty());
        // SHA-256 fingerprint has 32 bytes = 32 hex pairs separated by colons
        let parts: Vec<&str> = identity.fingerprint.split(':').collect();
        assert_eq!(parts.len(), 32);
    }

    #[test]
    fn test_session_token_entropy() {
        let t1 = generate_session_token();
        let t2 = generate_session_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 64); // 32 bytes in hex
    }

    #[test]
    fn test_pairing_code_symmetry() {
        let fp_a = "AA:BB:CC:DD";
        let fp_b = "11:22:33:44";
        let code1 = generate_pairing_code(fp_a, fp_b);
        let code2 = generate_pairing_code(fp_b, fp_a);
        assert_eq!(code1, code2);
        assert_eq!(code1.len(), 4);
        assert!(code1.chars().all(|c| c.is_ascii_digit()));
    }
}
