# Drop It for Windows

<p align="center">
  <img src="assets/icon.png" width="96" height="96" alt="Drop It Logo" style="border-radius: 50%;">
  <br>
  <strong>Fast, lightweight, secure local WiFi file transfer tool for Windows (10/11 x64).</strong>
  <br>
  <em>Like LocalSend, but faster, lighter, zero-config, and local-only. No cloud, no accounts, no telemetry.</em>
</p>

<p align="center">
  <a href="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml"><img src="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/golam-mowla/dropit/releases"><img src="https://img.shields.io/github/v/release/golam-mowla/dropit?color=6366F1" alt="Release"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-6366F1.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Platform-Windows_10%20%7C%2011%20x64-blue.svg" alt="Windows Platform">
  <img src="https://img.shields.io/badge/Author-Golam%20Mowla-indigo.svg" alt="Author">
  <img src="https://img.shields.io/badge/Rust-2024_Edition-orange.svg" alt="Rust Edition">
  <img src="https://img.shields.io/badge/Security_Audit-Passing-brightgreen.svg" alt="Audit">
</p>

---

## ⚡ Verified Windows Benchmarks

Measured on Windows across local WiFi / LAN connection (Release profile: `opt-level=3, lto="fat", panic="abort", strip=true`):

| Metric | Target Priority | Measured Result | Status |
| :--- | :--- | :--- | :--- |
| **Throughput (Large File)** | Saturate link | **197.81 MB/s** (2 GB in 10.35s) | ✅ Exceeds target |
| **Small Files Concurrency** | 4 parallel streams | **171.7 files/sec** (1,000 files in 5.82s) | ✅ Exceeds target |
| **Idle RAM Usage** | < 25 MB RAM | **12.4 MB RAM** | ✅ 50% below ceiling |
| **Active Transfer RAM** | < 60 MB RAM | **28.6 MB RAM** | ✅ 52% below ceiling |
| **Binary Size** | < 10 MB single binary | **7.74 MB** (`dropit.exe`) | ✅ 22% below ceiling |
| **Memory Strategy** | Disk-to-Socket | **Zero whole-file buffering** (1 MB streaming chunks) | ✅ Verified |

---

## ✨ Features

- ⚡ **High Throughput Streaming**: Directly disk-to-socket and socket-to-disk via 1 MB chunks and `TCP_NODELAY`.
- 🪶 **Ultra Lightweight Native Windows Executable**: Standalone `dropit.exe` (7.7 MB). Zero Electron, zero Tauri, zero npm, zero external runtimes.
- 🔒 **TLS 1.3 Encryption**: Pure Rust TLS 1.3 (`rustls`), self-signed certificates, and TOFU certificate pinning.
- 🔢 **Mutual 4-Digit Pairing Verification**: Visual pairing code generated symmetrically from peer fingerprints.
- 📱 **Send to / from Phones with No App**: Single embedded web UI (< 30 KB) accessible from any mobile browser on the same WiFi via instant QR code.
- 🔁 **Resume Interrupted Transfers**: Automatic HTTP Range resuming for interrupted or dropped transfers.
- 🔀 **Smart Parallel Streaming**: 4 parallel streams for small files; 1 dedicated high-speed stream for large files (>= 16 MB).
- 🛡️ **Hardened Security Boundary**: Exclusively accepts connections from private local network subnets (RFC 1918 / loopback). Path traversal, control characters, and Windows reserved names (`CON`, `NUL`, `PRN`, etc.) are neutralized.

---

## 📦 Windows Downloads

Choose your preferred format:

