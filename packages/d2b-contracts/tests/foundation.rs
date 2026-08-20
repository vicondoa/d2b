use d2b_contracts::{
    Error, FeatureFlag, Hello, SemverRange, Version, decode_json_body,
    ids::{OperationId, RealmId, WorkloadId},
    public_wire::BridgeCheck,
    token::{ProtocolToken, TokenError},
    v3::{IfName, IfNameError},
    workload_identity::{WorkloadIdentity, WorkloadTarget},
};

#[test]
fn foundational_values_keep_validation_and_redaction_contracts() {
    assert_eq!(IfName::new(""), Err(IfNameError::Empty));
    assert_eq!(IfName::new("bad.name"), Err(IfNameError::InvalidCharacter));
    assert_eq!(IfName::new("abcdefghijklmnop"), Err(IfNameError::TooLong));
    let ifname = IfName::new("d2b-br0").expect("valid interface name");
    assert_eq!(ifname.as_str(), "d2b-br0");
    assert_eq!(format!("{ifname:?}"), "IfName(<redacted>)");
    assert_eq!(format!("{ifname}"), "IfName(<redacted>)");

    assert_eq!(ProtocolToken::parse(""), Err(TokenError::Empty));
    assert!(ProtocolToken::parse("codec.v1").is_ok());
    assert!(OperationId::parse("operation-1").is_ok());

    let version = Version::new("0.4.0").expect("valid version");
    let range = SemverRange::new(">=0.4.0, <0.5.0").expect("valid range");
    assert!(range.allows(&version));
}

#[test]
fn wire_errors_preserve_stable_shape_and_reject_unknown_fields() {
    let hello = Hello {
        client_version: SemverRange::new(">=0.4.0, <0.5.0").expect("valid range"),
        supported_features: vec![FeatureFlag::new("typed-errors").expect("valid feature")],
    };
    let json = serde_json::to_value(&hello).expect("serialize hello");
    assert_eq!(json["clientVersion"], ">=0.4.0, <0.5.0");

    let error = Error::unknown_field("Hello", "unexpected");
    let encoded = serde_json::to_value(error).expect("serialize error");
    assert_eq!(encoded["kind"], "wire-unknown-field");
    assert_eq!(encoded["code"], 22);
    assert!(encoded["message"].as_str().unwrap().contains("unexpected"));

    let unknown = serde_json::json!({
        "clientVersion": ">=0.4.0, <0.5.0",
        "supportedFeatures": [],
        "unexpected": true
    });
    let decoded = decode_json_body::<Hello>("Hello", &serde_json::to_vec(&unknown).unwrap());
    assert_eq!(
        decoded
            .expect_err("unknown fields fail closed")
            .kind()
            .as_str(),
        "wire-unknown-field"
    );

    for bridge in ["abcdefghijklmnop", "bad.name"] {
        let invalid_ifname = serde_json::json!({
            "bridge": bridge,
            "present": true,
            "tap": null
        });
        let decoded = decode_json_body::<BridgeCheck>(
            "BridgeCheck",
            &serde_json::to_vec(&invalid_ifname).unwrap(),
        );
        assert_eq!(
            decoded
                .expect_err("invalid interface names fail as typed wire errors")
                .kind()
                .as_str(),
            "wire-ifname-invalid"
        );
    }
}

#[test]
fn workload_identity_round_trips_without_reintroducing_legacy_owners() {
    let realm_id = RealmId::parse("work").expect("valid realm");
    let identity = WorkloadIdentity::new(
        WorkloadId::parse("builder").expect("valid workload"),
        realm_id.clone(),
        d2b_contracts::realm::RealmPath::new(vec![realm_id]).expect("valid realm path"),
        WorkloadTarget::parse("builder.work.d2b").expect("valid target"),
    );
    let json = serde_json::to_string(&identity).expect("serialize identity");
    let decoded: WorkloadIdentity = serde_json::from_str(&json).expect("deserialize identity");
    assert_eq!(decoded, identity);
    assert_eq!(decoded.target().to_canonical(), "builder.work.d2b");
}

#[test]
fn wire_dto_uses_the_canonical_ifname_type() {
    let response = BridgeCheck {
        bridge: IfName::new("d2b-br0").expect("valid bridge"),
        present: true,
        tap: Some(IfName::new("d2b-tap0").expect("valid tap")),
    };
    let encoded = serde_json::to_value(response).expect("serialize bridge check");
    assert_eq!(encoded["bridge"], "d2b-br0");
    assert_eq!(encoded["tap"], "d2b-tap0");
}
