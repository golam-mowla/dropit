use crate::security::is_private_or_loopback_ip;
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{RwLock, broadcast};
use tracing::{debug, error, info, warn};

pub const MULTICAST_IPV4: Ipv4Addr = Ipv4Addr::new(239, 255, 53, 42);
pub const DISCOVERY_PORT: u16 = 53421;
pub const BEACON_INTERVAL: Duration = Duration::from_secs(2);
pub const PEER_EXPIRY_DURATION: Duration = Duration::from_secs(6);
pub const MDNS_SERVICE_TYPE: &str = "_dropit._tcp.local.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceAnnouncement {
    pub device_id: String,
    pub name: String,
    pub fingerprint: String,
    pub port: u16,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoverySource {
    Multicast,
    Mdns,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub announcement: DeviceAnnouncement,
    pub address: SocketAddr,
    pub last_seen: Instant,
    pub source: DiscoverySource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub device_id: String,
    pub name: String,
    pub fingerprint: String,
    pub address: String,
    pub port: u16,
    pub version: String,
    pub source: String,
    pub last_seen_secs_ago: u64,
}

impl DiscoveredPeer {
    pub fn to_info(&self) -> PeerInfo {
        PeerInfo {
            device_id: self.announcement.device_id.clone(),
            name: self.announcement.name.clone(),
            fingerprint: self.announcement.fingerprint.clone(),
            address: self.address.ip().to_string(),
            port: self.announcement.port,
            version: self.announcement.version.clone(),
            source: match self.source {
                DiscoverySource::Multicast => "multicast".to_string(),
                DiscoverySource::Mdns => "mdns".to_string(),
            },
            last_seen_secs_ago: self.last_seen.elapsed().as_secs(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PeerEvent {
    Added(DiscoveredPeer),
    Updated(DiscoveredPeer),
    Removed(String), // device_id
}

#[derive(Clone)]
pub struct PeerRegistry {
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
    event_tx: broadcast::Sender<PeerEvent>,
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerRegistry {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(128);
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PeerEvent> {
        self.event_tx.subscribe()
    }

    pub async fn update_or_insert(
        &self,
        announcement: DeviceAnnouncement,
        address: SocketAddr,
        source: DiscoverySource,
    ) {
        let mut peers = self.peers.write().await;
        let device_id = announcement.device_id.clone();
        let now = Instant::now();

        if let Some(existing) = peers.get_mut(&device_id) {
            existing.last_seen = now;
            existing.address = address;
            existing.source = source;
            if existing.announcement != announcement {
                existing.announcement = announcement.clone();
                let peer_clone = existing.clone();
                let _ = self.event_tx.send(PeerEvent::Updated(peer_clone));
            }
        } else {
            let peer = DiscoveredPeer {
                announcement,
                address,
                last_seen: now,
                source,
            };
            peers.insert(device_id, peer.clone());
            let _ = self.event_tx.send(PeerEvent::Added(peer));
        }
    }

    pub async fn get_active_peers(&self) -> Vec<DiscoveredPeer> {
        let peers = self.peers.read().await;
        let mut list: Vec<DiscoveredPeer> = peers
            .values()
            .filter(|p| p.last_seen.elapsed() < PEER_EXPIRY_DURATION)
            .cloned()
            .collect();
        list.sort_by(|a, b| a.announcement.name.cmp(&b.announcement.name));
        list
    }

    pub async fn get_active_peer_infos(&self) -> Vec<PeerInfo> {
        self.get_active_peers()
            .await
            .into_iter()
            .map(|p| p.to_info())
            .collect()
    }

    pub async fn cleanup_stale_peers(&self) -> Vec<String> {
        let mut peers = self.peers.write().await;
        let mut removed = Vec::new();
        peers.retain(|id, peer| {
            if peer.last_seen.elapsed() >= PEER_EXPIRY_DURATION {
                removed.push(id.clone());
                false
            } else {
                true
            }
        });

        for id in &removed {
            let _ = self.event_tx.send(PeerEvent::Removed(id.clone()));
        }
        removed
    }
}

pub struct DiscoveryManager {
    registry: PeerRegistry,
    announcement: DeviceAnnouncement,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl DiscoveryManager {
    pub fn new(announcement: DeviceAnnouncement, registry: PeerRegistry) -> Self {
        Self {
            registry,
            announcement,
            shutdown_tx: None,
        }
    }

    pub fn registry(&self) -> &PeerRegistry {
        &self.registry
    }

    pub async fn start(&mut self) -> Result<()> {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        // 1. Start UDP Multicast listener
        let registry = self.registry.clone();
        let local_id = self.announcement.device_id.clone();
        let mut shutdown_rx1 = shutdown_tx.subscribe();
        tokio::spawn(async move {
            if let Err(err) = run_multicast_listener(registry, local_id, &mut shutdown_rx1).await {
                error!("Multicast listener terminated with error: {:#}", err);
            }
        });

        // 2. Start UDP Multicast broadcaster
        let announcement = self.announcement.clone();
        let mut shutdown_rx2 = shutdown_tx.subscribe();
        tokio::spawn(async move {
            if let Err(err) = run_multicast_broadcaster(announcement, &mut shutdown_rx2).await {
                error!("Multicast broadcaster terminated with error: {:#}", err);
            }
        });

        // 3. Start mDNS daemon fallback (publish + browse)
        let registry_mdns = self.registry.clone();
        let announcement_mdns = self.announcement.clone();
        let mut shutdown_rx3 = shutdown_tx.subscribe();
        tokio::spawn(async move {
            if let Err(err) =
                run_mdns_discovery(announcement_mdns, registry_mdns, &mut shutdown_rx3).await
            {
                warn!("mDNS discovery fallback encountered an error: {:#}", err);
            }
        });

        // 4. Stale peer pruning task
        let registry_cleanup = self.registry.clone();
        let mut shutdown_rx4 = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let expired = registry_cleanup.cleanup_stale_peers().await;
                        if !expired.is_empty() {
                            debug!("Removed {} expired peer(s)", expired.len());
                        }
                    }
                    _ = shutdown_rx4.recv() => {
                        break;
                    }
                }
            }
        });

        info!(
            "Device discovery started (Multicast + mDNS fallback) for '{}'",
            self.announcement.name
        );
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(ref tx) = self.shutdown_tx {
            let _ = tx.send(());
        }
    }
}

/// Creates a cross-platform UDP multicast socket using socket2
fn create_multicast_socket(port: u16) -> Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("Failed to create UDP socket")?;

