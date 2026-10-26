use dropit::config::DropItConfig;
use dropit::crypto::TlsIdentity;
use dropit::discovery::PeerRegistry;
use dropit::server::{ServerState, run_server};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast};

#[tokio::test]
async fn test_web_ui_and_api_endpoints() {
    let config = DropItConfig {
        device_name: "UI Test Node".to_string(),
        ..Default::default()
    };

    let identity = TlsIdentity::generate_self_signed("UI Test Node").unwrap();
    let config_arc = Arc::new(RwLock::new(config));
    let registry = PeerRegistry::new();

    let server_state = ServerState::new(
        config_arc.clone(),
        identity.clone(),
        Some(registry.clone()),
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

    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();

    let base_url = format!("https://127.0.0.1:{port}");

    // 1. Test GET / serves embedded HTML
    let index_res = client.get(&base_url).send().await.unwrap();
    assert_eq!(index_res.status(), reqwest::StatusCode::OK);
    let html = index_res.text().await.unwrap();
    assert!(html.contains("Drop It - Local File Transfer"));
    assert!(html.contains("id=\"dropzone\""));
    assert!(html.contains("id=\"peers-grid\""));

    // 2. Test GET /api/info
    let info_res = client
        .get(format!("{base_url}/api/info"))
        .send()
        .await
        .unwrap();
    assert_eq!(info_res.status(), reqwest::StatusCode::OK);
    let info_json: serde_json::Value = info_res.json().await.unwrap();
    assert_eq!(info_json["name"], "UI Test Node");
    assert_eq!(info_json["fingerprint"], identity.fingerprint);

    // 3. Test GET /api/qr returns SVG XML
    let qr_res = client
        .get(format!("{base_url}/api/qr"))
        .send()
        .await
        .unwrap();
    assert_eq!(qr_res.status(), reqwest::StatusCode::OK);
    let qr_svg = qr_res.text().await.unwrap();
    assert!(qr_svg.contains("<svg"));
    assert!(qr_svg.contains("</svg>"));

    // 4. Test POST /api/settings updates config
    let update_res = client
        .post(format!("{base_url}/api/settings"))
        .json(&serde_json::json!({
            "name": "Renamed Node",
            "auto_accept_trusted": true,
            "verify_hash": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_res.status(), reqwest::StatusCode::OK);

    // Verify changed settings in /api/info
    let info_res2 = client
        .get(format!("{base_url}/api/info"))
        .send()
        .await
        .unwrap();
    let info_json2: serde_json::Value = info_res2.json().await.unwrap();
    assert_eq!(info_json2["name"], "Renamed Node");
    assert_eq!(info_json2["auto_accept_trusted"], true);
    assert_eq!(info_json2["verify_hash"], false);

    let _ = shutdown_tx.send(());
}
