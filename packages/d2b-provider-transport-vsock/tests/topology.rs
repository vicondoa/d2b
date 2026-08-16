use d2b_provider_transport_vsock::{
    ParentStoreResourceCensus, TopologyError, TransportLimits, VsockTransportSettings, ZoneLinkSpec,
};

fn spec() -> ZoneLinkSpec {
    ZoneLinkSpec {
        child_zone_name: "k1".to_owned(),
        transport_provider_ref: "Provider/transport-vsock".to_owned(),
        transport_settings: VsockTransportSettings::new("Guest/k1-vm").unwrap(),
        transport_credentials: Vec::new(),
        disabled: false,
        limits: TransportLimits::default(),
    }
}

#[test]
fn canonical_zonelink_fields_are_child_local_and_credentials_are_empty() {
    spec().validate("k1").unwrap();
}

#[test]
fn legacy_provider_fields_and_parent_reciprocal_rows_are_rejected() {
    let mut invalid = spec();
    invalid.transport_provider_ref = "Provider/legacy-vsock".to_owned();
    assert_eq!(invalid.validate("k1"), Err(TopologyError::ProviderMismatch));
    assert_eq!(
        (ParentStoreResourceCensus {
            provider_rows: 1,
            zone_link_rows: 0,
        })
        .validate(),
        Err(TopologyError::ParentStoreReciprocalResource)
    );
}

#[test]
fn transport_credentials_must_be_empty_and_child_name_self_matches() {
    let mut invalid = spec();
    invalid.transport_credentials.push("secret".to_owned());
    assert_eq!(
        invalid.validate("k1"),
        Err(TopologyError::CredentialsNotEmpty)
    );
    assert_eq!(
        spec().validate("parent"),
        Err(TopologyError::ChildZoneMismatch)
    );
}
