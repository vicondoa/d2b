use d2b_contracts::host_generation::{
    ApplyHostGenerationHandoff, HandoffCallerRole, HandoffError, HandoffState,
    HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1,
};
use d2b_contracts::v3::{
    ActivationDetail, ActivationMode, NIXOS_GENERATION_RESOURCE_TYPE, NixosGenerationSpec,
    ResourceRef,
};

fn provider_ref() -> ResourceRef {
    ResourceRef::parse("Provider/activation-nixos").expect("valid provider ref")
}

fn guest_ref() -> ResourceRef {
    ResourceRef::parse("Guest/dev-vm").expect("valid guest ref")
}

#[test]
fn generation_spec_round_trips_without_store_paths() {
    let spec = NixosGenerationSpec::new(
        provider_ref(),
        guest_ref(),
        "dev-vm-system",
        ActivationMode::Switch,
        None,
    )
    .expect("valid generation spec");
    let bytes = serde_json::to_vec(&spec).expect("serialize");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("\"systemArtifactId\":\"dev-vm-system\""));
    assert!(!text.contains("/nix/store"));
    assert!(!text.contains("activationDetail"));
    let parsed: NixosGenerationSpec = serde_json::from_str(&text).expect("round trip");
    assert_eq!(parsed, spec);
}

#[test]
fn generation_spec_rejects_wrong_refs_unknown_fields_and_status_fields() {
    assert!(
        NixosGenerationSpec::new(
            ResourceRef::parse("Provider/other").unwrap(),
            guest_ref(),
            "dev-vm-system",
            ActivationMode::Switch,
            None,
        )
        .is_err()
    );
    assert!(serde_json::from_str::<NixosGenerationSpec>(
        r#"{"providerRef":"Provider/activation-nixos","executionRef":"Guest/dev-vm","systemArtifactId":"dev-vm-system","activationMode":"switch","activationDetail":"applied"}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<NixosGenerationSpec>(
        r#"{"providerRef":"Provider/activation-nixos","executionRef":"Guest/dev-vm","systemArtifactId":"/nix/store/secret","activationMode":"switch"}"#,
    )
    .is_err());
}

#[test]
fn activation_detail_is_a_closed_status_value() {
    let encoded = serde_json::to_string(&ActivationDetail::BootDefault).unwrap();
    assert_eq!(encoded, "\"boot-default\"");
    assert!(serde_json::from_str::<ActivationDetail>("\"not-a-detail\"").is_err());
    assert_eq!(
        NIXOS_GENERATION_RESOURCE_TYPE,
        "activation-nixos.d2bus.org.NixosGeneration"
    );
}

#[test]
fn compatibility_floor_and_handoff_preserve_source_on_refusal_or_failure() {
    let floor = SourceGenerationCompatibilityFloorV1::new(7, [0x11; 32]).unwrap();
    assert_eq!(floor.protocol(), "source-handoff-v1");
    assert!(floor.validate_target(6, [0x11; 32]).is_err());

    let mut wrong_generation = floor.begin_handoff(7, 8).unwrap();
    assert_eq!(
        wrong_generation.validate_target(9, [0x11; 32]),
        Err(HandoffError::TargetGenerationMismatch)
    );
    assert!(wrong_generation.source_remains_usable());
    assert_eq!(wrong_generation.state(), HandoffState::Refused);

    let mut handoff = floor.begin_handoff(7, 8).unwrap();
    assert_eq!(handoff.state(), HandoffState::Recorded);
    assert_eq!(
        handoff.validate_target(8, [0x22; 32]),
        Err(HandoffError::TargetFingerprintMismatch)
    );
    assert!(handoff.source_remains_usable());
    assert_eq!(handoff.state(), HandoffState::Refused);
}

#[test]
fn broker_handoff_boundary_is_typed_and_caller_derived() {
    let floor = SourceGenerationCompatibilityFloorV1::new(7, [0x11; 32]).unwrap();
    let request = ApplyHostGenerationHandoff {
        caller_role: HandoffCallerRole::Lifecycle,
        target: guest_ref(),
        intent: HostGenerationHandoffIntent {
            source_generation: 7,
            target_generation: 8,
            system_artifact_id: d2b_contracts::v3::ArtifactId::parse("dev-vm-system").unwrap(),
            activation_mode: ActivationMode::Switch,
            compatibility: floor,
        },
    };
    assert!(request.validate().is_ok());
    let mut encoded = serde_json::to_value(request).unwrap();
    encoded["intent"]["systemArtifactId"] = serde_json::json!("/nix/store/private");
    assert!(serde_json::from_value::<ApplyHostGenerationHandoff>(encoded).is_err());
}
