use d2b_contracts::{
    CapabilitySet, DisplayEnvironmentPosture, EnvironmentPosture, Error, ExecutionIdentityPosture,
    FeatureFlag, Hello, IsolationPosture, LauncherIcon, LauncherItemKind, LauncherItemSummary,
    OperationId, ProtocolToken, SemverRange, SessionPersistencePosture, Version,
    WorkloadExecutionPosture, WorkloadProviderKind, WorkloadState, decode_json_body,
    ids::{RealmId, WorkloadId},
    token::TokenError,
    workload_identity::{WorkloadIdentity, WorkloadTarget},
};

#[test]
fn foundational_values_keep_validation_and_redaction_contracts() {
    assert_eq!(ProtocolToken::parse(""), Err(TokenError::Empty));
    assert!(ProtocolToken::parse("codec.v1").is_ok());
    assert!(OperationId::parse("operation-1").is_ok());

    let version = Version::new("0.4.0").expect("valid version");
    let range = SemverRange::new(">=0.4.0, <0.5.0").expect("valid range");
    assert!(range.allows(&version));
}

#[test]
fn neutral_contracts_are_available_from_the_canonical_root() {
    let operation = OperationId::parse("operation-1").expect("valid operation id");
    let item_id = ProtocolToken::parse("browser").expect("valid item id");
    let item = LauncherItemSummary {
        id: item_id.clone(),
        name: "Browser".to_owned(),
        icon: LauncherIcon::default(),
        kind: LauncherItemKind::Exec,
        graphical: true,
        capabilities: CapabilitySet::empty(),
    };
    let posture = WorkloadExecutionPosture {
        isolation: IsolationPosture::VirtualMachine,
        environment: EnvironmentPosture::RuntimeManaged,
        display_environment: DisplayEnvironmentPosture::NotApplicable,
        execution_identity: ExecutionIdentityPosture::ProviderManaged,
        session_persistence: SessionPersistencePosture::RuntimeManaged,
    };

    assert_eq!(operation.as_str(), "operation-1");
    assert_eq!(item.id, item_id);
    assert_eq!(item.kind, LauncherItemKind::Exec);
    assert_eq!(posture.isolation, IsolationPosture::VirtualMachine);
    assert_eq!(WorkloadProviderKind::LocalVm, WorkloadProviderKind::LocalVm);
    assert_eq!(WorkloadState::Running, WorkloadState::Running);
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
