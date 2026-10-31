# Drop It Upgrade & Architecture Plan (PLAN.md)

**Project**: Drop It (v0.1.0)  
**Author / Maintainer**: Golam Mowla (<https://github.com/golam-mowla>)  
**Role**: Senior Full-Stack & Security Engineer  

---

## 1. Current Architecture & Inspection Summary (STEP 0)

### 1.1 Stack
- **Language & Runtime**: Rust (2024 Edition), `tokio` (1.54) async runtime.
- **Desktop GUI**: Native Windows GUI built with `eframe` / `egui` (0.31). Uses native Windows system tray (`tray-icon` 0.20), Win32 API integration, and Explorer context menu integration (`winreg`).
- **Networking & Server**: `axum` (0.8) and `axum-server` (0.8) with `rustls` (0.23, Ring crypto provider).
- **Client**: `reqwest` (0.13) with `rustls` and streaming response support.
- **Discovery**: 
  - Dual discovery mechanism: UDP Multicast beacons (`239.255.53.42:53421`) every 2 seconds + mDNS service broadcast (`_dropit._tcp.local.`, `mdns-sd` 0.21).
  - Direct reverse registration on incoming HTTP requests (`/api/message`, `/api/offer`) to ensure immediate two-way connectivity even through asymmetric firewall policies.
- **Cryptography & Security**:
  - Self-signed ECDSA P-256 X.509 certificate generated on first boot (`rcgen` 0.14) with SANs for all local network adapter IPs and loopbacks.
  - Device identity is represented by the uppercase colon-separated SHA-256 fingerprint of its DER certificate.
  - Deterministic 4-digit symmetric pairing code calculated via BLAKE3 hash of sorted fingerprints: `blake3(min_fp | DropItPairing | max_fp) % 10000`.
  - Streaming file integrity hashing using BLAKE3 in 1 MB buffers.
  - Network boundary: RFC 1918 private IP guard (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, link-local and loopback).

### 1.2 Transfer Protocol
1. **Offer Negotiation**:
   - Sender issues `POST /api/offer` with `TransferOfferRequest` (session ID, sender name, device ID, fingerprint, files list with sizes and BLAKE3 hashes, sender port).
   - Receiver verifies private IP guard, checks concurrent session limits (max 8) and file count/size boundaries.
   - If peer is pinned/trusted and `auto_accept_trusted` is true, auto-accepts; otherwise, triggers desktop UI prompt.
   - Receiver replies with `TransferOfferResponse` (`accepted: bool`, `session_token`, `pairing_code`).
2. **Data Streaming & Resumption**:
   - Sender queries `HEAD /api/upload/{session_id}/{file_id}` to inspect already-written bytes.
   - Sender uploads via `PUT /api/upload/{session_id}/{file_id}` with `Authorization: Bearer <token>`.
   - Receiver supports HTTP `Content-Range: bytes <start>-<end>/<total>` / `Range: bytes=<start>-`, seeks to `<start>`, and streams into file in 1 MB chunks without buffering whole files in RAM.
   - Receiver computes hash on the fly (or streaming pass on resume) and returns `UploadResult` with verification status.

---

## 2. Gap Analysis (What is Missing vs. Requirements)

| Feature Area | Current State (v0.1.0) | Target State (Upgrade) |
|---|---|---|
| **Desktop Layout** | Two-column layout (sidebar + chat/transfer channel) | 3-Column / Rail layout: **Narrow Icon Rail** (5 items: Transfers, History, Favorites, Nearby, Settings) + **Device Sidebar** (This device card, search, Favorites, Nearby with online indicators) + **Main Panel** |
| **Main Panel Header** | Simple peer name + pairing code | Device name, status subtitle `"Encrypted • TLS 1.3 • Same Wi-Fi"`, secondary `"Send folder"`, and single accent-filled blue `"Send files"` button |
| **Drop Zone & Queue** | Window-wide drag-drop, basic chat cards | Dedicated **dashed drop zone** ("Drop files or folders here", immediate send) + structured **transfer queue** below it (file name, size, progress bar, speed, ETA, status) + pinned bottom message bar |
| **Theme & Design** | Single dark theme, text-based symbols | Strict **flat design**, 8px controls, 12px cards, single blue accent (`#2563EB`), zero gradients/shadows, light + dark theme toggle, clean consistent vector icon rendering |
| **Pause / Resume / Cancel** | Resume via Range headers existed in server, but sender lacked Pause/Resume/Cancel interactive state machine | Full interactive transfer state machine (`queued`, `sending`, `paused`, `completed`, `failed`, `cancelled`). Pause, Resume, and Cancel buttons on every card. Sync between sender and receiver. Cancel deletes partial file. "Cancel all" and "Retry" buttons. Auto-pause on connection drop |
| **Extra Features** | Basic chat and transfer | Transfer History view (Open file, Show in folder, Clear history), clipboard text/link sharing, auto-accept for favorites (OFF by default), per-device save location, desktop notification on transfer finish |
| **Key Storage Security** | Private key stored in plain file `key.pem` | **Windows DPAPI** (`CryptProtectData` / `CryptUnprotectData`) encryption for private keys on Windows. Never plain text |
| **Pairing Code Security** | 4-digit code without expiration/rate limits | **6-digit secure pairing code**, 60-second expiration, single-use, 5-attempt rate limiter with 5-minute cooldown |
| **File Security** | Filename sanitization present | Add free disk space check before acceptance, alert/warn on dangerous executable extensions (`.exe`, `.bat`, `.apk`, `.msi`, `.ps1`, `.sh`), dual SHA-256 + BLAKE3 verification |
| **Visibility & Privacy** | Always announcing | "Visible to nearby devices" toggle and Invisible mode; short pseudonym/alias before pairing |
| **Android Application** | Windows-only | Standalone Android App (Kotlin + Compose) sharing the exact same TLS 1.3, discovery, and transfer protocol; foreground service for background transfers; Android share sheet integration |

---

## 3. Implementation Plan & Module Changes

> [!IMPORTANT]
> **No existing module will be deleted without your explicit consent.** All upgrades will build on, refactor, and extend the existing modules (`src/gui.rs`, `src/transfer.rs`, `src/server.rs`, `src/client.rs`, `src/crypto.rs`, `src/config.rs`, `src/security.rs`).

### Step 1: UI Redesign (Desktop & Android Architecture)
- **`src/gui.rs`**:
  - Implement the **Narrow Icon Rail** on the left (Transfers, History, Favorites, Nearby, Settings).
  - Implement the **Device Sidebar**: "This Device" card with local name, IP, and visibility toggle; search box; collapsible/grouped Favorites and Nearby devices with real-time green pulse indicators.
  - Implement the **Main Panel**:
    - Top header: Connected device name, security badge (`"Encrypted • TLS 1.3 • Same Wi-Fi"`), secondary `"Send folder"`, and primary blue `"Send files"` button.
    - **Dashed Drop Zone**: Flat dashed rectangle with icon and text `"Drop files or folders here"`. Dropping files immediately enqueues and starts transfer.
    - **Transfer Queue**: Cards displaying file icon, filename, size, progress bar, transfer speed (MB/s), ETA remaining, and status badge.
    - **Bottom Bar**: Pinned message input with clipboard share and send buttons.
  - **Theme System**: Clean Flat Light theme and Flat Dark theme with 8px control corner radius, 12px card corner radius, zero gradients/shadows, and single blue accent (`#2563EB`).
  - **Consistent Iconography**: Crisp Tabler/Lucide vector rendering using `egui` painter shapes instead of mismatched emojis or system glyphs.

### Step 2: Pause / Resume / Cancel Engine
- **`src/transfer.rs` & `src/client.rs` & `src/server.rs`**:
  - Enhance `FileTransferStatus`:
    `Queued`, `Sending { chunk_offset: u64 }`, `Paused { offset: u64, reason: PauseReason }`, `Completed`, `Failed(String)`, `Cancelled`.
  - Transfer Protocol Endpoints:
    - `POST /api/transfer/{session_id}/cancel`: Immediately aborts receiver stream, cleans up partial destination file from disk, marks session cancelled.
    - `POST /api/transfer/{session_id}/pause`: Informs peer that transfer is paused.
    - `POST /api/transfer/{session_id}/resume`: Re-synchronizes byte offset and resumes 1 MB chunk stream.
  - Chunked Streaming Client:
    - Sender reads and transmits in 1 MB chunks with `tokio::sync::watch` cancellation and pause tokens.
    - Auto-pause on network socket drops (`"paused (connection lost)"`) with periodic ping; auto-resumes once peer announces back online.
  - UI Controls:
    - Pause / Resume / Cancel buttons per card.
    - Header actions: "Cancel all" and "Retry all failed".

### Step 3: Extra Features
- **Transfer History**:
  - Persistent SQLite or JSON history file storing all completed/failed transfers with timestamp, peer, file name, size, hash, and local path.
  - UI panel with "Open file", "Show in folder", and "Clear history".
- **Clipboard Sharing**:
  - One-click "Share Clipboard" button sending text or links directly to the peer channel.
- **Per-Device Save Location**:
  - Config option per device in `DropItConfig::device_download_dirs: HashMap<String, PathBuf>`.
- **Desktop Notifications**:
  - Native Windows toast notification upon transfer completion.
- **Auto-Accept for Favorites**:
  - Distinct toggle per favorite device (default: OFF) to allow silent incoming transfers only when explicitly enabled by user.

### Step 4: Android App Architecture
- **Protocol Interoperability**:
  - Desktop and Android share identical:
    1. TLS 1.3 HTTP server with self-signed ECDSA P-256 and SHA-256 fingerprint pinning.
    2. UDP Multicast (239.255.53.42:53421) and mDNS (`_dropit._tcp.local.`) announcements.
    3. 6-digit pairing code negotiation and `/api/offer`, `/api/upload`, `/api/transfer/*` endpoints.
- **Implementation Strategy**:
  - **Native Android App (`android/` directory)**: Kotlin + Jetpack Compose with OkHttp / Ktor for HTTP/TLS 1.3, standard Android NSD for mDNS, and MulticastLock for UDP discovery.
  - **Why Native Android with Compose?**
    - 100% compliant with Android Scoped Storage (`MediaStore` & Storage Access Framework).
    - True Android Foreground Service with persistent notification to prevent Android OS from killing transfers when the screen is locked or app is backgrounded.
    - System Share Sheet integration (`ACTION_SEND` and `ACTION_SEND_MULTIPLE`).
    - Android Keystore integration for secure hardware-backed private key storage.
    - Clean matching flat UI matching the desktop design (Bottom navigation: Transfers, Devices, History, Settings).

### Step 5: Security Hardening (Mandatory)
1. **OS Secure Key Storage**:
   - Windows: Use Windows Data Protection API (DPAPI) via `CryptProtectData` / `CryptUnprotectData` to encrypt `key.pem` on disk using the logged-in user's Windows credentials.
   - Android: Android Keystore (`AndroidKeyStore` provider) for ECDSA P-256 key pair.
2. **Pairing Code Overhaul**:
   - 6-digit numeric code from cryptographically secure RNG (`rand::rng()`).
   - 60-second expiration timestamp.
   - Single-use validation token.
   - Brute-force rate limiting: 5 failed attempts locks pairing for 5 minutes.
   - Unpaired devices cannot initiate file transfers without active pairing confirmation.
3. **File Transfer Hardening**:
   - Check available free disk space on target drive before accepting offer (`windows-sys` / `GetDiskFreeSpaceExW` on Windows, `StatFs` on Android). If free space < total offer size + 100 MB buffer, reject with descriptive error.
   - Filename sanitization against path traversal, control chars, reserved names.
   - Detect and flag high-risk file types (`.exe`, `.bat`, `.apk`, `.msi`, `.ps1`, `.sh`, `.vbs`, `.cmd`) with a security warning badge. Never automatically execute received files.
   - Dual cryptographic verification: BLAKE3 streaming hash + SHA-256 hash. If hash mismatches, delete partial/corrupted file and mark failed.
4. **Network & Discovery Privacy**:
   - Invisible Mode toggle: stops sending multicast/mDNS beacons while still allowing inbound connections from paired favorites.
   - Ephemeral alias used during discovery before mutual pairing.
   - Strict local-only network restriction (RFC 1918 + loopback).
5. **Dependency Audit & SECURITY.md**:
   - Run `cargo audit`, enforce zero high-severity vulnerabilities.
   - Expand `SECURITY.md` with complete threat model, attack mitigations, and disclosure process.

### Step 6: Testing Strategy
- Unit tests in `tests/`:
  - 1 MB chunking and Range header math.
  - Pause, resume, and cancellation mechanics.
  - Dual hash calculation and verification.
  - Security suite: path traversal payloads, expired pairing codes, replayed pairing attempts, rate limiter cooldown, fingerprint mismatch blocks, oversized files, low disk space.
- Integration test suite simulating sender and receiver over loopback TLS 1.3.

### Step 7: Build & Packaging
- Windows 10/11 x64 standalone executable and Inno Setup installer.
- Android Gradle build setup with debug APK and release APK signing instructions.
- Small, readable git commits documenting each enhancement.

---

## 4. Confirmation & Next Step

Before proceeding with code modifications:
- No existing modules will be removed.
- All existing security guarantees will be strengthened (TLS 1.3, DPAPI key storage, 6-digit pairing with rate limits, disk space checks, dangerous extension warnings).

Ready to proceed with **Step 1 (UI Redesign)** and **Step 5 (Security Key Storage & Pairing Engine)**.
