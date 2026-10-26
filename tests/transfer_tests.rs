use dropit::client::DropItClient;
use dropit::config::DropItConfig;
use dropit::crypto::{self, TlsIdentity};
use dropit::server::{ServerState, run_server};
use dropit::transfer::{FileMetadata, TransferOfferRequest};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast};

#[tokio::test]
async fn test_secure_file_streaming_roundtrip() {
    // 1. Setup temporary directories for sender and receiver
    let temp_base = std::env::temp_dir().join(format!("dropit_test_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    // 2. Create receiver config and TLS identity
    let mut receiver_config = DropItConfig::default();
    receiver_config.device_name = "Receiver Node".to_string();
    receiver_config.download_dir = receiver_dir.clone();
    receiver_config.auto_accept_trusted = true; // Auto accept for test

    let receiver_identity = TlsIdentity::generate_self_signed("Receiver Node").unwrap();
    let receiver_config_arc = Arc::new(RwLock::new(receiver_config));

    let server_state =
        ServerState::new(receiver_config_arc.clone(), receiver_identity.clone(), None);

    // Pick an available port on loopback
    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let chosen_port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{chosen_port}").parse().unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    tokio::spawn(async move {
        let _ = run_server(server_state, actual_addr, Some(shutdown_rx)).await;
    });

    // Wait for server to bind
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 3. Setup sender identity and create test file (5 MB)
    let sender_identity = TlsIdentity::generate_self_signed("Sender Node").unwrap();
    let test_file_path = sender_dir.join("test_payload.bin");
    let test_payload: Vec<u8> = (0..(5 * 1024 * 1024)).map(|i| (i % 251) as u8).collect();
    std::fs::write(&test_file_path, &test_payload).unwrap();
    let expected_hash = blake3::hash(&test_payload).to_hex().to_string();

    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{chosen_port}");

    // 4. Test GET /api/info
    let info = client.get_device_info(&host_port).await.unwrap();
    assert_eq!(info["name"], "Receiver Node");
    assert_eq!(info["fingerprint"], receiver_identity.fingerprint);

    // 5. Test POST /api/offer
    let session_id = "test-session-1".to_string();
    let meta = FileMetadata {
        file_id: "0".to_string(),
        filename: "test_payload.bin".to_string(),
        size_bytes: test_payload.len() as u64,
        blake3_hash: Some(expected_hash.clone()),
    };

    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: "Sender Node".to_string(),
        sender_device_id: "sender-id-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        files: vec![meta.clone()],
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted, "Offer should be accepted");
    let token = offer_resp.session_token.expect("session token provided");
    let expected_pairing_code =
        crypto::generate_pairing_code(&receiver_identity.fingerprint, &sender_identity.fingerprint);
    assert_eq!(offer_resp.pairing_code, expected_pairing_code);

    // 6. Test PUT /api/upload streaming
    let progress_events = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let progress_events_clone = progress_events.clone();
    let upload_result = client
        .stream_file(
            &host_port,
            &session_id,
            &token,
            &meta,
            &test_file_path,
            move |_progress| {
                progress_events_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .await
        .unwrap();

    assert!(progress_events.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert!(upload_result.verified);
    assert_eq!(upload_result.bytes_written, test_payload.len() as u64);
    assert_eq!(upload_result.blake3_hash, expected_hash);

    // 7. Verify file on disk at receiver destination
    let received_path = receiver_dir.join("test_payload.bin");
    assert!(received_path.exists(), "Received file must exist");
    let received_data = std::fs::read(&received_path).unwrap();
    assert_eq!(received_data.len(), test_payload.len());
    assert_eq!(received_data, test_payload);

    // 8. Verify TOFU: sender fingerprint was pinned in receiver config
    {
        let config = receiver_config_arc.read().await;
        let pinned = config.trusted_devices.get("sender-id-1");
        assert_eq!(pinned, Some(&sender_identity.fingerprint));
    }

    // 9. Shutdown server & cleanup
    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}
