use d2b_provider_runtime_qemu_media::{
    GuestPhase, GuestProviderSpecSettings, GuestSpec, GuestStatus, ProviderPhase,
    build_guest_resource_spec,
};

#[test]
fn guest_conformance_keeps_common_and_provider_status_layers_distinct() {
    let spec = build_guest_resource_spec(
        Some(d2b_contracts_resource::v3::ResourceRef::parse("Volume/boot").unwrap()),
        2,
        4096,
        GuestProviderSpecSettings::default(),
    )
    .unwrap();
    let base: GuestSpec =
        serde_json::from_slice(&serde_json::to_vec(&GuestSpec::system_default()).unwrap()).unwrap();
    assert_eq!(base, GuestSpec::system_default());
    assert_eq!(
        spec.provider().unwrap().schema_id().to_canonical_string(),
        "runtime-qemu-media.d2bus.org/Guest/spec"
    );
    let status = GuestStatus::new(GuestPhase::Ready, ProviderPhase::PausedAtBoot);
    assert_eq!(status.phase(), GuestPhase::Ready);
    assert_eq!(status.provider_phase(), ProviderPhase::PausedAtBoot);
    assert_eq!(
        status.provider.schema_id,
        "runtime-qemu-media.d2bus.org/Guest/status"
    );
}
