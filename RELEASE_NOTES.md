### 🚀 Drop It v0.1.0 - Next-Gen Local File Sharing (Windows & Android)

**Author**: [Golam Mowla](https://github.com/golam-mowla)

Drop It is a high-speed, local-network peer-to-peer file transfer system built with strict **TLS 1.3 encryption**, **Windows DPAPI** hardware-tied security, zero third-party DLL dependencies, and a modern 3-column desktop layout.

---

### ✨ What's New in this Build:

#### 1. 🎨 Modern 3-Column Desktop Interface
- **Left Icon Rail**: Dedicated tabs for **Transfers**, **History**, **Favorites**, **Nearby Devices**, and **Settings**, plus Dark/Light theme toggle and Mobile QR pairing button.
- **Device Sidebar**: "This Device" card with active port, live status indicator, and instant **"Visible to nearby devices"** stealth mode toggle.
- **Unified Main Panel**:
  - Connected device header showing **"Encrypted • TLS 1.3 • Same Wi-Fi"**.
  - **Dashed Drop Zone**: Drag & drop any file or folder to send immediately.
  - Quick action buttons: **Send Files** (accent blue) and **Send Folder**.
  - **Transfer Queue Cards**: Real-time progress bar, live speed (`MB/s`), dynamic ETA remaining, and dedicated **Pause**, **Resume**, and **Cancel** buttons.
  - **Bottom Message & Clipboard Bar**: Instant chat messaging and **"Paste & Send Clipboard"** button.

#### 2. ⏯️ Resumable Chunked Transfers (Pause / Resume / Cancel)
- High-efficiency **1 MB chunked streaming** with HTTP Range resumption.
- State-synchronized controls: **Pause**, **Resume**, and **Cancel** controls stay synchronized between sender and receiver.
- Network resilience: Automatic graceful pause on network disruption (`paused (connection lost)`) and seamless resumption upon reconnection.
- Automatic partial file cleanup upon cancellation or SHA-256 integrity failure.

#### 3. 🛡️ Mandatory Security Hardening
- **TLS 1.3 Strict Enforcement**: No fallback to unencrypted or legacy TLS protocols.
- **Windows DPAPI Private Key Protection**: Private keys encrypted directly via Windows Data Protection API (`CryptProtectData`), never stored in plaintext.
- **Strict Peer Fingerprint Pinning**: Peer certificates are pinned upon initial pairing; rogue fingerprint changes are actively blocked.
- **Rate-Limited Pairing**: 6-digit secure pairing codes with 5-attempt rate limit and a 5-minute lockout to eliminate brute-force risk.
- **Pre-Transfer Checks**: Receiver acceptance prompt, Win32 free disk space validation, and filename sanitization against path traversal (`../`) and dangerous scripts.

#### 4. 📱 Full Android Companion App (`android/`)
- Pure **Jetpack Compose** matching the desktop flat design: bottom navigation (**Transfers**, **Devices**, **History**, **Settings**), device list, and transfer cards.
- **Foreground Service** (`DropItTransferService`): Keeps WiFi and CPU active so large transfers continue seamlessly even when the screen is locked.
- **Android Share Sheet Integration**: Select "Share to Drop It" from any app or gallery to send files instantly.

#### 5. 🪟 Windows Native Binary & Installer
- **100% Native & Standalone**: Statically linked with `libunwind.a` — zero missing runtime DLL popups!
- Clean 1-click Inno Setup Windows installer (`dropit-setup-x64.exe`) and portable zip (`dropit-windows-x64.zip`).

---

### ⬇️ Download Links
- **Windows Installer (1-Click)**: [`dropit-setup-x64.exe`](https://github.com/golam-mowla/dropit/releases/download/v0.1.0/dropit-setup-x64.exe)
- **Windows Portable (.zip)**: [`dropit-windows-x64.zip`](https://github.com/golam-mowla/dropit/releases/download/v0.1.0/dropit-windows-x64.zip)
- **SHA-256 Checksums**: [`dropit-setup-x64.sha256`](https://github.com/golam-mowla/dropit/releases/download/v0.1.0/dropit-setup-x64.sha256) | [`dropit-windows-x64.sha256`](https://github.com/golam-mowla/dropit/releases/download/v0.1.0/dropit-windows-x64.sha256)
