# Drop It

[![CI](https://github.com/dropit-app/dropit/actions/workflows/ci.yml/badge.svg)](https://github.com/dropit-app/dropit/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/dropit-app/dropit)](https://github.com/dropit-app/dropit/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-indigo.svg)](https://opensource.org/licenses/MIT)

> **Drop It** is a fast, lightweight, secure file-transfer tool for devices on the same local WiFi network.  
> Like LocalSend, but faster, lighter, zero-config, and local-only.

---

## Highlights

- ⚡ **High Throughput**: Streams directly disk-to-socket and socket-to-disk using 1 MB chunks and TCP optimizations without buffering whole files in memory.
- 🪶 **Ultra Lightweight**: Idle RAM < 25 MB, active transfer < 60 MB, single standalone binary < 10 MB.
- 🔒 **End-to-End Local Security**: TLS 1.3 only, trust-on-first-use (TOFU) certificate pinning, 4-digit verification pairing code, and private IP range enforcement.
- 🌐 **Zero App Required on Phones**: Generates a local QR code in terminal and web UI; transfer files via standard mobile web browsers.
- 🚫 **Zero External Dependencies**: No internet, no cloud, no user accounts, no telemetry, no CDNs.

---

## Downloads

| Platform | Architecture | Binary Package |
| :--- | :--- | :--- |
| **Windows** | x86_64 (64-bit) | [`dropit-windows-x64.zip`](https://github.com/dropit-app/dropit/releases/latest) |
| **Linux** | x86_64 (64-bit) | [`dropit-linux-x64.tar.gz`](https://github.com/dropit-app/dropit/releases/latest) |
| **Linux** | aarch64 (ARM64) | [`dropit-linux-arm64.tar.gz`](https://github.com/dropit-app/dropit/releases/latest) |
| **macOS** | Apple Silicon (M1/M2/M3/M4) | [`dropit-macos-arm64.tar.gz`](https://github.com/dropit-app/dropit/releases/latest) |
| **macOS** | Intel (x86_64) | [`dropit-macos-x64.tar.gz`](https://github.com/dropit-app/dropit/releases/latest) |

---

## 2-Step Quick Start

### 1. Start Drop It
Run Drop It on both devices connected to the same WiFi network:
```bash
# Start receiver / server with web UI
dropit receive
```
Or open the URL or scan the printed QR code from any mobile browser on the same WiFi!

### 2. Send Files
From the UI, drag and drop files onto a discovered device card, or use the CLI:
```bash
# Discover nearby devices and send
dropit send photo.zip --to "Crimson Falcon"
```

---

## Platform Notes & Tips

### Windows Firewall Prompt
When running Drop It for the first time on Windows, Windows Defender Firewall will ask for permission to allow network communication.  
- **Action**: Check **Private networks** (home or work network) and click **Allow access**. This allows UDP multicast discovery and TLS HTTP connections on your local WiFi.

### macOS Unsigned App Warning
If downloading the binary directly on macOS:
```bash
# Remove the quarantine attribute if macOS blocks execution
xattr -d com.apple.quarantine dropit
chmod +x dropit
```

---

## Security Model Summary

- **TLS 1.3 Only**: Self-signed ECDSA certificate generated on first boot; SHA-256 certificate fingerprint acts as persistent device identity.
- **Trust-on-First-Use (TOFU)**: Pinned peer fingerprints with visual 4-digit confirmation pairing code.
- **Strict Network Boundary**: Restricts HTTP transfers exclusively to RFC 1918 private IPv4 subnets (`192.168.x.x`, `10.x.x.x`, `172.16.x.x - 172.31.x.x`) and IPv6 local link addresses.
- **Path Sanitization**: Directory traversal (`../`), null bytes, reserved OS filenames, and absolute paths are rejected. Files are saved strictly in `~/Downloads/DropIt`.

For more details, review [SECURITY.md](SECURITY.md).

---

## Building From Source

Prerequisites: Rust (stable 1.85+)
```bash
# Clone the repository
git clone https://github.com/dropit-app/dropit.git
cd dropit

# Build optimized release binary
cargo build --release

# Binary will be located at target/release/dropit
```

---

## License

Released under the [MIT License](LICENSE).
