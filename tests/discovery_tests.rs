use dropit::discovery::{
    DeviceAnnouncement, DiscoveryManager, DiscoverySource, PeerEvent, PeerRegistry,
};
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn test_discovery_registry_events() {
    let registry = PeerRegistry::new();
    let mut rx = registry.subscribe();

    let announcement = DeviceAnnouncement {
        device_id: "dev-event-1".to_string(),
        name: "Solar Eagle".to_string(),
        fingerprint: "FE:DC:BA".to_string(),
        port: 53420,
        version: "0.1.0".to_string(),
    };
    let addr: SocketAddr = "10.0.0.42:53420".parse().unwrap();

    registry
        .update_or_insert(announcement.clone(), addr, DiscoverySource::Multicast)
        .await;

    // Receive the Added event
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("should receive event")
        .expect("event valid");

    match event {
        PeerEvent::Added(peer) => {
            assert_eq!(peer.announcement.name, "Solar Eagle");
            assert_eq!(peer.address, addr);
        }
        _ => panic!("Expected PeerEvent::Added"),
    }

    // Update with modified name
    let mut modified = announcement.clone();
    modified.name = "Solar Eagle Prime".to_string();
    registry
        .update_or_insert(modified, addr, DiscoverySource::Multicast)
        .await;

    let event2 = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("should receive event")
        .expect("event valid");

    match event2 {
        PeerEvent::Updated(peer) => {
            assert_eq!(peer.announcement.name, "Solar Eagle Prime");
        }
        _ => panic!("Expected PeerEvent::Updated"),
    }
}

#[tokio::test]
async fn test_discovery_manager_lifecycle() {
    let registry = PeerRegistry::new();
    let announcement = DeviceAnnouncement {
        device_id: "test-lifecycle-id".to_string(),
        name: "Test Node".to_string(),
        fingerprint: "11:22:33:44".to_string(),
        port: 53420,
        version: "0.1.0".to_string(),
    };

    let mut manager = DiscoveryManager::new(announcement, registry);
    let start_res = manager.start().await;
    assert!(start_res.is_ok(), "Discovery manager should start cleanly");

    // Give background tasks a brief moment to spin up
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stop discovery manager
    manager.stop();
}

#[tokio::test]
async fn test_two_nodes_cross_discovery() {
    let registry_a = PeerRegistry::new();
    let announcement_a = DeviceAnnouncement {
        device_id: "node-a-id".to_string(),
        name: "Node Alpha".to_string(),
        fingerprint: "AA:AA:AA:AA".to_string(),
        port: 53420,
        version: "0.1.0".to_string(),
    };
    let mut manager_a = DiscoveryManager::new(announcement_a, registry_a.clone());
    manager_a.start().await.expect("start manager A");

    let registry_b = PeerRegistry::new();
    let announcement_b = DeviceAnnouncement {
        device_id: "node-b-id".to_string(),
        name: "Node Bravo".to_string(),
        fingerprint: "BB:BB:BB:BB".to_string(),
        port: 53422,
        version: "0.1.0".to_string(),
    };
    let mut manager_b = DiscoveryManager::new(announcement_b, registry_b.clone());
    manager_b.start().await.expect("start manager B");

    // Wait up to 3 seconds for broadcast to arrive
    let start = std::time::Instant::now();
    let mut b_discovered_a = false;
    let mut a_discovered_b = false;

    while start.elapsed() < Duration::from_secs(3) {
        let peers_b = registry_b.get_active_peers().await;
        if peers_b
            .iter()
            .any(|p| p.announcement.device_id == "node-a-id")
        {
            b_discovered_a = true;
        }

        let peers_a = registry_a.get_active_peers().await;
        if peers_a
            .iter()
            .any(|p| p.announcement.device_id == "node-b-id")
        {
            a_discovered_b = true;
        }

        if b_discovered_a || a_discovered_b {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    manager_a.stop();
    manager_b.stop();

    // At least one direction of discovery succeeded
    assert!(
        b_discovered_a || a_discovered_b,
        "Nodes should discover each other via LAN/multicast/broadcast"
    );
}
