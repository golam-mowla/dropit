use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tracing::{debug, error};

pub const DEFAULT_DISCOVERY_PORT: u16 = 53421;
pub const DEFAULT_SERVER_PORT: u16 = 53420;

/// Broadcast announcement payload for LAN discovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceAnnouncement {
    pub device_id: String,
    pub name: String,
    pub port: u16,
    pub cert_fingerprint: String,
}

impl DeviceAnnouncement {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("Failed to serialize announcement")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).context("Failed to parse announcement")
    }
}

/// Discovered peer information including remote socket address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub announcement: DeviceAnnouncement,
    pub address: SocketAddr,
}

/// Creates a reusable UDP broadcast socket bound to the specified port.
pub fn create_broadcast_socket(port: u16) -> Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(not(windows))]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;

    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    socket.bind(&bind_addr.into())?;
    Ok(socket.into())
}

/// Scans the local network for nearby Drop It instances for `timeout` duration.
pub async fn scan_nearby(timeout_dur: Duration, port: u16) -> Result<Vec<DiscoveredPeer>> {
    let std_socket = create_broadcast_socket(port)?;
    let socket = tokio::net::UdpSocket::from_std(std_socket)?;
    let mut peers = Vec::new();

    let deadline = tokio::time::Instant::now() + timeout_dur;
    let mut buf = [0u8; 2048];

    // Broadcast a probe ping
    let broadcast_target = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, port));
    let _ = socket
        .send_to(b"DROPIT_DISCOVER_PROBE", broadcast_target)
        .await;

    while tokio::time::Instant::now() < deadline {
        let sleep_time = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::select! {
            res = socket.recv_from(&mut buf) => {
                match res {
                    Ok((len, addr)) => {
                        if let Ok(announcement) = DeviceAnnouncement::from_bytes(&buf[..len])
                            && !peers.iter().any(|p: &DiscoveredPeer| p.announcement.device_id == announcement.device_id)
                        {
                            peers.push(DiscoveredPeer {
                                announcement,
                                address: addr,
                            });
                        }
                    }
                    Err(e) => {
                        debug!("UDP receive error: {}", e);
                    }
                }
            }
            _ = tokio::time::sleep(sleep_time) => {
                break;
            }
        }
    }

    Ok(peers)
}

/// Broadcasts a device announcement every 2 seconds until cancelled.
pub async fn start_broadcasting(
    announcement: DeviceAnnouncement,
    port: u16,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let std_socket = create_broadcast_socket(0)?;
    let socket = tokio::net::UdpSocket::from_std(std_socket)?;
    let payload = announcement.to_bytes()?;
    let broadcast_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, port));

    loop {
        if *stop_rx.borrow() {
            break;
        }
        if let Err(e) = socket.send_to(&payload, broadcast_addr).await {
            error!("Failed to broadcast announcement: {}", e);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announcement_roundtrip() {
        let announcement = DeviceAnnouncement {
            device_id: "test-id-123".to_string(),
            name: "Swift Falcon".to_string(),
            port: 53420,
            cert_fingerprint: "AA:BB:CC:DD".to_string(),
        };

        let bytes = announcement.to_bytes().expect("Failed to serialize");
        let parsed = DeviceAnnouncement::from_bytes(&bytes).expect("Failed to deserialize");
        assert_eq!(announcement, parsed);
    }
}
