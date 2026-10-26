use anyhow::Result;
use clap::{Parser, Subcommand};
use dropit::config;
use dropit::crypto;
use dropit::discovery::{DeviceAnnouncement, DiscoveryManager, PeerEvent, PeerRegistry};
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "dropit")]
#[command(author = "DropIt Contributors")]
#[command(version = "0.1.0")]
#[command(about = "Fast, lightweight, secure local WiFi file transfer tool", long_about = None)]
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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
            verify_hash: _,
        }) => {
            println!("🚀 Mode: Send");
            println!("📁 Files to transfer: {}", files.len());
            for f in &files {
                let size_str = match std::fs::metadata(f) {
                    Ok(m) => format!("({} bytes)", m.len()),
                    Err(_) => "(not found)".to_string(),
                };
                println!("   - {:?} {}", f, size_str);
            }

            let registry = PeerRegistry::new();
            let mut discovery = DiscoveryManager::new(announcement, registry.clone());
            discovery.start().await?;

            if let Some(target) = to {
                println!("🔍 Searching for target device '{target}'...");
                let start = std::time::Instant::now();
                let mut found_peer = None;

                while start.elapsed() < Duration::from_secs(5) {
                    let peers = registry.get_active_peers().await;
                    for p in peers {
                        if p.announcement.name.eq_ignore_ascii_case(&target)
                            || p.announcement.device_id == target
                            || p.address.ip().to_string() == target
                        {
                            found_peer = Some(p);
                            break;
                        }
                    }
                    if found_peer.is_some() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }

                if let Some(peer) = found_peer {
                    println!(
                        "✅ Target device found: {} ({}) [FP: {}]",
                        peer.announcement.name, peer.address, peer.announcement.fingerprint
                    );
                    println!("(Direct file streaming via TLS 1.3 will connect in Phase 3)");
                } else {
                    println!(
                        "❌ Could not find device '{target}' on the local network within 5 seconds."
                    );
                }
            } else {
                println!("🔍 Scanning for nearby devices (waiting 3s)...");
                tokio::time::sleep(Duration::from_secs(3)).await;
                let peers = registry.get_active_peers().await;
                if peers.is_empty() {
                    println!("ℹ️  No other Drop It devices detected on this WiFi network yet.");
                } else {
                    println!("📡 Nearby devices found:");
                    for p in peers {
                        println!(
                            "   • {} ({}) - FP: {}",
                            p.announcement.name, p.address, p.announcement.fingerprint
                        );
                    }
                }
            }
            discovery.stop();
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

            println!("📥 Mode: Receive / Web UI");
            println!("🌐 Server Port: {}", config.port);
            println!("📂 Download directory: {:?}", config.download_dir);

            let registry = PeerRegistry::new();
            let mut event_rx = registry.subscribe();
            let mut discovery = DiscoveryManager::new(announcement, registry);
            discovery.start().await?;

            println!("📡 LAN discovery active. Listening for nearby devices (Ctrl+C to quit)...");

            tokio::spawn(async move {
                while let Ok(event) = event_rx.recv().await {
                    match event {
                        PeerEvent::Added(peer) => {
                            println!(
                                "🟢 Discovered device: {} ({}) via {:?}",
                                peer.announcement.name, peer.address, peer.source
                            );
                        }
                        PeerEvent::Updated(peer) => {
                            println!(
                                "🔄 Updated device: {} ({})",
                                peer.announcement.name, peer.address
                            );
                        }
                        PeerEvent::Removed(id) => {
                            println!("🔴 Device disappeared: {}", id);
                        }
                    }
                }
            });

            tokio::signal::ctrl_c().await?;
            println!("\n👋 Shutting down...");
            discovery.stop();
        }

        Some(Commands::Discover { timeout }) => {
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
        }

        None => {
            println!(
                "💡 Run `dropit receive` to wait for incoming transfers, or `dropit send <file>` to transfer."
            );
            println!("💡 Run `dropit discover` to scan for nearby devices.");
            println!("💡 Run `dropit --help` for available options.");
        }
    }

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
