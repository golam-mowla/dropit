use dropit::client::DropItClient;
use dropit::config::DropItConfig;
use dropit::crypto::{self, TlsIdentity};
use dropit::server::{ServerState, run_server};
use dropit::transfer::{FileMetadata, TransferOfferRequest};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, broadcast};

#[tokio::test]
async fn test_send_2gb_large_file_streaming() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_scale_2gb_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Scale Receiver".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("Scale Receiver").unwrap();
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

    // Create 2 GB sparse file (2 * 1024 * 1024 * 1024 = 2,147,483,648 bytes)
    let two_gb_bytes: u64 = 2 * 1024 * 1024 * 1024;
    let large_file_path = sender_dir.join("large_2gb.bin");
    {
        let file = std::fs::File::create(&large_file_path).unwrap();
        file.set_len(two_gb_bytes).unwrap();
    }

    // Fast streaming hash calculation for 2 GB sparse file (all zeroes)
    println!("Calculating BLAKE3 streaming hash for 2 GB file...");
    let hash_start = Instant::now();
    let expected_hash = crypto::hash_file_streaming(&large_file_path).await.unwrap();
    println!(
        "2 GB BLAKE3: {} (calculated in {:.2}s)",
        expected_hash,
        hash_start.elapsed().as_secs_f64()
    );

    let sender_identity = TlsIdentity::generate_self_signed("Scale Sender").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let meta = FileMetadata {
        file_id: "0".to_string(),
        filename: "large_2gb.bin".to_string(),
        size_bytes: two_gb_bytes,
        blake3_hash: Some(expected_hash.clone()),
        sha256_hash: None,
    };

    let offer_req = TransferOfferRequest {
        session_id: "scale-2gb-session".to_string(),
        sender_name: "Scale Sender".to_string(),
        sender_device_id: "scale-sender-1".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: vec![meta.clone()],
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    println!("Starting 2 GB streaming transfer disk -> socket -> disk...");
    let transfer_start = Instant::now();

    let upload_res = client
        .stream_file(
            &host_port,
            "scale-2gb-session",
            &token,
            &meta,
            &large_file_path,
            |_p| {},
        )
        .await
        .unwrap();

    let duration_secs = transfer_start.elapsed().as_secs_f64();
    let speed_mb_s = (two_gb_bytes as f64 / (1024.0 * 1024.0)) / duration_secs;

    println!(
        "🚀 2 GB transfer completed in {:.2}s ({:.2} MB/s)!",
        duration_secs, speed_mb_s
    );

    assert!(upload_res.verified);
    assert_eq!(upload_res.bytes_written, two_gb_bytes);
    assert_eq!(upload_res.blake3_hash, expected_hash);

    // Verify received file on disk
    let received_path = receiver_dir.join("large_2gb.bin");
    assert!(received_path.exists());
    let recv_meta = std::fs::metadata(&received_path).unwrap();
    assert_eq!(recv_meta.len(), two_gb_bytes);

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_send_1000_small_files_batch() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_scale_1000_{}", rand::random::<u32>()));
    let sender_dir = temp_base.join("sender");
    let receiver_dir = temp_base.join("receiver");
    std::fs::create_dir_all(&sender_dir).unwrap();
    std::fs::create_dir_all(&receiver_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "1000 Files Receiver".to_string(),
        download_dir: receiver_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("1000 Files Receiver").unwrap();
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

    // Create 1,000 small files (128 bytes each)
    let num_files = 1_000;
    println!("Creating {num_files} small test files...");
    let mut metas = Vec::with_capacity(num_files);
    let mut items = Vec::with_capacity(num_files);

    for i in 0..num_files {
        let filename = format!("file_{i:04}.txt");
        let path = sender_dir.join(&filename);
        let content = format!("DropIt small test payload #{i:04}\n");
        std::fs::write(&path, content.as_bytes()).unwrap();

        let meta = FileMetadata {
            file_id: i.to_string(),
            filename,
            size_bytes: content.len() as u64,
            blake3_hash: None,
            sha256_hash: None,
        };
        metas.push(meta.clone());
        items.push((meta, path));
    }

    let sender_identity = TlsIdentity::generate_self_signed("1000 Sender").unwrap();
    let client = Arc::new(DropItClient::new());
    let host_port = format!("127.0.0.1:{port}");

    let offer_req = TransferOfferRequest {
        session_id: "batch-1000-session".to_string(),
        sender_name: "1000 Sender".to_string(),
        sender_device_id: "sender-1000-id".to_string(),
        sender_fingerprint: sender_identity.fingerprint,
        sender_port: 53420,
        files: metas,
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    println!("Starting parallel transfer of 1,000 files across 4 streams...");
    let batch_start = Instant::now();

    let results = client
        .send_files_smart(
            host_port.clone(),
            "batch-1000-session".to_string(),
            token,
            items,
            |_p| {},
        )
        .await
        .unwrap();

    let batch_duration = batch_start.elapsed().as_secs_f64();
    println!(
        "🚀 1,000 small files transferred in {:.2}s ({:.1} files/sec)!",
        batch_duration,
        num_files as f64 / batch_duration
    );

    assert_eq!(results.len(), num_files);
    for res in &results {
        assert!(res.verified);
    }

    // Verify all 1,000 files exist in destination directory
    let dest_count = std::fs::read_dir(&receiver_dir).unwrap().count();
    assert_eq!(dest_count, num_files);

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}
