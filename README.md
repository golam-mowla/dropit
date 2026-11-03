<p align="center">
  <img src="assets/logo.png" alt="Drop It" width="128" height="128" />
</p>

# Drop It

Fast, lightweight, secure local WiFi file transfer tool and anonymous end-to-end encrypted messenger for Windows (10/11 x64).

## Features

- **Local WiFi Transfer**: High-speed peer-to-peer file transfer over LAN.
- **TLS 1.3 Encryption**: All traffic encrypted with self-signed X.509 certificates and fingerprint verification.
- **Single-Use Session Tokens**: One-time token required for each transfer session.
- **Hardened Path Sanitization**: Path traversal, alternate data streams (`name:stream`), trailing spaces/dots, and Windows reserved names are rejected or sanitized.
- **Integrity Verification**: Blake3 cryptographic hash verification after transfer.
- **Zero Configuration Discovery**: Automatic device announcement and discovery over UDP broadcast.

## Ports

| Protocol | Port | Description |
|---|---|---|
| TCP | 53420 | HTTPS / TLS 1.3 file transfer server |
| UDP | 53421 | Local network broadcast discovery |

## CLI Usage

### Receive files
```powershell
dropit receive --port 53420 --dir ./downloads
```

### Discover nearby devices
```powershell
dropit discover --timeout 3
```

### Send files
```powershell
dropit send 192.168.1.50 ./photo.png ./document.pdf --port 53420
```

## Security & Privacy

Drop It is designed from the ground up for strict anonymity and defense-in-depth:
- **Zero Telemetry**: No tracking, phone-home metrics, external DNS calls, or cloud telemetry servers.
- **Anonymous Ephemeral Identities**: Peers generate random aliases (format: *Adjective Animal*, e.g., Swift Falcon) and 128-bit randomized device IDs with zero linkage to Windows usernames, SIDs, hostnames, or hardware MAC addresses.
- **SPAKE2 Code Pairing & TOFU**: Connect with a 5-minute single-use N-word-word pairing code (e.g., 7-amber-falcon) authenticated against TLS certificate fingerprints and cached via Trust-On-First-Use (TOFU) pinning.
- **End-to-End Encryption with Forward Secrecy**: Chat messages use an HKDF-SHA256 symmetric hash ratchet, deriving fresh ChaCha20-Poly1305 message keys for every single payload.
- **DPAPI-Protected Storage**: Local chat history on Windows is protected using Data Protection API (CryptProtectData), encrypted transparently with the user's OS credentials.

## Roadmap

- **Internet Relay Support**: Version 1 is strictly direct local LAN. Internet relay and NAT traversal servers will be introduced in future releases as an opt-in relay service.

## Building

Requires Rust 1.85+ (Edition 2024).

```powershell
cargo build --release
```