    socket
        .set_reuse_address(true)
        .context("Failed to set SO_REUSEADDR")?;

    #[cfg(not(windows))]
    socket
        .set_reuse_port(true)
        .context("Failed to set SO_REUSEPORT")?;

    socket
        .set_nonblocking(true)
        .context("Failed to set non-blocking")?;

    let bind_addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into();
    socket
        .bind(&bind_addr.into())
        .context("Failed to bind UDP multicast socket")?;

    // Join multicast on all available local network interfaces
    let mut joined_any = false;
    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (_name, ip) in interfaces {
            if let IpAddr::V4(ipv4) = ip
                && !ipv4.is_loopback()
                && socket.join_multicast_v4(&MULTICAST_IPV4, &ipv4).is_ok()
            {
                joined_any = true;
            }
        }
    }

    if !joined_any {
        let _ = socket.join_multicast_v4(&MULTICAST_IPV4, &Ipv4Addr::UNSPECIFIED);
    }

    Ok(socket.into())
}

async fn run_multicast_listener(
    registry: PeerRegistry,
    local_device_id: String,
    shutdown: &mut tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let std_socket = create_multicast_socket(DISCOVERY_PORT)?;
    let socket = UdpSocket::from_std(std_socket)?;
    let mut buf = vec![0u8; 4096];

    loop {
        tokio::select! {
            recv_result = socket.recv_from(&mut buf) => {
                match recv_result {
                    Ok((len, peer_addr)) => {
                        if !is_private_or_loopback_ip(&peer_addr.ip()) {
                            continue;
                        }

                        if let Ok(announcement) =
                            serde_json::from_slice::<DeviceAnnouncement>(&buf[..len])
                            && announcement.device_id != local_device_id
                        {
                            let mut peer_http_addr = peer_addr;
                            peer_http_addr.set_port(announcement.port);
                            registry
                                .update_or_insert(
                                    announcement,
                                    peer_http_addr,
                                    DiscoverySource::Multicast,
                                )
                                .await;
                        }
                    }
                    Err(err) => {
                        debug!("UDP multicast recv error: {}", err);
                    }
                }
            }
            _ = shutdown.recv() => {
                break;
            }
        }
    }
    Ok(())
}

async fn run_multicast_broadcaster(
    announcement: DeviceAnnouncement,
    shutdown: &mut tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let multicast_target: SocketAddr = SocketAddrV4::new(MULTICAST_IPV4, DISCOVERY_PORT).into();
    let payload = serde_json::to_vec(&announcement)?;

    let mut interval = tokio::time::interval(BEACON_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = socket.send_to(&payload, multicast_target).await;

                // Also send broadcast to local subnet broadcast addresses if available
                if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
                    for (_name, ip) in interfaces {
                        if let IpAddr::V4(ipv4) = ip
                            && !ipv4.is_loopback()
                        {
                            let octets = ipv4.octets();
                            let bcast_ip = Ipv4Addr::new(octets[0], octets[1], octets[2], 255);
                            let bcast_addr = SocketAddrV4::new(bcast_ip, DISCOVERY_PORT);
                            let _ = socket.send_to(&payload, SocketAddr::V4(bcast_addr)).await;
                        }
                    }
                }
            }
            _ = shutdown.recv() => {
                break;
            }
        }
    }
    Ok(())
}

