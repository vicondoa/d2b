use d2b_provider_transport_vsock::{PortClass, VsockTransportSettings};

#[test]
fn transport_settings_schema_rejects_cid_and_port_fields() {
    let schema: serde_json::Value =
        serde_json::from_str(VsockTransportSettings::schema_json()).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("cid").is_none());
    assert!(schema["properties"].get("port").is_none());
}

#[test]
fn transport_settings_round_trip_keeps_the_closed_port_class() {
    let settings: VsockTransportSettings =
        serde_json::from_str(r#"{"guestRef":"Guest/guest-a"}"#).unwrap();
    assert_eq!(settings.port_class, PortClass::D2bLink);
    assert_eq!(settings.connect_timeout_seconds, 30);
    settings.validate().unwrap();
}
