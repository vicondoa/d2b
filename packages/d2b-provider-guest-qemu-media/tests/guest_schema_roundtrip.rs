use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_provider_guest_qemu_media::{
    GuestProviderSpecSettings, MINIMAL_GUEST_BASE_JSON, build_guest_resource_spec,
};

#[test]
fn guest_settings_round_trip_without_host_locators() {
    let json = r#"{
        "bootMediaRef": "Volume/boot-media",
        "bootMediaView": "guest-attach",
        "removableVolumeRefs": [
            {"volumeRef": "Volume/removable", "view": "guest-attach"}
        ],
        "cpuModel": "host",
        "machineType": "q35",
        "bios": "ovmf",
        "pauseAtBoot": true,
        "displayWindow": false,
        "serialConsole": true,
        "tablet": true,
        "rtcBase": "utc",
        "extraFeatures": []
    }"#;

    let settings: GuestProviderSpecSettings = serde_json::from_str(json).unwrap();
    let rendered = serde_json::to_string(&settings).unwrap();
    assert!(rendered.contains("Volume/boot-media"));
    assert!(!rendered.contains("path"));
    assert!(!rendered.contains("argv"));
    assert!(!rendered.contains("credential"));
}

#[test]
fn unknown_fields_and_invalid_refs_are_rejected() {
    assert!(
        serde_json::from_str::<GuestProviderSpecSettings>(
            r#"{"bootMediaRef":"Host/not-a-volume"}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<GuestProviderSpecSettings>(r#"{"unexpected":true}"#).is_err());
}

#[test]
fn guest_spec_requires_the_runtime_provider() {
    let settings = GuestProviderSpecSettings::default();
    let resource = build_guest_resource_spec(None, 2, 4096, settings).unwrap();
    assert_eq!(
        resource.provider_ref().unwrap().to_canonical_string(),
        "Provider/runtime-qemu-media"
    );
    assert!(
        d2b_contracts_resource::v3::ResourceSpec::new(
            Some(d2b_contracts_resource::v3::ResourceRef::parse("Provider/other").unwrap()),
            None,
            d2b_contracts_resource::v3::CanonicalJsonObject::empty(),
            None,
        )
        .is_ok()
    );
}

#[test]
fn canonical_guest_base_round_trips_and_rejects_shadow_fields() {
    let base = CanonicalJsonObject::parse(MINIMAL_GUEST_BASE_JSON.as_bytes()).unwrap();
    let rendered = serde_json::to_string(&base).unwrap();
    assert_eq!(
        CanonicalJsonObject::parse(rendered.as_bytes()).unwrap(),
        base,
        "the pinned minimal Guest base must round-trip through its canonical form"
    );
    for reserved in ["providerRef", "updatePolicy", "provider"] {
        assert!(
            !MINIMAL_GUEST_BASE_JSON.contains(reserved),
            "the minimal Guest base must not restate a universal or Provider-layer field"
        );
    }
    // Shadow-field refusal is the typed shape's behavior; the owning crate's
    // tests pin it (d2b-provider-guest guest_spec tests), and the daemon
    // decodes stored specs through that typed shape.
}
