use d2b_provider_runtime_qemu_media::{
    GuestPhase, GuestProviderSpecSettings, GuestSpec, GuestStatus, ProviderPhase,
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
fn status_provider_phase_is_closed_and_bounded() {
    let status = GuestStatus::new(GuestPhase::Pending, ProviderPhase::WaitingDependencies);
    assert_eq!(status.phase(), GuestPhase::Pending);
    assert_eq!(status.provider_phase(), ProviderPhase::WaitingDependencies);
    assert!(GuestStatus::from_provider_phase("not-a-provider-phase").is_err());
    assert!(GuestStatus::from_provider_phase(&"x".repeat(65)).is_err());
}

#[test]
fn guest_spec_requires_the_runtime_provider() {
    let settings = GuestProviderSpecSettings::default();
    assert!(GuestSpec::new("Provider/runtime-qemu-media", None, 2, 4096, settings,).is_ok());
    assert!(
        GuestSpec::new(
            "Provider/other",
            None,
            2,
            4096,
            GuestProviderSpecSettings::default()
        )
        .is_err()
    );
}
