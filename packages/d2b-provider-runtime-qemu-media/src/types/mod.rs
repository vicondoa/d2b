//! Resource specifications and status projections for qemu-media Guests.

mod guest;

pub use guest::{
    Bios, ConditionStatus, CpuModel, DeviceAttachment, ExtraFeature, GuestCondition, GuestPhase,
    GuestProviderDetails, GuestProviderSpecSettings, GuestProviderStatus, GuestRuntimeStatus,
    GuestSpec, GuestSpecError, GuestStatus, MachineType, NetworkAttachment, ProviderPhase,
    RemovableVolumeRef, RtcBase,
};
