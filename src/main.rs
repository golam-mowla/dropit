use anyhow::Result;
use clap::{Parser, Subcommand};
use dropit::config;
use dropit::crypto;
use std::path::PathBuf;
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

        /// Name or IP of the target device
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

    match cli.command {
        Some(Commands::Send {
            files,
            to,
            verify_hash: _,
        }) => {
            println!("🚀 Mode: Send");
            println!("📁 Files to transfer: {}", files.len());
            if let Some(target) = to {
                println!("🎯 Target device: {target}");
            } else {
                println!("🔍 Scanning local network for nearby devices...");
            }
            println!("(Network discovery and client transfer active in Phase 2 & 3)");
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
            println!("🌐 Port: {}", config.port);
            println!("📂 Download directory: {:?}", config.download_dir);
            println!("(HTTP/TLS server and embedded UI active in Phase 3 & 4)");
        }
        None => {
            println!(
                "💡 Run `dropit receive` to wait for incoming transfers, or `dropit send <file>` to transfer."
            );
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
