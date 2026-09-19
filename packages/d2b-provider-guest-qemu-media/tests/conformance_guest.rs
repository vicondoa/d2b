use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_provider_guest_qemu_media::{
    GuestProviderSpecSettings, MINIMAL_GUEST_BASE_JSON, build_guest_resource_spec,
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
    let base = CanonicalJsonObject::parse(MINIMAL_GUEST_BASE_JSON.as_bytes()).unwrap();
    assert_eq!(spec.base(), &base);
    assert_eq!(
        spec.provider().unwrap().schema_id().to_canonical_string(),
        "runtime-qemu-media.d2bus.org/Guest/spec"
    );
}