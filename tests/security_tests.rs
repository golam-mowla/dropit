use dropit::client::DropItClient;
use dropit::config::DropItConfig;
use dropit::crypto::TlsIdentity;
use dropit::server::{MAX_CONCURRENT_SESSIONS, ServerState, run_server};
use dropit::transfer::{FileMetadata, TransferOfferRequest};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast};

#[tokio::test]
async fn test_path_traversal_and_malicious_filenames() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_sec_traversal_{}", rand::random::<u32>()));
    let download_dir = temp_base.join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Security Receiver".to_string(),
        download_dir: download_dir.clone(),
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("Security Receiver").unwrap();
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

    let sender_identity = TlsIdentity::generate_self_signed("Attacker Node").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    // Malicious filenames: directory traversals, absolute paths, reserved names, forbidden chars
    let attacks = vec![
        ("0", "../../escape.txt"),
        ("1", r"..\..\windows_escape.txt"),
        ("2", "/etc/passwd"),
        ("3", r"C:\Windows\System32\malicious.exe"),
        ("4", "CON.txt"),
        ("5", "NUL"),
        ("6", "test:colon*star?question.txt"),
        ("7", "control\0null\r\nnewline.txt"),
    ];

    let mut metas = Vec::new();
    for (id, filename) in &attacks {
        metas.push(FileMetadata {
            file_id: id.to_string(),
            filename: filename.to_string(),
            size_bytes: 12,
            blake3_hash: None,
            sha256_hash: None,
        });
    }

    let offer_req = TransferOfferRequest {
        session_id: "sec-attack-session".to_string(),
        sender_name: "Attacker Node".to_string(),
        sender_device_id: "attacker-device-id".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: metas.clone(),
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);
    let token = offer_resp.session_token.expect("session token");

    // Upload payload to all files
    let payload = b"hello attack";
    let temp_file = temp_base.join("temp_upload.bin");
    std::fs::write(&temp_file, payload).unwrap();

    for meta in &metas {
        let upload_res = client
            .stream_file(
                &host_port,
                "sec-attack-session",
                &token,
                meta,
                &temp_file,
                |_p| {},
            )
            .await
            .unwrap();
        assert!(upload_res.verified);
    }

    // Verify all written files are strictly confined to download_dir
    let written_files: Vec<std::path::PathBuf> = std::fs::read_dir(&download_dir)
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect();

    assert_eq!(written_files.len(), attacks.len());
    for file in &written_files {
        // Assert file is directly inside download_dir (no subdirectories created or escaped)
        assert_eq!(file.parent().unwrap(), download_dir.as_path());
        let fname = file.file_name().unwrap().to_str().unwrap();
        assert!(!fname.contains('/'));
        assert!(!fname.contains('\\'));
        assert!(!fname.contains(".."));
        assert!(!fname.starts_with("CON."));
        assert_ne!(fname, "NUL");
    }

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_pinned_fingerprint_mismatch_rejection() {
    let temp_base = std::env::temp_dir().join(format!("dropit_sec_tofu_{}", rand::random::<u32>()));
    let download_dir = temp_base.join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();

    let mut trusted_devices = std::collections::HashMap::new();
    let original_fp = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99".to_string();
    trusted_devices.insert("trusted-peer-1".to_string(), original_fp);

    let receiver_config = DropItConfig {
        device_name: "Pinned Receiver".to_string(),
        download_dir: download_dir.clone(),
        auto_accept_trusted: true,
        trusted_devices,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("Pinned Receiver").unwrap();
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

    // Sender with changed/spoofed certificate fingerprint (MITM scenario)
    let different_fp = "FF:EE:DD:CC:BB:AA:99:88:77:66:55:44:33:22:11:00:FF:EE:DD:CC:BB:AA:99:88:77:66:55:44:33:22:11:00".to_string();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let offer_req = TransferOfferRequest {
        session_id: "sec-mitm-session".to_string(),
        sender_name: "Impersonator Node".to_string(),
        sender_device_id: "trusted-peer-1".to_string(),
        sender_fingerprint: different_fp,
        sender_port: 53420,
        files: vec![FileMetadata {
            file_id: "0".to_string(),
            filename: "exploit.txt".to_string(),
            size_bytes: 10,
            blake3_hash: None,
            sha256_hash: None,
        }],
    };

    let offer_res = client.send_offer(&host_port, &offer_req).await;
    assert!(
        offer_res.is_err(),
        "Offer with mismatched pinned fingerprint MUST be rejected"
    );
    let err_str = offer_res.err().unwrap().to_string();
    assert!(
        err_str.contains("mismatch") || err_str.contains("409") || err_str.contains("rejected"),
        "Error message should mention fingerprint mismatch: {err_str}"
    );

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_rate_limiting_rapid_offers() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_sec_ratelimit_{}", rand::random::<u32>()));
    let download_dir = temp_base.join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "RateLimit Receiver".to_string(),
        download_dir,
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("RateLimit Receiver").unwrap();
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

    let sender_identity = TlsIdentity::generate_self_signed("Spam Node").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let offer_req = TransferOfferRequest {
        session_id: "spam-session-1".to_string(),
        sender_name: "Spam Node".to_string(),
        sender_device_id: "spam-device-id".to_string(),
        sender_fingerprint: sender_identity.fingerprint.clone(),
        sender_port: 53420,
        files: vec![FileMetadata {
            file_id: "0".to_string(),
            filename: "spam.txt".to_string(),
            size_bytes: 4,
            blake3_hash: None,
            sha256_hash: None,
        }],
    };

    // First offer succeeds
    let res1 = client.send_offer(&host_port, &offer_req).await;
    assert!(res1.is_ok());

    // Immediate second offer within 1ms must trigger 429 Rate Limit
    let mut offer_req2 = offer_req.clone();
    offer_req2.session_id = "spam-session-2".to_string();
    let res2 = client.send_offer(&host_port, &offer_req2).await;
    assert!(res2.is_err(), "Rapid offer should hit rate limit");
    let err_msg = res2.err().unwrap().to_string();
    assert!(
        err_msg.contains("Rate limit") || err_msg.contains("429"),
        "Error should reflect rate limiting: {err_msg}"
    );

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_max_concurrent_sessions_limit() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_sec_sessions_{}", rand::random::<u32>()));
    let download_dir = temp_base.join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Sessions Receiver".to_string(),
        download_dir,
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("Sessions Receiver").unwrap();
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

    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    // Fill up to MAX_CONCURRENT_SESSIONS
    for i in 0..MAX_CONCURRENT_SESSIONS {
        let sender_identity =
            TlsIdentity::generate_self_signed(&format!("Session Node {i}")).unwrap();
        // Pause 210ms between offers to not hit rate limit
        tokio::time::sleep(Duration::from_millis(210)).await;

        let offer = TransferOfferRequest {
            session_id: format!("session-{i}"),
            sender_name: format!("Node {i}"),
            sender_device_id: format!("device-{i}"),
            sender_fingerprint: sender_identity.fingerprint,
            sender_port: 53420,
            files: vec![FileMetadata {
                file_id: "0".to_string(),
                filename: format!("file_{i}.dat"),
                size_bytes: 10,
                blake3_hash: None,
                sha256_hash: None,
            }],
        };
        let res = client.send_offer(&host_port, &offer).await;
        assert!(res.is_ok(), "Session {i} should be created");
    }

    // Now try to exceed MAX_CONCURRENT_SESSIONS
    tokio::time::sleep(Duration::from_millis(210)).await;
    let overflow_sender = TlsIdentity::generate_self_signed("Overflow Node").unwrap();
    let overflow_offer = TransferOfferRequest {
        session_id: "overflow-session".to_string(),
        sender_name: "Overflow Node".to_string(),
        sender_device_id: "overflow-device".to_string(),
        sender_fingerprint: overflow_sender.fingerprint,
        sender_port: 53420,
        files: vec![FileMetadata {
            file_id: "0".to_string(),
            filename: "overflow.dat".to_string(),
            size_bytes: 10,
            blake3_hash: None,
            sha256_hash: None,
        }],
    };

    let overflow_res = client.send_offer(&host_port, &overflow_offer).await;
    assert!(
        overflow_res.is_err(),
        "Offer exceeding MAX_CONCURRENT_SESSIONS MUST be rejected"
    );
    let err_msg = overflow_res.err().unwrap().to_string();
    assert!(
        err_msg.contains("busy") || err_msg.contains("503"),
        "Error should indicate server busy: {err_msg}"
    );

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_unauthorized_token_rejection() {
    let temp_base =
        std::env::temp_dir().join(format!("dropit_sec_unauth_{}", rand::random::<u32>()));
    let download_dir = temp_base.join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();

    let receiver_config = DropItConfig {
        device_name: "Auth Receiver".to_string(),
        download_dir,
        auto_accept_trusted: true,
        ..Default::default()
    };
    let receiver_identity = TlsIdentity::generate_self_signed("Auth Receiver").unwrap();
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

    let sender_identity = TlsIdentity::generate_self_signed("Sender Node").unwrap();
    let client = DropItClient::new();
    let host_port = format!("127.0.0.1:{port}");

    let offer_req = TransferOfferRequest {
        session_id: "auth-session".to_string(),
        sender_name: "Sender Node".to_string(),
        sender_device_id: "sender-device".to_string(),
        sender_fingerprint: sender_identity.fingerprint,
        sender_port: 53420,
        files: vec![FileMetadata {
            file_id: "0".to_string(),
            filename: "secret.txt".to_string(),
            size_bytes: 10,
            blake3_hash: None,
            sha256_hash: None,
        }],
    };

    let offer_resp = client.send_offer(&host_port, &offer_req).await.unwrap();
    assert!(offer_resp.accepted);

    // Attempt upload with an invalid/forged token
    let fake_token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let temp_file = temp_base.join("temp.bin");
    std::fs::write(&temp_file, b"test payload").unwrap();

    let upload_res = client
        .stream_file(
            &host_port,
            "auth-session",
            fake_token,
            &offer_req.files[0],
            &temp_file,
            |_p| {},
        )
        .await;

    assert!(
        upload_res.is_err(),
        "Upload with fake token MUST be rejected"
    );
    let err_msg = upload_res.err().unwrap().to_string();
    assert!(
        err_msg.contains("403") || err_msg.contains("Forbidden") || err_msg.contains("failed"),
        "Error message should reflect authorization failure: {err_msg}"
    );

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_base);
}
