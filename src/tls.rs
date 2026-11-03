use anyhow::{Context, Result};
use rcgen::{CertificateParams, KeyPair};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// In-memory TLS identity (certificate + private key + fingerprint).
#[derive(Clone)]
pub struct TlsIdentity {
    pub cert_pem: String,
    pub key_pem: String,
    pub fingerprint: String,
}

/// Installs the Ring crypto provider as the default for rustls if not already installed.
pub fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl TlsIdentity {
    /// Generates a new self-signed certificate and private key for LAN file transfer.
    pub fn generate() -> Result<Self> {
        init_crypto_provider();
        let mut params = CertificateParams::default();
        params.subject_alt_names = vec![
            rcgen::SanType::DnsName("localhost".to_string().try_into()?),
            rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
        ];

        let key_pair = KeyPair::generate()?;
        let cert = params.self_signed(&key_pair)?;

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();
        let fingerprint = compute_cert_fingerprint_from_pem(&cert_pem)?;

        Ok(Self {
            cert_pem,
            key_pem,
            fingerprint,
        })
    }

    /// Computes rustls server configuration.
    pub fn server_config(&self) -> Result<rustls::ServerConfig> {
        let cert_der = pem_to_der(&self.cert_pem)?;
        let key_der = pem_to_der(&self.key_pem)?;

        let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der)];
        let private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(key_der.into());

        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)?;

        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(config)
    }

    /// Computes rustls client configuration that trusts the given certificate PEM.
    pub fn client_config_with_cert(peer_cert_pem: &str) -> Result<rustls::ClientConfig> {
        let peer_der = pem_to_der(peer_cert_pem)?;
        let mut root_store = rustls::RootCertStore::empty();
        root_store.add(rustls::pki_types::CertificateDer::from(peer_der))?;

        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(root_store)
        .with_no_client_auth();

        Ok(config)
    }
}

/// Extracts raw DER bytes from a PEM string.
pub fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    for line in pem.lines() {
        if line.starts_with("-----") {
            continue;
        }
    }
    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    use base64::Engine;
    base64::prelude::BASE64_STANDARD
        .decode(b64.trim())
        .context("Failed to decode PEM base64")
}

/// Computes SHA-256 fingerprint in standard format (AA:BB:CC:...).
pub fn compute_cert_fingerprint_from_pem(pem: &str) -> Result<String> {
    let der = pem_to_der(pem)?;
    let hash = Sha256::digest(&der);
    let hex_pairs: Vec<String> = hash.iter().map(|b| format!("{:02X}", b)).collect();
    Ok(hex_pairs.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_tls_identity() {
        let identity = TlsIdentity::generate().expect("Failed to generate identity");
        assert!(identity.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(identity.key_pem.contains("BEGIN PRIVATE KEY"));
        assert_eq!(identity.fingerprint.split(':').count(), 32);

        let server_cfg = identity.server_config();
        assert!(server_cfg.is_ok());

        let client_cfg = TlsIdentity::client_config_with_cert(&identity.cert_pem);
        assert!(client_cfg.is_ok());
    }
}