async fn run_mdns_discovery(
    announcement: DeviceAnnouncement,
    registry: PeerRegistry,
    shutdown: &mut tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let mdns = ServiceDaemon::new().context("Failed to start mDNS daemon")?;
    let instance_name = announcement.device_id.clone();
    let host_name = format!("{}.local.", announcement.device_id);
    let port = announcement.port;

    let properties = [
        ("id", announcement.device_id.as_str()),
        ("name", announcement.name.as_str()),
        ("fingerprint", announcement.fingerprint.as_str()),
        ("version", announcement.version.as_str()),
    ];

    let service_info = ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance_name,
        &host_name,
        "",
        port,
        &properties[..],
    )
    .context("Failed to create mDNS service info")?;

    mdns.register(service_info)
        .context("Failed to register mDNS service")?;

    let monitor = mdns
        .browse(MDNS_SERVICE_TYPE)
        .context("Failed to browse mDNS service")?;

    let local_id = announcement.device_id;

    loop {
        tokio::select! {
            event = monitor.recv_async() => {
                match event {
                    Ok(ServiceEvent::ServiceResolved(info)) => {
                        let props = info.get_properties();
                        let peer_id = props.get_property_val_str("id").unwrap_or_default();
                        if peer_id.is_empty() || peer_id == local_id {
                            continue;
                        }

                        let peer_name = props.get_property_val_str("name").unwrap_or("Unknown").to_string();
                        let peer_fp = props.get_property_val_str("fingerprint").unwrap_or_default().to_string();
                        let peer_ver = props.get_property_val_str("version").unwrap_or("0.1.0").to_string();
                        let peer_port = info.get_port();

                        // Pick the first private IPv4 address
                        for ip in info.get_addresses_v4() {
                            let ip_addr = IpAddr::V4(ip);
                            if is_private_or_loopback_ip(&ip_addr) {
                                let socket_addr = SocketAddr::new(ip_addr, peer_port);
                                let peer_announcement = DeviceAnnouncement {
                                    device_id: peer_id.to_string(),
                                    name: peer_name.clone(),
                                    fingerprint: peer_fp.clone(),
                                    port: peer_port,
                                    version: peer_ver.clone(),
                                };
                                registry.update_or_insert(peer_announcement, socket_addr, DiscoverySource::Mdns).await;
                                break;
                            }
                        }
                    }
                    Ok(ServiceEvent::ServiceRemoved(_service_type, fullname)) => {
                        debug!("mDNS service removed: {}", fullname);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        debug!("mDNS monitor stream error: {}", err);
                        break;
                    }
                }
            }
            _ = shutdown.recv() => {
                let _ = mdns.unregister(&format!("{instance_name}.{MDNS_SERVICE_TYPE}"));
                let _ = mdns.shutdown();
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_peer_registry_insert_and_get() {
        let registry = PeerRegistry::new();
        let announcement = DeviceAnnouncement {
            device_id: "dev-123".to_string(),
            name: "Amber Otter".to_string(),
            fingerprint: "AA:BB:CC".to_string(),
            port: 53420,
            version: "0.1.0".to_string(),
        };
        let addr: SocketAddr = "192.168.1.50:53420".parse().unwrap();

        registry
            .update_or_insert(announcement.clone(), addr, DiscoverySource::Multicast)
            .await;

        let active = registry.get_active_peers().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].announcement.name, "Amber Otter");
        assert_eq!(active[0].address, addr);
    }

    #[tokio::test]
    async fn test_peer_registry_stale_cleanup() {
        let registry = PeerRegistry::new();
        let announcement = DeviceAnnouncement {
            device_id: "dev-stale".to_string(),
            name: "Ghost Falcon".to_string(),
            fingerprint: "11:22:33".to_string(),
            port: 53420,
            version: "0.1.0".to_string(),
        };
        let addr: SocketAddr = "192.168.1.99:53420".parse().unwrap();

        registry
            .update_or_insert(announcement, addr, DiscoverySource::Multicast)
            .await;

        // Manually age the peer
        {
            let mut peers = registry.peers.write().await;
            if let Some(peer) = peers.get_mut("dev-stale") {
                peer.last_seen = Instant::now() - Duration::from_secs(10);
            }
        }

        let active_before = registry.get_active_peers().await;
        assert_eq!(active_before.len(), 0);

        let removed = registry.cleanup_stale_peers().await;
        assert_eq!(removed, vec!["dev-stale"]);
    }
}
