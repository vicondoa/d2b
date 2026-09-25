//! Resource specifications for qemu-media Guests.

mod guest;

pub use guest::{
    Bios, CpuModel, DeviceAttachment, ExtraFeature, GuestProviderSpecSettings,
    GuestResourceSpecError, GuestSpecError, MachineType, MINIMAL_GUEST_BASE_JSON,
    NetworkAttachment, RemovableVolumeRef, RtcBase, audio_capability,
    build_guest_resource_spec, runtime_volume_name,
};

