#[test]
fn test_config_roundtrip() {
    // We can test config serialization and deserialization
    let default_cfg = dropit::config::DropItConfig::default();
    let json = serde_json::to_string(&default_cfg).expect("serialize config");
    let deserialized: dropit::config::DropItConfig =
        serde_json::from_str(&json).expect("deserialize config");

    assert_eq!(default_cfg.device_id, deserialized.device_id);
    assert_eq!(default_cfg.device_name, deserialized.device_name);
    assert_eq!(default_cfg.port, deserialized.port);
}
