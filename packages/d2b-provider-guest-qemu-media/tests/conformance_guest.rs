use d2b_provider_guest_qemu_media::{
    GuestProviderSpecSettings, GuestSpec, build_guest_resource_spec,
};

#[test]
fn guest_conformance_keeps_the_common_base_and_provider_extension_distinct() {
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
}
