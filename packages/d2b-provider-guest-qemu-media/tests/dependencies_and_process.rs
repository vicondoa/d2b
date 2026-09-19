use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_guest_qemu_media::{
    DeviceAdmission, DeviceObservation, DevicePhase, PlatformClass, ProcessSpec, RuntimeVolumeSpec,
    build_process_spec, runtime_volume_name, validate_process_spec,
};

fn guest() -> ResourceRef {
    ResourceRef::parse("Guest/media-vm").unwrap()
}

#[test]
fn runtime_volume_is_ephemeral_and_waits_for_process_proof() {
    let volume = RuntimeVolumeSpec::new(guest(), "corp", 10 * 1024 * 1024, 1024).unwrap();
    assert_eq!(volume.cleanup_policy(), "vm-stop-with-proof");
    assert_eq!(volume.owner_ref(), &guest());
    assert_eq!(
        volume.provider_ref.to_canonical_string(),
        "Provider/volume-local"
    );
    assert_eq!(volume.views.len(), 2);
    assert_eq!(
        volume.layout[0].restart_policy,
        "preserve-across-controller-restart"
    );
    assert_eq!(volume.layout[1].restart_policy, "clear-on-runner-restart");
    assert_eq!(volume.layout[2].restart_policy, "clear-on-runner-restart");
    assert!(volume.validate().is_ok());
    assert!(serde_json::to_string(&volume).unwrap().contains("qmp.sock"));
}

#[test]
fn runtime_volume_name_follows_the_declared_shape() {
    assert_eq!(runtime_volume_name("media-vm"), "media-vm-runtime");
    assert_eq!(runtime_volume_name("a"), "a-runtime");
    // The controller's runtime Volume row and the daemon's dependency lookup
    // both resolve the name from this declaration: the guest name plus the
    // declared suffix.
    let volume = RuntimeVolumeSpec::new(guest(), "corp", 10 * 1024 * 1024, 1024).unwrap();
    assert!(volume.name.ends_with("-runtime"));
    assert!(volume.validate().is_ok());
}

#[test]
fn device_admission_requires_the_guest_owner() {
    let observation = DeviceObservation {
        device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
        phase: DevicePhase::Ready,
        owner_ref: Some(ResourceRef::parse("Guest/other").unwrap()),
        platform: PlatformClass::X86_64Linux,
        authority_key: [7_u8; 32],
        process_identity: Some("process-a".to_owned()),
        media_contract: "qemu-media/v1".to_owned(),
    };
    assert!(
        DeviceAdmission::validate(&guest(), &observation, "process-a", "qemu-media/v1").is_err()
    );
    let owned = DeviceObservation {
        owner_ref: Some(guest()),
        ..observation
    };
    assert!(DeviceAdmission::validate(&guest(), &owned, "process-a", "qemu-media/v1").is_ok());
    let shared = DeviceObservation {
        owner_ref: None,
        ..owned
    };
    assert!(DeviceAdmission::validate(&guest(), &shared, "process-a", "qemu-media/v1").is_ok());
}

#[test]
fn process_spec_contains_only_opaque_attachments() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [ResourceRef::parse("Network/corp-net").unwrap()],
    )
    .unwrap();
    let json = serde_json::to_string(&process).unwrap();
    assert!(json.contains("qemu-media-runner"));
    assert!(!json.contains("argv"));
    assert!(!json.contains("/nix/store"));
    assert!(!json.contains("broker"));
}

#[test]
fn process_spec_uses_canonical_contract_and_rejects_shadow_fields() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [],
    )
    .unwrap();
    validate_process_spec(&process).unwrap();
    assert!(
        serde_json::from_str::<ProcessSpec>(
            r#"{"providerRef":"Provider/runtime-qemu-media","template":"qemu-media-runner"}"#
        )
        .is_err()
    );
}

#[test]
fn process_validation_checks_the_canonical_execution_template() {
    let process = build_process_spec(
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Volume/runtime").unwrap(),
        Some(ResourceRef::parse("Device/host-kvm").unwrap()),
        [],
    )
    .unwrap();
    let mut json = serde_json::to_value(&process).unwrap();
    json["template"] = serde_json::json!("wrong-template");
    let changed: ProcessSpec = serde_json::from_value(json).unwrap();
    assert!(validate_process_spec(&changed).is_err());
}
