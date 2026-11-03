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

Drop It is designed to be fully anonymous:
- No accounts, no email, no phone numbers, and no telemetry.
- Devices use random aliases (e.g. `Swift Falcon`).
- Direct connections communicate peer-to-peer over your local network; external internet access is not required.

## Building

Requires Rust 1.85+ (Edition 2024).

```powershell
cargo build --release
```
