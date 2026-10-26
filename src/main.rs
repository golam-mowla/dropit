use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dropit::client::DropItClient;
use dropit::config::{self, DropItConfig};
use dropit::crypto::{self, TlsIdentity};
use dropit::discovery::{DeviceAnnouncement, DiscoveryManager, PeerEvent, PeerRegistry};
use dropit::server::{OfferDecisionRequest, ServerState, run_server};
use dropit::transfer::{FileMetadata, TransferOfferRequest};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "dropit")]
#[command(author = "Golam Mowla <https://github.com/golam-mowla>")]
#[command(version = "0.1.0")]
#[command(about = "Fast, lightweight, secure local WiFi file transfer tool for Windows", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Send one or more files to a discovered device on your local WiFi
    Send {
        /// Files to send
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Name, IP, or Device ID of the target device
        #[arg(short, long)]
        to: Option<String>,

        /// Verify blake3 hash after transfer
        #[arg(long, default_value_t = true)]
        verify_hash: bool,
    },

    /// Start Drop It receiver mode and local web UI
    Receive {
        /// Port to bind the HTTP TLS server (default: 53420)
        #[arg(short, long)]
        port: Option<u16>,

        /// Destination directory to save received files
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Auto-accept transfers from pinned trusted devices
        #[arg(long, default_value_t = false)]
        auto_accept: bool,
    },

    /// Scan and list nearby Drop It devices on the local network
    Discover {
        /// Timeout in seconds to scan for devices
        #[arg(short, long, default_value_t = 5)]
        timeout: u64,
    },
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = run().await {
        eprintln!("\n❌ Drop It Error: {:#}", e);
        #[cfg(windows)]
        {
            println!("\nPress Enter to exit...");
            let mut buf = String::new();
            let _ = std::io::stdin().read_line(&mut buf);
        }
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    crypto::init_crypto_provider();

    let cli = Cli::parse();

    // Load or initialize persistent configuration & TLS identity
    let config_dir = config::DropItConfig::config_dir()?;
    let mut config = config::DropItConfig::load_or_create()?;
    let identity = crypto::TlsIdentity::load_or_generate(&config_dir, &config.device_name)?;

    print_welcome_banner(&config, &identity);

    let announcement = DeviceAnnouncement {
        device_id: config.device_id.clone(),
        name: config.device_name.clone(),
        fingerprint: identity.fingerprint.clone(),
        port: config.port,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    match cli.command {
        Some(Commands::Send {
            files,
            to,
            verify_hash,
        }) => {
            handle_send_command(config, identity, announcement, files, to, verify_hash).await?;
        }

        Some(Commands::Receive {
            port,
            dir,
            auto_accept,
        }) => {
            if let Some(p) = port {
                config.port = p;
            }
            if let Some(d) = dir {
                config.download_dir = d;
            }
            config.auto_accept_trusted = auto_accept;

            handle_receive_command(config, identity, announcement).await?;
        }

        Some(Commands::Discover { timeout }) => {
            handle_discover_command(announcement, timeout).await?;
        }

        None => {
            println!("🚀 No subcommand provided — launching Drop It Web UI & Receiver server...");
            let local_url = format!("https://127.0.0.1:{}", config.port);
            println!("🌐 Opening Web UI in default browser: {}", local_url);
            open_browser(&local_url);
            handle_receive_command(config, identity, announcement).await?;
        }
    }

    Ok(())
}

async fn handle_send_command(
    config: DropItConfig,
    identity: TlsIdentity,
    announcement: DeviceAnnouncement,
    file_paths: Vec<PathBuf>,
    to: Option<String>,
    verify_hash: bool,
) -> Result<()> {
    // 1. Validate files exist
    let mut meta_list = Vec::new();
    let mut total_size = 0u64;

    for (idx, path) in file_paths.iter().enumerate() {
        if !path.exists() {
            bail!("File not found: {:?}", path);
        }
        let m = std::fs::metadata(path)
            .with_context(|| format!("Failed to read metadata for {:?}", path))?;
        if m.is_dir() {
            bail!("Directories cannot be sent directly: {:?}", path);
        }

        let size = m.len();
        total_size += size;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed_file")
            .to_string();

        let blake3_hash = if verify_hash {
            print!("⏳ Hashing {}...", filename);
            let _ = std::io::stdout().flush();
            let h = crypto::hash_file_streaming(path).await?;
            println!(" Done ({})", &h[..8]);
            Some(h)
        } else {
            None
        };

        meta_list.push(FileMetadata {
            file_id: idx.to_string(),
            filename,
            size_bytes: size,
            blake3_hash,
        });
    }

    println!(
        "📁 Files to send: {} ({:.2} MB total)",
        meta_list.len(),
        total_size as f64 / (1024.0 * 1024.0)
    );

    // 2. Discover or resolve target device
    let registry = PeerRegistry::new();
    let mut discovery = DiscoveryManager::new(announcement, registry.clone());
    discovery.start().await?;

    let target_str = match to {
        Some(t) => t,
        None => {
            println!("🔍 Scanning for nearby devices (waiting 3s)...");
            tokio::time::sleep(Duration::from_secs(3)).await;
            let peers = registry.get_active_peers().await;
            if peers.is_empty() {
                discovery.stop();
                bail!("No nearby Drop It devices found. Specify target with `--to <device>`");
            }
            println!("📡 Select a target device:");
            for (i, p) in peers.iter().enumerate() {
                println!("  [{}] {} ({})", i + 1, p.announcement.name, p.address);
            }
            peers[0].announcement.name.clone()
        }
    };

    println!("🔍 Resolving target device '{}'...", target_str);
    let start = std::time::Instant::now();
    let mut target_addr = None;
    let mut target_fp = String::new();
    let mut target_name = target_str.clone();

    // Check if target is a direct IP:Port
    if let Ok(addr) = target_str.parse::<SocketAddr>() {
        target_addr = Some(addr);
    } else if let Ok(ip) = target_str.parse::<std::net::IpAddr>() {
        target_addr = Some(SocketAddr::new(ip, config.port));
    } else {
        while start.elapsed() < Duration::from_secs(5) {
            let peers = registry.get_active_peers().await;
            for p in peers {
                if p.announcement.name.eq_ignore_ascii_case(&target_str)
                    || p.announcement.device_id == target_str
                {
                    target_addr = Some(p.address);
                    target_fp = p.announcement.fingerprint.clone();
                    target_name = p.announcement.name.clone();
                    break;
                }
            }
            if target_addr.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    discovery.stop();

    let target_addr = match target_addr {
        Some(a) => a,
        None => bail!(
            "Could not find target device '{}' on the local network",
            target_str
        ),
    };

    let client = DropItClient::new();
    let host_port = target_addr.to_string();

    // Fetch device info if target fingerprint is not known yet
    if target_fp.is_empty()
        && let Ok(info) = client.get_device_info(&host_port).await
    {
        if let Some(fp) = info.get("fingerprint").and_then(|v| v.as_str()) {
            target_fp = fp.to_string();
        }
        if let Some(n) = info.get("name").and_then(|v| v.as_str()) {
            target_name = n.to_string();
        }
    }

    let pairing_code = crypto::generate_pairing_code(&identity.fingerprint, &target_fp);

    println!("🤝 Connecting to: {} ({})", target_name, host_port);
    println!("🔢 Pairing Confirmation Code: [ {} ]", pairing_code);
    println!("⏳ Waiting for receiver acceptance...");

    let session_id = hex::encode(rand::random::<[u8; 16]>());
    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: config.device_name.clone(),
        sender_device_id: config.device_id.clone(),
        sender_fingerprint: identity.fingerprint.clone(),
        files: meta_list.clone(),
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await?;
    if !offer_resp.accepted {
        bail!(
            "Transfer declined by receiver: {}",
            offer_resp.reason.unwrap_or_else(|| "Unknown reason".into())
        );
    }

    let session_token = offer_resp
        .session_token
        .context("Missing session token from accepted offer")?;
    println!("✅ Offer accepted! Streaming files directly disk -> socket...");

    let total_start = std::time::Instant::now();
    let items: Vec<(FileMetadata, PathBuf)> = meta_list.iter().cloned().zip(file_paths).collect();

    let client = Arc::new(client);
    let results = client
        .send_files_smart(
            host_port.clone(),
            session_id.clone(),
            session_token.clone(),
            items,
            |p| {
                let pct = if p.total_bytes > 0 {
                    (p.bytes_transferred as f64 / p.total_bytes as f64) * 100.0
                } else {
                    100.0
                };
                let eta_str = p
                    .eta_secs
                    .map(|s| format!("{s}s"))
                    .unwrap_or_else(|| "--".into());
                if p.completed {
                    println!(
                        "\n   ✔️ [{}] 100% | {:>6.2} MB/s | {:>6.1} MB",
                        p.filename,
                        p.speed_mb_s,
                        p.total_bytes as f64 / (1024.0 * 1024.0)
                    );
                } else {
                    print!(
                        "\r   [{}] {:>5.1}% | {:>6.2} MB/s | ETA: {:>4} | {:>6.1} / {:>6.1} MB   ",
                        p.filename,
                        pct,
                        p.speed_mb_s,
                        eta_str,
                        p.bytes_transferred as f64 / (1024.0 * 1024.0),
                        p.total_bytes as f64 / (1024.0 * 1024.0)
                    );
                    let _ = std::io::stdout().flush();
                }
            },
        )
        .await?;

    for res in results {
        println!(
            "   ✔️ Verified: {} (blake3: {})",
            res.verified,
            if res.blake3_hash.len() >= 8 {
                &res.blake3_hash[..8]
            } else {
                &res.blake3_hash
            }
        );
    }

    let elapsed = total_start.elapsed().as_secs_f64();
    let overall_speed = if elapsed > 0.05 {
        (total_size as f64 / (1024.0 * 1024.0)) / elapsed
    } else {
        0.0
    };

    println!(
        "\n🎉 Transfer complete! {} file(s) sent in {:.2}s ({:.2} MB/s average).",
        meta_list.len(),
        elapsed,
        overall_speed
    );
    Ok(())
}

async fn handle_receive_command(
    config: DropItConfig,
    identity: TlsIdentity,
    announcement: DeviceAnnouncement,
) -> Result<()> {
    println!("📥 Mode: Receive / TLS 1.3 Server & Web UI");
    println!("🌐 Server Port: {}", config.port);
    println!("📂 Save Path   : {:?}", config.download_dir);

    // Detect primary local IPv4 address
    let mut lan_ip = "127.0.0.1".to_string();
    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in interfaces {
            if let std::net::IpAddr::V4(ipv4) = ip
                && !ipv4.is_loopback()
                && !ipv4.is_link_local()
            {
                lan_ip = ipv4.to_string();
                break;
            }
        }
    }

    let local_url = format!("https://{lan_ip}:{}", config.port);
    println!("\n📱 Open Web UI in browser: {local_url}");

    // Render terminal QR code
    if let Ok(qr_str) = crypto::generate_qr_terminal(&local_url) {
        println!("\nScan with your phone to open the Web UI instantly:");
        println!("{qr_str}");
    }

    let config_arc = Arc::new(RwLock::new(config.clone()));
    let (prompt_tx, mut prompt_rx) = mpsc::channel::<OfferDecisionRequest>(16);

    // Start discovery
    let registry = PeerRegistry::new();
    let mut event_rx = registry.subscribe();
    let mut discovery = DiscoveryManager::new(announcement, registry.clone());
    discovery.start().await?;

    let server_state = ServerState::new(
        config_arc.clone(),
        identity.clone(),
        Some(registry.clone()),
        Some(prompt_tx),
    );

    let bind_addr: SocketAddr = SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        config.port,
    );

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    // Start HTTP/TLS Server in background
    let server_handle = tokio::spawn(async move {
        if let Err(err) = run_server(server_state, bind_addr, Some(shutdown_rx)).await {
            eprintln!("Server error: {:#}", err);
        }
    });

    // Discovery event monitor
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                PeerEvent::Added(peer) => {
                    println!(
                        "🟢 Discovered nearby device: {} ({})",
                        peer.announcement.name, peer.address
                    );
                }
                PeerEvent::Removed(id) => {
                    println!("🔴 Nearby device disappeared: {}", id);
                }
                _ => {}
            }
        }
    });

    // Offer prompt handler
    tokio::spawn(async move {
        while let Some(req) = prompt_rx.recv().await {
            println!("\n🔔 Incoming transfer offer!");
            println!(
                "   From: {} [FP: {}]",
                req.offer.sender_name, req.offer.sender_fingerprint
            );
            println!("   Pairing Confirmation Code: [ {} ]", req.pairing_code);
            println!("   Files ({}) :", req.offer.files.len());
            for f in &req.offer.files {
                println!(
                    "     - {} ({:.2} MB)",
                    f.filename,
                    f.size_bytes as f64 / (1024.0 * 1024.0)
                );
            }

            if req.is_trusted {
                println!("   (Device is pinned and trusted)");
            }

            print!("👉 Accept transfer? [Y/n]: ");
            let _ = std::io::stdout().flush();

            // Simple line reading
            let mut line = String::new();
            let accepted = if std::io::stdin().read_line(&mut line).is_ok() {
                let s = line.trim();
                s.is_empty() || s.eq_ignore_ascii_case("y") || s.eq_ignore_ascii_case("yes")
            } else {
                true
            };

            if accepted {
                println!("✅ Transfer accepted! Receiving files...");
            } else {
                println!("❌ Transfer declined.");
            }
            let _ = req.response_tx.send(accepted);
        }
    });

    println!("\n📡 Drop It is ready to receive files on your local WiFi network.");
    println!("💡 Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    println!("\n👋 Shutting down server...");
    let _ = shutdown_tx.send(());
    discovery.stop();
    let _ = server_handle.await;

    Ok(())
}

