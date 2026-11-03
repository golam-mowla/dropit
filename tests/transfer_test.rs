use dropit::client::DropItClient;
use dropit::pairing::SessionTokenStore;
use dropit::server::{self, FileMetadata, ServerState, TransferIntentRequest};
use dropit::tls::TlsIdentity;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

async fn spawn_test_server(
    auto_accept: bool,
) -> (u16, tempfile::TempDir, tokio::sync::watch::Sender<bool>) {
    dropit::tls::init_crypto_provider();
    let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let download_dir = temp_dir.path().join("downloads");
    tokio::fs::create_dir_all(&download_dir).await.unwrap();

    let tls_identity = TlsIdentity::generate().expect("Failed to generate TLS cert");
    let token_store = SessionTokenStore::new();

    let server_state = ServerState::new(
        "test-server-id".to_string(),
        "Test Receiver".to_string(),
        download_dir,
        token_store,
        tls_identity.fingerprint.clone(),
        auto_accept,
    );

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let assigned_port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{}", assigned_port).parse().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    tokio::spawn(async move {
        let _ = server::run_server(actual_addr, server_state, &tls_identity, shutdown_rx).await;
    });

    tokio::time::sleep(Duration::from_millis(250)).await;
    (assigned_port, temp_dir, shutdown_tx)
}

