use dropit::client::DropItClient;
use dropit::pairing::SessionTokenStore;
use dropit::server::{self, ServerState};
use dropit::tls::TlsIdentity;
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::test]
async fn test_loopback_transfer_and_hash_verification() {
    let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let download_dir = temp_dir.path().join("received");
    tokio::fs::create_dir_all(&download_dir).await.unwrap();

    let tls_identity = TlsIdentity::generate().expect("Failed to generate TLS cert");
    let token_store = SessionTokenStore::new();

    let server_state = ServerState {
        device_id: "test-receiver-id".to_string(),
        name: "Test Receiver".to_string(),
        download_dir: download_dir.clone(),
        token_store,
        cert_fingerprint: tls_identity.fingerprint.clone(),
        auto_accept: true,
    };

    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = std::net::TcpListener::bind(bind_addr).unwrap();
    let assigned_port = listener.local_addr().unwrap().port();
    drop(listener);

    let actual_addr: SocketAddr = format!("127.0.0.1:{}", assigned_port).parse().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let identity_clone = tls_identity.clone();
    let server_handle = tokio::spawn(async move {
        server::run_server(actual_addr, server_state, &identity_clone, shutdown_rx).await
    });

    // Wait for server to bind
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create a dummy file to send
    let send_file_path = temp_dir.path().join("hello_dropit.txt");
    let content = b"Hello from Drop It integration test!";
    tokio::fs::write(&send_file_path, content).await.unwrap();

    let client = DropItClient::new("127.0.0.1", assigned_port).expect("Client init failed");
    let info = client.get_info().await.expect("Failed to fetch info");
    assert_eq!(info.device_id, "test-receiver-id");

    // Send file and verify hash
    client
        .send_files(&[&send_file_path], true)
        .await
        .expect("Transfer failed");

    // Verify file exists on receiver side with matching content
    let received_path = download_dir.join("hello_dropit.txt");
    assert!(received_path.exists());
    let received_content = tokio::fs::read(&received_path).await.unwrap();
    assert_eq!(received_content, content);

    // Shutdown server
    let _ = shutdown_tx.send(true);
    let _ = server_handle.await;
}
