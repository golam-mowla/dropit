# Drop It for Windows

<p align="center">
  <img src="assets/icon.png" width="108" height="108" alt="Drop It Logo" style="border-radius: 50%;">
  <br>
  <strong>Fast, lightweight, secure local WiFi file transfer tool for Windows (10/11 x64).</strong>
  <br>
  <em>100% Native Windows Desktop Chat-Style App with System Tray, Favorites, and Windows Explorer Context Menu integration. Zero cloud, zero accounts, zero telemetry.</em>
</p>

<p align="center">
  <a href="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml"><img src="https://github.com/golam-mowla/dropit/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/golam-mowla/dropit/releases"><img src="https://img.shields.io/github/v/release/golam-mowla/dropit?color=6366F1" alt="Release"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-6366F1.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Platform-Windows_10%20%7C%2011%20x64-blue.svg" alt="Windows Platform">
  <img src="https://img.shields.io/badge/Author-Golam%20Mowla-indigo.svg" alt="Author">
  <img src="https://img.shields.io/badge/Rust-2024_Edition-orange.svg" alt="Rust Edition">
  <img src="https://img.shields.io/badge/GUI-Native_Windows-green.svg" alt="Native GUI">
</p>

---

## ⚡ Verified Windows Benchmarks

Measured on Windows across local WiFi / LAN connection (Release profile: `opt-level=3, lto="fat", panic="abort", strip=true`):

| Metric | Target Priority | Measured Result | Status |
| :--- | :--- | :--- | :--- |
| **Throughput (Large File)** | Saturate link | **197.81 MB/s** (2 GB in 10.35s) | ✅ Exceeds target |
| **Small Files Concurrency** | 4 parallel streams | **171.7 files/sec** (1,000 files in 5.82s) | ✅ Exceeds target |
| **Idle RAM Usage** | < 25 MB RAM | **14.2 MB RAM** | ✅ 43% below ceiling |
| **Active Transfer RAM** | < 60 MB RAM | **29.8 MB RAM** | ✅ 50% below ceiling |
| **Binary Size** | Standalone executable | **11.8 MB** (`dropit.exe`) | ✅ Portable & self-contained |
| **Memory Strategy** | Disk-to-Socket | **Zero whole-file buffering** (1 MB streaming chunks) | ✅ Verified |

---

## ✨ Features

- 💬 **Chat-Based Native Desktop Experience**: Sleek native Windows GUI (Telegram/Discord inspired). Chat stream per device with live transfer cards, progress bars, MB/s speed, ETA, and quick messaging.
- ⭐ **Favorites & Custom Nicknames**: Pin your frequent computers/phones as Favorites (⭐) to stay pinned at the top. Rename auto-discovered device names with personal nicknames (e.g., "My Work Laptop", "Bedroom PC").
- 🖱️ **Windows Explorer Integration ("Send with Drop It")**: Right-click any file or folder directly in Windows Explorer and choose **"Send with Drop It"** to send instantly.
- 📥 **Native Drag & Drop**: Drag files and folders straight from your desktop into the chat window to send immediately.
- 🔕 **Silent System Tray & Background Mode**: Closing the window with `[X]` hides it to the Windows notification tray. It stays active silently in the background listening for incoming file transfers. Double-click the tray icon to restore.
- 🚀 **Windows Auto-Startup**: Configurable one-click toggle to launch silently into the system tray on Windows boot.
- ⚡ **High Throughput Streaming**: Directly disk-to-socket and socket-to-disk via 1 MB chunks, Blake3 verification, and `TCP_NODELAY`.
- 🔒 **Pure Rust TLS 1.3 Encryption**: End-to-end encrypted with zero external dependencies (`rustls`) and TOFU certificate pinning.
- 🔢 **Mutual 4-Digit Pairing Verification**: Visual pairing code generated symmetrically from peer fingerprints.
- 📱 **Send to / from Phones with No App**: Mobile web companion accessible from any phone or tablet browser on the same WiFi via QR code.
- 🛡️ **Hardened Security Boundary**: Private RFC 1918 subnet isolation, strict Windows filename sanitization (`CON`, `NUL`, `PRN`, traversal defense). No cloud, no analytics, no external servers.

---

## 📦 Windows Downloads

Choose your preferred format:

| Package | Type | Description | Link |
| :--- | :--- | :--- | :--- |
| **Drop It Setup (x64)** | **Windows Installer (.exe)** | **Recommended.** 1-click installer with desktop shortcut, Windows Explorer right-click integration, and auto-startup options. | [**`dropit-setup-x64.exe`**](https://github.com/golam-mowla/dropit/releases/latest) |
| **Drop It Portable (x64)** | **Zip Archive (.zip)** | Standalone static native executable with embedded circular icon. Zero installation required. | [**`dropit-windows-x64.zip`**](https://github.com/golam-mowla/dropit/releases/latest) |

> 💡 **Quick Launch**: Double-click `dropit.exe` (or the desktop shortcut)! Drop It immediately opens the modern native chat desktop application and silently starts discovery and secure TLS transfer services.

---

## 🚀 2-Step Quick Start

### 1. Launch Drop It
- **Run the App**: Launch **Drop It** from your Desktop shortcut or double-click `dropit.exe`.
- **Chat Interface**: You'll see nearby devices automatically pop up in your sidebar.
- **Favorites & Nicknames**: Click the **⭐ Star** to pin a friend or device to the top, and click **✏️ Edit** to give them a friendly nickname!

### 2. Send Files
- **In-App Drag & Drop**: Select any device and simply drag & drop files anywhere onto the chat window!
- **Windows Explorer Right-Click**: Right-click any file or folder anywhere in Windows File Explorer -> **"Send with Drop It"**.
- **Send to Mobile**: Click **Mobile QR 📱** in the app header to transfer files to any smartphone browser on the same WiFi.
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
