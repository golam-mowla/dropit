use anyhow::{Context, Result};
use clap::Parser;
use dropit::cli::{Cli, Commands};
use dropit::client::DropItClient;
use dropit::discovery::{self, DeviceAnnouncement};
use dropit::pairing::SessionTokenStore;
use dropit::server::{self, ServerState};
use dropit::tls::TlsIdentity;
use std::net::SocketAddr;
use std::time::Duration;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Send(send_args) => {
            info!(
                "Sending {} file(s) to {}:{}",
                send_args.files.len(),
                send_args.target,
                send_args.port
            );
            let client = DropItClient::new(&send_args.target, send_args.port)?;
            client
                .send_files(&send_args.files, send_args.verify_hash)
                .await?;
        }
        Commands::Receive(receive_args) => {
            info!("Starting Drop It receiver on port {}", receive_args.port);
            let tls_identity =
                TlsIdentity::generate().context("Failed to generate TLS certificate")?;
            info!(
                "Server certificate fingerprint: {}",
                tls_identity.fingerprint
            );

            let token_store = SessionTokenStore::new();
            let device_id = format!("{:016x}", rand::random::<u64>());
            let device_name = "Swift Falcon".to_string();

            let server_state = ServerState {
                device_id: device_id.clone(),
                name: device_name.clone(),
                download_dir: receive_args.dir,
                token_store,
                cert_fingerprint: tls_identity.fingerprint.clone(),
                auto_accept: receive_args.auto_accept,
            };

            let announcement = DeviceAnnouncement {
                device_id,
                name: device_name,
                port: receive_args.port,
                cert_fingerprint: tls_identity.fingerprint.clone(),
            };

            let (stop_broadcast_tx, stop_broadcast_rx) = tokio::sync::watch::channel(false);
            let (shutdown_server_tx, shutdown_server_rx) = tokio::sync::watch::channel(false);

            // Spawn background discovery broadcaster
            tokio::spawn(async move {
                if let Err(e) = discovery::start_broadcasting(
                    announcement,
                    discovery::DEFAULT_DISCOVERY_PORT,
                    stop_broadcast_rx,
                )
                .await
                {
                    error!("Discovery broadcast error: {}", e);
                }
            });

            // Handle Ctrl-C gracefully
            tokio::spawn(async move {
                if let Ok(()) = tokio::signal::ctrl_c().await {
                    info!("Received shutdown signal (Ctrl+C)");
                    let _ = stop_broadcast_tx.send(true);
                    let _ = shutdown_server_tx.send(true);
                }
            });

            let bind_addr: SocketAddr = format!("0.0.0.0:{}", receive_args.port).parse()?;
            server::run_server(bind_addr, server_state, &tls_identity, shutdown_server_rx).await?;
        }
        Commands::Discover(discover_args) => {
            info!(
                "Scanning for nearby Drop It devices (timeout: {}s)...",
                discover_args.timeout
            );
            let peers = discovery::scan_nearby(
                Duration::from_secs(discover_args.timeout),
                discover_args.port,
            )
            .await?;
            if peers.is_empty() {
                info!("No nearby devices found.");
            } else {
                info!("Found {} device(s):", peers.len());
                for peer in peers {
                    info!(
                        "- {} ({}) at {}:{} [cert: {}]",
                        peer.announcement.name,
                        peer.announcement.device_id,
                        peer.address.ip(),
                        peer.announcement.port,
                        peer.announcement.cert_fingerprint
                    );
                }
            }
        }
    }

    Ok(())
}