#[tokio::test]
async fn test_loopback_transfer_and_hash_verification() {
    let (port, temp_dir, shutdown_tx) = spawn_test_server(true).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let send_file_path = temp_dir.path().join("hello_dropit.txt");
    let content = b"Hello from Drop It integration test!";
    tokio::fs::write(&send_file_path, content).await.unwrap();

    client
        .send_files(&[&send_file_path], true)
        .await
        .expect("Transfer failed");

    let received_path = temp_dir.path().join("downloads").join("hello_dropit.txt");
    assert!(received_path.exists());
    let received_content = tokio::fs::read(&received_path).await.unwrap();
    assert_eq!(received_content, content);

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_pairing_with_code_and_tofu() {
    dropit::tls::init_crypto_provider();
    let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let download_dir = temp_dir.path().join("received_pairing");
    tokio::fs::create_dir_all(&download_dir).await.unwrap();

    let tls_identity = TlsIdentity::generate().expect("Failed to generate TLS cert");
    let token_store = SessionTokenStore::new();
    let pairing_code = token_store.pairing_session().generate_code();

    let server_state = ServerState::new(
        "test-pairing-receiver".to_string(),
        "Pairing Receiver".to_string(),
        download_dir,
        token_store,
        tls_identity.fingerprint.clone(),
        true,
    );

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let assigned_port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{}", assigned_port).parse().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let identity_clone = tls_identity.clone();
    tokio::spawn(async move {
        let _ = server::run_server(actual_addr, server_state, &identity_clone, shutdown_rx).await;
    });

    tokio::time::sleep(Duration::from_millis(250)).await;

    let client = DropItClient::new("127.0.0.1", assigned_port).expect("Client init failed");

    // 1. Wrong code fails
    let wrong_res = client
        .pair_with_code("0-invalid-code", "client-device-1", "fingerprint-aaa")
        .await;
    assert!(wrong_res.is_err());

    // 2. Correct code succeeds
    let pair_resp = client
        .pair_with_code(&pairing_code, "client-device-1", "fingerprint-aaa")
        .await
        .expect("Pairing with valid code must succeed");
    assert_eq!(pair_resp.server_fingerprint, tls_identity.fingerprint);
    assert!(!pair_resp.token.is_empty());

    // 3. TOFU check: same client with different fingerprint must be rejected
    let attacker_res = client
        .pair_with_code("any-code", "client-device-1", "fingerprint-forged")
        .await;
    assert!(attacker_res.is_err());

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_directory_transfer() {
    let (port, temp_dir, shutdown_tx) = spawn_test_server(true).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let source_dir = temp_dir.path().join("source_folder");
    let sub_dir = source_dir.join("subfolder");
    tokio::fs::create_dir_all(&sub_dir).await.unwrap();

    let f1 = source_dir.join("file1.txt");
    let f2 = sub_dir.join("file2.txt");
    tokio::fs::write(&f1, b"File 1 in root").await.unwrap();
    tokio::fs::write(&f2, b"File 2 in nested directory")
        .await
        .unwrap();

    client
        .send_directory(&source_dir, true)
        .await
        .expect("Directory transfer failed");

    let received_f1 = temp_dir.path().join("downloads").join("file1.txt");
    let received_f2 = temp_dir.path().join("downloads").join("file2.txt");
    assert!(received_f1.exists());
    assert!(received_f2.exists());

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_1000_small_files_transfer() {
    let (port, temp_dir, shutdown_tx) = spawn_test_server(true).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let batch_dir = temp_dir.path().join("batch_1000");
    tokio::fs::create_dir_all(&batch_dir).await.unwrap();

    let mut file_paths = Vec::with_capacity(1000);
    for i in 0..1000 {
        let p = batch_dir.join(format!("item_{:04}.dat", i));
        tokio::fs::write(&p, format!("data content {}", i).as_bytes())
            .await
            .unwrap();
        file_paths.push(p);
    }

    client
        .send_files(&file_paths, false)
        .await
        .expect("1000 files transfer failed");

    // Verify sample of transferred files
    let dl = temp_dir.path().join("downloads");
    assert!(dl.join("item_0000.dat").exists());
    assert!(dl.join("item_0500.dat").exists());
    assert!(dl.join("item_0999.dat").exists());

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_transfer_intent_rejection() {
    // Start server with auto_accept = false
    let (port, _temp_dir, shutdown_tx) = spawn_test_server(false).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let intent = TransferIntentRequest {
        session_id: "session-reject-123".to_string(),
        sender_id: "sender-abc".to_string(),
        sender_alias: "Swift Falcon".to_string(),
        files: vec![FileMetadata {
            id: "1".to_string(),
            filename: "doc.pdf".to_string(),
            size: 1024,
            blake3_hash: None,
        }],
        total_size: 1024,
    };

    let client_clone = DropItClient::new("127.0.0.1", port).unwrap();
    // Simulate user rejecting the request
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = client_clone
            .decide_transfer(
                "session-reject-123",
                false,
                Some("Declined by user".to_string()),
            )
            .await;
    });

    let resp = client.prepare_transfer(&intent).await.unwrap();
    assert_eq!(resp.status, "rejected");

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_transfer_cancellation() {
    let (port, temp_dir, shutdown_tx) = spawn_test_server(true).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let cancel_flag = Arc::new(AtomicBool::new(false));

    let f1 = temp_dir.path().join("file_cancel.txt");
    tokio::fs::write(&f1, b"Payload before cancellation")
        .await
        .unwrap();

    // Trigger local cancellation
    cancel_flag.store(true, Ordering::SeqCst);

    let res = client
        .send_files_with_intent(
            &[&f1],
            "sender-id-1",
            "Swift Falcon",
            true,
            Some(cancel_flag),
        )
        .await;

    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("cancelled"));

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_embedded_web_ui_endpoint() {
    let (port, _temp_dir, shutdown_tx) = spawn_test_server(true).await;

    // Direct reqwest GET / and GET /web
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let resp_root = http
        .get(format!("https://127.0.0.1:{}/", port))
        .send()
        .await
        .unwrap();
    assert!(resp_root.status().is_success());
    let html_root = resp_root.text().await.unwrap();
    assert!(html_root.contains("Drop It"));
    assert!(html_root.contains("qr-canvas"));

    let resp_web = http
        .get(format!("https://127.0.0.1:{}/web", port))
        .send()
        .await
        .unwrap();
    assert!(resp_web.status().is_success());

    let _ = shutdown_tx.send(true);
}

#[tokio::test]
async fn test_loopback_chat_messaging_and_file_bubble() {
    let (port, temp_dir, shutdown_tx) = spawn_test_server(true).await;
    let client = DropItClient::new("127.0.0.1", port).expect("Client init failed");

    let session_id = "session-chat-100";

    // 1. Send text chat message
    let text_msg = dropit::messenger::ChatMessage::new(
        session_id,
        "device-alice",
        "Swift Falcon",
        dropit::messenger::MessagePayload::Text("Hello over encrypted channel!".to_string()),
    );
    let msg_id = client
        .send_chat_message(&text_msg)
        .await
        .expect("Failed to send text chat");
    assert_eq!(msg_id, text_msg.id);

    // 2. Send typing indicator
    let typing_msg = dropit::messenger::ChatMessage::new(
        session_id,
        "device-alice",
        "Swift Falcon",
        dropit::messenger::MessagePayload::TypingIndicator { is_typing: true },
    );
    client
        .send_chat_message(&typing_msg)
        .await
        .expect("Failed to send typing");

    // 3. Send file associated with the same session_id
    let f1 = temp_dir.path().join("chat_shared_file.bin");
    tokio::fs::write(&f1, b"Payload inside shared conversation thread")
        .await
        .unwrap();
    client
        .send_files_with_intent_and_session(
            Some(session_id),
            &[&f1],
            "device-alice",
            "Swift Falcon",
            true,
            None,
        )
        .await
        .expect("Transfer failed");

    // 4. Retrieve conversation thread messages
    let msgs = client
        .get_chat_messages(session_id)
        .await
        .expect("Failed to get messages");
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].id, text_msg.id);

    // Verify presence of text
    match &msgs[0].payload {
        dropit::messenger::MessagePayload::Text(content) => {
            assert_eq!(content, "Hello over encrypted channel!");
        }
        _ => panic!("Expected Text payload"),
    }

    // Verify presence of file bubble in the shared thread
    match &msgs[2].payload {
        dropit::messenger::MessagePayload::FileBubble { filename, size, .. } => {
            assert_eq!(filename, "chat_shared_file.bin");
            assert_eq!(*size, 41);
        }
        _ => panic!("Expected FileBubble payload for transferred file"),
    }

    let _ = shutdown_tx.send(true);
}

#[test]
fn test_messenger_symmetric_ratchet_forward_secrecy() {
    let shared_secret = [0x77u8; 32];
    let mut alice = dropit::messenger::SymmetricRatchet::new(shared_secret);
    let mut bob = dropit::messenger::SymmetricRatchet::new(shared_secret);

    // Turn 1: Alice -> Bob
    let plaintext1 = b"Message 1 with forward secrecy";
    let key_alice1 = alice.step().expect("Alice step 1 failed");
    let frame1 = dropit::messenger::pack_frame(&key_alice1, 1, plaintext1).expect("Pack failed");
    let key_bob1 = bob.step().expect("Bob step 1 failed");
    assert_eq!(key_alice1, key_bob1);
    let decrypted1 = dropit::messenger::unpack_frame(&key_bob1, &frame1).expect("Unpack failed");
    assert_eq!(decrypted1, plaintext1);

    // Turn 2: Alice -> Bob
    let plaintext2 = b"Message 2 with next step key";
    let key_alice2 = alice.step().expect("Alice step 2 failed");
    let frame2 = dropit::messenger::pack_frame(&key_alice2, 2, plaintext2).expect("Pack failed");
    let key_bob2 = bob.step().expect("Bob step 2 failed");
    assert_eq!(key_alice2, key_bob2);
    assert_ne!(key_alice1, key_alice2);
    let decrypted2 = dropit::messenger::unpack_frame(&key_bob2, &frame2).expect("Unpack failed");
    assert_eq!(decrypted2, plaintext2);

    // Tampered frame fails tag authentication
    let mut tampered_frame = frame2.clone();
    let last = tampered_frame.len() - 1;
    tampered_frame[last] ^= 0x01;
    let tamper_res = dropit::messenger::unpack_frame(&key_bob2, &tampered_frame);
    assert!(tamper_res.is_err());
}