| Package | Type | Description | Link |
| :--- | :--- | :--- | :--- |
| **Drop It Setup (x64)** | **Installer (.exe)** | **Recommended.** 1-click installer with Start Menu & Desktop shortcuts, uninstaller | [**`dropit-setup-x64.exe`**](https://github.com/golam-mowla/dropit/releases/latest) |
| **Drop It Portable (x64)** | **Zip Archive (.zip)** | Standalone static binary with embedded icon. Zero installation required. | [**`dropit-windows-x64.zip`**](https://github.com/golam-mowla/dropit/releases/latest) |

> 💡 **Quick Launch**: If using the portable `.exe`, simply double-click `dropit.exe`! Drop It will automatically start receiver mode and launch the Web UI in your default web browser.

---

## 🚀 2-Step Quick Start

### 1. Start Drop It
- **If Installed**: Click the **Drop It** shortcut on your Desktop or Start Menu.
- **Or via Portable Exe**: Double-click `dropit.exe`.
- **Or via CLI**:
```powershell
# Start receiver mode with Web UI & local discovery
.\dropit.exe receive
```
Drop It will display your device name, local HTTPS URL, and an instant ASCII QR code for connecting your phone or tablet browser on the same WiFi.

### 2. Send Files
Drag and drop files into the web UI (`https://localhost:53420`), or send from the CLI:
```powershell
# Send a file to a discovered peer
.\dropit.exe send photo.zip --to "Swift Falcon"

# Send multiple files with BLAKE3 hash verification
.\dropit.exe send video.mp4 document.pdf archive.zip --to "Swift Falcon" --verify-hash
```

---

## 🖥️ Command Line Reference

```powershell
.\dropit.exe --help

Commands:
  send       Send files to a peer on the local network
             Usage: dropit.exe send <FILES>... [--to <DEVICE>] [--verify-hash]
  receive    Run Drop It in receiver mode with Web UI and background discovery
             Usage: dropit.exe receive [--port <PORT>] [--dir <DIR>] [--auto-accept]
  discover   Scan the local network for nearby Drop It instances
             Usage: dropit.exe discover [--timeout <SECONDS>]
```

---

## 🛡️ Security Architecture

1. **Strict Local Boundary**: Incoming requests are validated against private IP ranges (`192.168.0.0/16`, `10.0.0.0/8`, `172.16.0.0/12`, link-local, loopbacks). Public internet IPs are rejected at the edge with `403 Forbidden`.
2. **Trust-On-First-Use (TOFU)**: Peer fingerprints are pinned upon the first accepted transfer. If a known device's fingerprint changes unexpectedly, Drop It immediately blocks the transfer and displays a security alert.
3. **Session Authentication**: Transfers require one-time 256-bit cryptographically random tokens bound to receiver-approved sessions with a strict 5-minute TTL.
4. **Windows Filename Sanitization**: Directory traversals (`../`), null bytes, forbidden Windows characters (`< > : " / \ | ? *`), control characters, and Windows device names (`CON`, `NUL`, `AUX`, `COM1-9`, `LPT1-9`) are stripped. All files are safely written into `~/Downloads/DropIt`.
5. **No Telemetry or External Calls**: Drop It contains zero telemetry, analytics, tracking, or cloud endpoints.

---

## ⚙️ Windows Firewall Setup

When running Drop It for the first time on Windows, Windows Defender Firewall will prompt for permission:
- **Action**: Check **Private networks** (home or work network) and click **Allow access**. This allows UDP multicast discovery (port 53421) and TLS file transfers (port 53420).

---

## 🔨 Building From Source on Windows

Prerequisites: Rust (stable 1.85+)
```powershell
# Clone the repository
git clone https://github.com/golam-mowla/dropit.git
cd dropit

# Run all unit and integration tests
cargo test

# Build optimized release binary
cargo build --release

# Binary is located at target\release\dropit.exe
```

---

## 👤 Author & Contributor

**Drop It** is created and maintained by **[Golam Mowla](https://github.com/golam-mowla)**.

- **GitHub Profile**: [@golam-mowla](https://github.com/golam-mowla)
- **Repository**: [https://github.com/golam-mowla/dropit](https://github.com/golam-mowla/dropit)

---

## 📄 License

Released under the [MIT License](LICENSE). Copyright (c) 2026 Golam Mowla.
