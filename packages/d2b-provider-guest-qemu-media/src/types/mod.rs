//! Resource specifications for qemu-media Guests.

mod guest;

pub use guest::{
    Bios, CpuModel, DeviceAttachment, ExtraFeature, GuestProviderSpecSettings,
    GuestResourceSpecError, GuestSpec, GuestSpecError, MachineType, NetworkAttachment,
    RemovableVolumeRef, RtcBase, build_guest_resource_spec,
};
pub(crate) use guest::validate_token;
