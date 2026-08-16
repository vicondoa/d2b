//! integration-target: fake-host

use d2b_provider_runtime_qemu_media::{
    GuestProviderSpecSettings, GuestSpec, ProcessSpec, RuntimeVolumeSpec,
};

/// Build the cross-resource fixture used by the fake-host integration lane.
pub fn fixture() -> (
    GuestSpec,
    RuntimeVolumeSpec,
    ProcessSpec,
) {
    let guest_ref = d2b_contracts::v3::ResourceRef::parse("Guest/media-vm")
        .expect("fixture Guest ref");
    let volume_ref = d2b_contracts::v3::ResourceRef::parse("Volume/runtime")
        .expect("fixture Volume ref");
    let guest = GuestSpec::new(
        "Provider/runtime-qemu-media",
        Some(d2b_contracts::v3::ResourceRef::parse("Volume/boot").expect("fixture Volume ref")),
        2,
        4096,
        GuestProviderSpecSettings::default(),
    )
    .expect("fixture Guest");
    let volume =
        RuntimeVolumeSpec::new(guest_ref.clone(), "corp", 10 * 1024 * 1024, 1024)
            .expect("fixture runtime Volume");
    let process = ProcessSpec::new(
        guest_ref,
        d2b_contracts::v3::ResourceRef::parse("Host/host-system").expect("fixture Host ref"),
        volume_ref,
        Some(d2b_contracts::v3::ResourceRef::parse("Device/host-kvm").expect("fixture Device ref")),
        Vec::<d2b_contracts::v3::ResourceRef>::new(),
    )
    .expect("fixture Process");
    (guest, volume, process)
}