async fn handle_discover_command(announcement: DeviceAnnouncement, timeout: u64) -> Result<()> {
    println!("🔍 Scanning local network for {timeout} seconds...");
    let registry = PeerRegistry::new();
    let mut event_rx = registry.subscribe();
    let mut discovery = DiscoveryManager::new(announcement, registry.clone());
    discovery.start().await?;

    let start = std::time::Instant::now();
    let scan_duration = Duration::from_secs(timeout);

    while start.elapsed() < scan_duration {
        let remaining = scan_duration.saturating_sub(start.elapsed());
        tokio::select! {
            Ok(event) = event_rx.recv() => {
                match event {
                    PeerEvent::Added(peer) => {
                        println!(
                            "✨ Found: {} ({}) [FP: {}] via {:?}",
                            peer.announcement.name, peer.address, peer.announcement.fingerprint, peer.source
                        );
                    }
                    PeerEvent::Removed(id) => {
                        println!("💨 Disappeared: {}", id);
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(remaining) => {
                break;
            }
        }
    }

    let peers = registry.get_active_peers().await;
    println!("-------------------------------------------------------------------");
    println!("📊 Total active devices found: {}", peers.len());
    for (idx, p) in peers.iter().enumerate() {
        println!(
            "  {}. {} ({}) [FP: {}]",
            idx + 1,
            p.announcement.name,
            p.address,
            p.announcement.fingerprint
        );
    }
    discovery.stop();
    Ok(())
}

fn print_welcome_banner(config: &config::DropItConfig, identity: &crypto::TlsIdentity) {
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    println!("║                           DROP IT                                 ║");
    println!("║       Fast, lightweight, secure local WiFi file transfer          ║");
    println!("╚═══════════════════════════════════════════════════════════════════╝");
    println!("📍 Device Name : {}", config.device_name);
    println!("🆔 Device ID   : {}", config.device_id);
    println!("🔑 Fingerprint : {}", identity.fingerprint);
    println!("📂 Save Path   : {:?}", config.download_dir);
    println!("-------------------------------------------------------------------");
}
