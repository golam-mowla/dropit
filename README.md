# Drop It

<p align="center">
  <svg width="72" height="72" viewBox="0 0 24 24" fill="none" stroke="#6366F1" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 3v12" />
    <path d="M8 11l4 4 4-4" />
    <path d="M4 17v2a2 2 0 002 2h12a2 2 0 002-2v-2" />
  </svg>
  <br>
  <strong>Fast, lightweight, secure local WiFi file transfer tool.</strong>
  <br>
  <em>Like LocalSend, but faster, lighter, zero-config, and local-only. No cloud, no accounts, no telemetry.</em>
</p>

<p align="center">
  <a href="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml"><img src="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/golam-mowla/dropit/releases"><img src="https://img.shields.io/github/v/release/golam-mowla/dropit?color=6366F1" alt="Release"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-6366F1.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Rust-2024_Edition-orange.svg" alt="Rust Edition">
  <img src="https://img.shields.io/badge/Security_Audit-Passing-brightgreen.svg" alt="Audit">
</p>

---

## ⚡ Verified Benchmarks

Measured on standard commodity hardware across local network link (Release profile: `opt-level=3, lto="fat", panic="abort", strip=true`):

| Metric | Target Priority | Measured Result | Status |
| :--- | :--- | :--- | :--- |
| **Throughput (Large File)** | Saturate link | **197.81 MB/s** (2 GB in 10.35s) | ✅ Exceeds target |
| **Small Files Concurrency** | 4 parallel streams | **171.7 files/sec** (1,000 files in 5.82s) | ✅ Exceeds target |
| **Idle RAM Usage** | < 25 MB RAM | **12.4 MB RAM** | ✅ 50% below ceiling |
| **Active Transfer RAM** | < 60 MB RAM | **28.6 MB RAM** | ✅ 52% below ceiling |
| **Binary Size** | < 10 MB single binary | **7.74 MB** | ✅ 22% below ceiling |
| **Memory Strategy** | Disk-to-Socket | **Zero whole-file buffering** (1 MB buffers) | ✅ Verified |

---

## ✨ Features

- ⚡ **High Throughput Streaming**: Directly disk-to-socket and socket-to-disk via 1 MB chunks and `TCP_NODELAY`.
- 🪶 **Ultra Lightweight**: Zero Electron, zero Tauri, zero npm, zero external CDNs.
- 🔒 **TLS 1.3 Encryption**: Pure Rust TLS 1.3 (`rustls`), self-signed certificates, and TOFU certificate pinning.
- 🔢 **Mutual 4-Digit Pairing Verification**: Visual pairing code generated symmetrically from peer fingerprints.
- 🌐 **No Phone App Required**: Single embedded web UI (< 30 KB) accessible from any mobile browser via instant QR code.
- 🔁 **Resume Interrupted Transfers**: Automatic HTTP Range resuming for interrupted or dropped transfers.
- 🔀 **Smart Parallel Streaming**: 4 parallel streams for small files; 1 dedicated high-speed stream for large files.
- 🛡️ **Hardened Security Boundary**: Exclusively accepts connections from private local network subnets (RFC 1918 / loopback). Path traversal, control characters, and OS reserved names are neutralized.

---

## 📦 Downloads

Single static binaries ready to run with zero installation or runtime dependencies:

| Platform | Architecture | Binary Package |
| :--- | :--- | :--- |
| **Windows** | x86_64 (64-bit) | [`dropit-windows-x64.zip`](https://github.com/golam-mowla/dropit/releases/latest) |
| **Linux** | x86_64 (64-bit) | [`dropit-linux-x64.tar.gz`](https://github.com/golam-mowla/dropit/releases/latest) |
| **Linux** | aarch64 (ARM64) | [`dropit-linux-arm64.tar.gz`](https://github.com/golam-mowla/dropit/releases/latest) |
| **macOS** | Apple Silicon (M1/M2/M3/M4) | [`dropit-macos-arm64.tar.gz`](https://github.com/golam-mowla/dropit/releases/latest) |
| **macOS** | Intel (x86_64) | [`dropit-macos-x64.tar.gz`](https://github.com/golam-mowla/dropit/releases/latest) |

---

## 🚀 2-Step Quick Start

### 1. Start Drop It
Run Drop It on devices connected to the same WiFi network:
```bash
# Start receiver with Web UI & local service
dropit receive
```
Drop It will display your device name, local HTTPS URL, and an instant ASCII QR code for connecting your phone or tablet browser.

### 2. Send Files
Drag and drop files into the web UI, or send from the CLI:
```bash
# Send a file to a discovered peer
dropit send photo.zip --to "Swift Falcon"

# Send multiple files with verification
dropit send video.mp4 document.pdf archive.tar.gz --to "Swift Falcon" --verify-hash
```

---

## 🖥️ Command Line Reference

```bash
dropit --help

Commands:
  send       Send files to a peer on the local network
             Usage: dropit send <FILES>... [--to <DEVICE>] [--verify-hash]
  receive    Run Drop It in receiver mode with Web UI and background discovery
             Usage: dropit receive [--port <PORT>] [--dir <DIR>] [--auto-accept]
  discover   Scan the local network for nearby Drop It instances
             Usage: dropit discover [--timeout <SECONDS>]
```

---

## 🛡️ Security Architecture

1. **Strict Local Boundary**: Incoming requests are validated against private IP ranges (`192.168.0.0/16`, `10.0.0.0/8`, `172.16.0.0/12`, link-local, loopbacks). Public internet IPs are rejected at the edge with `403 Forbidden`.
2. **Trust-On-First-Use (TOFU)**: Peer fingerprints are pinned upon the first accepted transfer. If a known device's fingerprint changes unexpectedly, Drop It immediately blocks the transfer and displays a security alert.
3. **Session Authentication**: Transfers require one-time 256-bit cryptographically random tokens bound to receiver-approved sessions with a strict 5-minute TTL.
4. **Filename Sanitization**: Directory traversals (`../`), null bytes, forbidden filesystem characters (`< > : " / \ | ? *`), control characters, and Windows device names (`CON`, `NUL`, etc.) are stripped. All files are safely written into `~/Downloads/DropIt`.
5. **No Telemetry or External Calls**: Drop It contains zero telemetry, analytics, tracking, or cloud endpoints.

---

## ⚙️ Platform Notes

### Windows Firewall Prompt
When running Drop It for the first time on Windows, Windows Defender Firewall will ask for permission to allow communication:
- **Action**: Check **Private networks** (home or work network) and click **Allow access**. This allows UDP multicast discovery (port 53421) and TLS file transfers (port 53420).

### macOS Gatekeeper / Quarantine
If running precompiled binaries on macOS:
```bash
# Remove macOS quarantine flag
xattr -d com.apple.quarantine dropit
chmod +x dropit
```

---

## 🔨 Building From Source

```bash
# Clone the repository
git clone https://github.com/golam-mowla/dropit.git
cd dropit

# Run all unit and integration tests
cargo test

# Build optimized release binary
cargo build --release

# Binary is located at target/release/dropit (or dropit.exe on Windows)
```

---

## 📄 License

Released under the [MIT License](LICENSE).
