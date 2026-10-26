# Security Policy

## Threat Model & Principles

Drop It is built with security-first design for local peer-to-peer file transfers.

1. **Local-Only Boundary**:
   - The server only binds and accepts connections from RFC 1918 private IPv4 networks (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) and loopback addresses.
   - Any external or public IP connection attempts are rejected immediately.
2. **Cryptographic Identity & TLS 1.3**:
   - Every node creates an ECDSA P-256 self-signed certificate upon initial startup.
   - The SHA-256 fingerprint of the certificate is the cryptographic identity of the device.
   - All network traffic is encrypted with TLS 1.3 only; older TLS versions are disabled.
3. **Trust-On-First-Use (TOFU) & Pairing**:
   - A 4-digit numeric code is derived from the peer public keys and displayed on both devices for out-of-band verification.
   - Once verified, peer fingerprints are pinned locally.
   - If a known device identity/fingerprint changes, Drop It alerts the user and halts transfers.
4. **Path Sanitization**:
   - Filenames are rigorously sanitized to prevent directory traversal (`../`), null-byte injection, control characters, Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`), and overwriting existing files without suffix disambiguation.
   - All files are contained within the designated download directory (`~/Downloads/DropIt`).
5. **Privacy**:
   - No telemetry, no external DNS lookups, no cloud servers, no analytics, no persistent file logging.

## Reporting a Vulnerability

If you discover a security vulnerability within Drop It, please report it responsibly by opening a Private Security Advisory on GitHub or contacting the maintainers directly. Do not report security vulnerabilities through public GitHub issues.
