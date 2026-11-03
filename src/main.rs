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

fn run_gui() -> Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Drop It")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_transparent(true),
        ..Default::default()
    };

    eframe::run_native(
        "Drop It",
        native_options,
        Box::new(|_cc| Ok(Box::new(dropit::gui::DropItApp::new_standalone()))),
    )
    .map_err(|e| anyhow::anyhow!("GUI execution error: {:?}", e))
}

#[tokio::main]
async fn main() -> Result<()> {
    dropit::tls::init_crypto_provider();
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Gui) => {
            info!("Launching Drop It glassmorphism desktop GUI");
            run_gui()
        }
        Some(Commands::Send(send_args)) => {
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
            Ok(())
        }
        Some(Commands::Receive(receive_args)) => {
            info!("Starting Drop It receiver on port {}", receive_args.port);
            let tls_identity =
                TlsIdentity::generate().context("Failed to generate TLS certificate")?;
            info!(
                "Server certificate fingerprint: {}",
                tls_identity.fingerprint
            );

            let token_store = SessionTokenStore::new();
            let pairing_code = token_store.pairing_session().generate_code();
            info!("Pairing code: {} (valid for 5 minutes)", pairing_code);

            let settings = dropit::settings::AppSettings::load();
            let device_id = dropit::identity::generate_anonymous_device_id();
            let device_name = settings.alias;

            let server_state = ServerState::new(
                device_id.clone(),
                device_name.clone(),
                receive_args.dir,
                token_store,
                tls_identity.fingerprint.clone(),
                receive_args.auto_accept,
            );

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
            Ok(())
        }
        Some(Commands::Discover(discover_args)) => {
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
            Ok(())
        }
    }
}
