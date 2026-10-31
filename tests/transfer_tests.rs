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
    let receiver_config = DropItConfig {
        device_name: "Receiver Node".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };

    let receiver_identity = TlsIdentity::generate_self_signed("Receiver Node").unwrap();
    let receiver_config_arc = Arc::new(RwLock::new(receiver_config));

    let server_state = ServerState::new(
        receiver_config_arc.clone(),
        receiver_identity.clone(),
        None,
        None,
    );

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
        sha256_hash: None,
    };

    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: "Sender Node".to_string(),
        sender_device_id: "sender-id-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
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

#[tokio::test]
async fn test_resume_interrupted_transfer() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_resume_test_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Resume Receiver".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };

    let receiver_identity = TlsIdentity::generate_self_signed("Resume Receiver").unwrap();
    let receiver_config_arc = Arc::new(RwLock::new(receiver_config));

    let server_state = ServerState::new(
        receiver_config_arc.clone(),
        receiver_identity.clone(),
        None,
        None,
    );

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    tokio::spawn(async move {
        let _ = run_server(server_state, actual_addr, Some(shutdown_rx)).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5 MB test file
    let total_size = 5 * 1024 * 1024usize;
    let full_payload: Vec<u8> = (0..total_size).map(|i| (i % 241) as u8).collect();
    let test_file_path = sender_dir.join("resumable.bin");
    std::fs::write(&test_file_path, &full_payload).unwrap();
    let expected_hash = blake3::hash(&full_payload).to_hex().to_string();

    let sender_identity = TlsIdentity::generate_self_signed("Sender Node").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let session_id = "resume-session".to_string();
    let meta = FileMetadata {
        file_id: "0".to_string(),
        filename: "resumable.bin".to_string(),
        size_bytes: total_size as u64,
        blake3_hash: Some(expected_hash.clone()),
        sha256_hash: None,
    };

    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: "Sender Node".to_string(),
        sender_device_id: "sender-resume-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: vec![meta.clone()],
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    // Simulate an interrupted transfer: receiver has partially received 2 MB
    let partial_size = 2 * 1024 * 1024usize;
    let partial_dest = receiver_dir.join("resumable.bin");
    std::fs::write(&partial_dest, &full_payload[..partial_size]).unwrap();

    // Verify HEAD /api/upload returns the partial size (2 MB)
    let status_bytes = client
        .get_upload_status(&host_port, &session_id, &token, &meta.file_id)
        .await
        .unwrap();
    assert_eq!(status_bytes, partial_size as u64);

    // Resume remaining 3 MB with HTTP Range
    let upload_session = dropit::client::UploadSession {
        host_port: &host_port,
        session_id: &session_id,
        session_token: &token,
    };
    let upload_res = client
        .stream_file_resumed(
            &upload_session,
            &meta,
            &test_file_path,
            partial_size as u64,
            |_p| {},
        )
        .await
        .unwrap();

    assert!(upload_res.verified);
    assert_eq!(upload_res.bytes_written, total_size as u64);
    assert_eq!(upload_res.blake3_hash, expected_hash);

    // Verify complete file on receiver disk
    let received_data = std::fs::read(&partial_dest).unwrap();
    assert_eq!(received_data.len(), total_size);
    assert_eq!(received_data, full_payload);

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_parallel_small_file_streams() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_parallel_test_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Parallel Receiver".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };

    let receiver_identity = TlsIdentity::generate_self_signed("Parallel Receiver").unwrap();
    let receiver_config_arc = Arc::new(RwLock::new(receiver_config));

    let server_state = ServerState::new(
        receiver_config_arc.clone(),
        receiver_identity.clone(),
        None,
        None,
    );

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    tokio::spawn(async move {
        let _ = run_server(server_state, actual_addr, Some(shutdown_rx)).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create 12 small files (64 KB each)
    let num_files = 12;
    let mut metas = Vec::new();
    let mut file_items = Vec::new();

    for i in 0..num_files {
        let filename = format!("small_file_{i}.dat");
        let path = sender_dir.join(&filename);
        let content: Vec<u8> = (0..(64 * 1024)).map(|b| ((b + i) % 255) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let hash = blake3::hash(&content).to_hex().to_string();

        let meta = FileMetadata {
            file_id: i.to_string(),
            filename,
            size_bytes: content.len() as u64,
            blake3_hash: Some(hash),
            sha256_hash: None,
        };
        metas.push(meta.clone());
        file_items.push((meta, path));
    }

    let sender_identity = TlsIdentity::generate_self_signed("Parallel Sender").unwrap();
    let client = Arc::new(DropItClient::new());
    let host_port = format!("127.0.0.1:{port}");

    let session_id = "parallel-session".to_string();
    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: "Parallel Sender".to_string(),
        sender_device_id: "sender-parallel-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: metas,
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    // Execute transfer with 4-parallel stream orchestrator
    let results = client
        .send_files_smart(
            host_port.clone(),
            session_id,
            token,
            file_items.clone(),
            |_p| {},
        )
        .await
        .unwrap();

    assert_eq!(results.len(), num_files);
    for res in &results {
        assert!(res.verified);
    }

    // Verify all 12 files arrived intact at destination
    for (meta, sender_path) in file_items {
        let dest = receiver_dir.join(&meta.filename);
        assert!(
            dest.exists(),
            "File {} should exist on receiver",
            meta.filename
        );
        let src_data = std::fs::read(&sender_path).unwrap();
        let dst_data = std::fs::read(&dest).unwrap();
        assert_eq!(src_data, dst_data);
    }

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_verify_hash_toggle() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_toggle_test_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Toggle Receiver".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };

    let receiver_identity = TlsIdentity::generate_self_signed("Toggle Receiver").unwrap();
    let receiver_config_arc = Arc::new(RwLock::new(receiver_config));

    let server_state = ServerState::new(
        receiver_config_arc.clone(),
        receiver_identity.clone(),
        None,
        None,
    );

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    tokio::spawn(async move {
        let _ = run_server(server_state, actual_addr, Some(shutdown_rx)).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let test_file_path = sender_dir.join("no_verify.bin");
    let payload = b"Fast transfer without preliminary hash check";
    std::fs::write(&test_file_path, payload).unwrap();

    let sender_identity = TlsIdentity::generate_self_signed("Sender Node").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let session_id = "no-verify-session".to_string();
    // blake3_hash is None (verify_hash disabled)
    let meta = FileMetadata {
        file_id: "0".to_string(),
        filename: "no_verify.bin".to_string(),
        size_bytes: payload.len() as u64,
        blake3_hash: None,
        sha256_hash: None,
    };

    let offer_req = TransferOfferRequest {
        session_id: session_id.clone(),
        sender_name: "Sender Node".to_string(),
        sender_device_id: "sender-noverify-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: vec![meta.clone()],
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    let upload_result = client
        .stream_file(
            &host_port,
            &session_id,
            &token,
            &meta,
            &test_file_path,
            |_p| {},
        )
        .await
        .unwrap();

    assert!(upload_result.verified);
    assert_eq!(upload_result.bytes_written, payload.len() as u64);

    let received_path = receiver_dir.join("no_verify.bin");
    assert!(received_path.exists());
    assert_eq!(std::fs::read(&received_path).unwrap(), payload);

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}
