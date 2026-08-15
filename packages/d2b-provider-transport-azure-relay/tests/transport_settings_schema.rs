use d2b_provider_transport_azure_relay::{RelayTransportSettings, RelayTransportSettingsError};

#[test]
fn settings_accept_only_bare_non_secret_identifiers() {
    let settings = RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").unwrap();
    assert_eq!(settings.relay_entity_id, "hc-d2b-k2");
    let schema: serde_json::Value =
        serde_json::from_str(RelayTransportSettings::schema_json()).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert!(matches!(
        RelayTransportSettings::new("https://relay.example", "hc-d2b-k2"),
        Err(RelayTransportSettingsError::InvalidIdentifier)
    ));
    assert!(matches!(
        RelayTransportSettings::new("relns-d2b-prod", "SharedAccessSignature"),
        Err(RelayTransportSettingsError::InvalidIdentifier)
    ));
}
