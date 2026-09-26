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

#[test]
fn deserialization_admits_exactly_what_the_constructor_admits() {
    let blob = r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"hc-d2b-k2"}"#;
    let settings =
        serde_json::from_str::<RelayTransportSettings>(blob).expect("an admitted settings blob");
    assert_eq!(
        settings,
        RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").unwrap()
    );
    assert_eq!(serde_json::to_string(&settings).unwrap(), blob);

    for refused in [
        // Secret-shaped entity identifiers: refused by the Rust validator and
        // recorded as an exclusion in the pinned schema.
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"SharedAccessSignature sr=x&sig=y"}"#,
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"hc-SharedAccessSignature-k2"}"#,
        // Namespace grammar, bound, and separator exclusions.
        r#"{"relayNamespaceId":"ns","relayEntityId":"hc-d2b-k2"}"#,
        r#"{"relayNamespaceId":"https://relay.example","relayEntityId":"hc-d2b-k2"}"#,
        r#"{"relayNamespaceId":"relns/d2b","relayEntityId":"hc-d2b-k2"}"#,
        r#"{"relayNamespaceId":"relns:d2b","relayEntityId":"hc-d2b-k2"}"#,
        r#"{"relayNamespaceId":"-relns-d2b-","relayEntityId":"hc-d2b-k2"}"#,
        // Entity grammar and bound.
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"HC-D2B-K2"}"#,
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"h"}"#,
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"hc_d2b"}"#,
        // Wire-shape failures the derived form owns.
        r#"{"relayNamespaceId":"relns-d2b-prod"}"#,
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"hc-d2b-k2","relayKeyName":"listen"}"#,
        r#"{"relayNamespaceId":7,"relayEntityId":"hc-d2b-k2"}"#,
    ] {
        assert!(
            serde_json::from_str::<RelayTransportSettings>(refused).is_err(),
            "{refused} must be refused at deserialization"
        );
    }

    let long_namespace = format!(
        r#"{{"relayNamespaceId":"{}","relayEntityId":"hc-d2b-k2"}}"#,
        "a".repeat(51)
    );
    let long_entity = format!(
        r#"{{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"{}"}}"#,
        "h".repeat(51)
    );
    for refused in [long_namespace, long_entity] {
        assert!(
            serde_json::from_str::<RelayTransportSettings>(&refused).is_err(),
            "{refused} must be refused at deserialization"
        );
    }
}

#[test]
fn refused_settings_surface_the_typed_validation_error() {
    let error = serde_json::from_str::<RelayTransportSettings>(
        r#"{"relayNamespaceId":"relns-d2b-prod","relayEntityId":"SharedAccessSignature sr=x&sig=y"}"#,
    )
    .expect_err("a secret-shaped entity identifier is refused");
    let typed = RelayTransportSettingsError::InvalidIdentifier;
    assert!(
        error.to_string().contains(&typed.to_string()),
        "deserialization must carry the typed validation error: {error}"
    );
}

#[test]
fn pinned_schema_records_the_secret_shape_exclusion() {
    let schema: serde_json::Value =
        serde_json::from_str(RelayTransportSettings::schema_json()).unwrap();
    assert_eq!(
        schema["properties"]["relayEntityId"]["not"]["pattern"],
        "SharedAccessSignature"
    );
}
