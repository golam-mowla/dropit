# Changelog

All notable changes to the **Drop It** project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.2.0] - 2026-11-03

### Added
- **Glassmorphism Desktop GUI (`eframe`/`egui`)**:
  - Full-featured Windows 10/11 desktop GUI featuring modern glass cards (`#4F46E5` -> `#A855F7` -> `#EC4899`).
  - Windows Acrylic / Mica backdrop integration via Win32 `DwmSetWindowAttribute`.
  - Drag-and-drop file transfer dropzone with progress bar and speed tracking.
  - Interactive nearby LAN peer discovery sidebar with one-click pairing.
  - Unified chat and file thread for peer-to-peer conversations.
  - Settings panel with instant alias regeneration and auto-accept toggles.
  - Native window icon embedded into the executable via `winres`.

- **Anonymous End-to-End Encrypted Messenger**:
  - Forward-secret symmetric ratchet protocol powered by HKDF-SHA256 and ChaCha20-Poly1305.
  - Shared thread combining instant text messaging, file bubbles, typing indicators, and delivery receipts.
  - Secure local chat persistence encrypted using Windows Data Protection API (DPAPI).
  - Length-prefixed framing for tamper-proof message transport over mutual TLS.

- **Strict Anonymity & Zero Telemetry**:
  - Human-friendly random device aliases (e.g., *Swift Falcon*, *Neon Comet*) generated locally from built-in word pools.
  - Isolated 128-bit pseudorandom device identifiers with no device fingerprinting or MAC address exposure.
  - Complete elimination of external telemetry, tracking, and remote phoning.
  - Configurable settings store with customizable alias and auto-accept options.

- **LocalSend-Style Peer-to-Peer Transfer Engine**:
  - Direct LAN transfers over mutual TLS 1.3 (TCP port 53420) and UDP discovery broadcast (port 53421).
  - Explicit Accept / Reject transfer dialog and session handshake with token verification.
  - High-performance recursive directory streaming and transfer cancellation support.
  - In-flight BLAKE3 chunk hashing and automatic receiver disk-space pre-allocation checks.
  - Embedded zero-dependency Web UI (`http://<ip>:53420`) for browser transfers.

- **Branding & Distribution**:
  - Multi-resolution Windows application icon (`assets/icon.ico`) with sizes 16px, 24px, 32px, 48px, 64px, 128px, and 256px.
  - Monochromatic and status-indicator tray icons (idle, connected, unread message).
  - Inno Setup installer script (`installer/dropit.iss`) to package `dropit-setup.exe`.
  - Automated release pipeline (.github/workflows/release.yml) generating portable `.zip`, standalone `.exe`, installer, and `SHA256SUMS.txt`.

---

## [0.1.0] - 2026-11-03

### Added
- Baseline LAN command-line file transfer tool (`send`, `receive`, `discover`).
- Custom UDP announcement discovery protocol on port 53421.
- Self-signed TLS 1.3 certificate generation via `rcgen` and `rustls` on port 53420.
- Basic BLAKE3 hash verification for file integrity.
