use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceAnnouncement {
    pub device_id: String,
    pub name: String,
    pub fingerprint: String,
    pub port: u16,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub announcement: DeviceAnnouncement,
    pub address: SocketAddr,
    pub last_seen_epoch_secs: u64,
}
