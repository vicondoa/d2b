//! Controller-side dependency and process projections.

pub mod device_watch;
pub mod display;
pub mod hotplug;
pub mod media_watch;
pub mod network;
pub mod process_builder;
pub mod reconcile;
pub mod status;
pub mod volume;

pub use device_watch::{
    AuthorityReservation, DeviceAdmission, DeviceAdmissionError, DeviceObservation, DevicePhase,
    HostGlobalAuthorityIndex, PlatformClass,
};
pub use display::{
    DisplayAttachment, DisplayObservation, DisplaySessionError, DisplaySessionPhase,
    WaylandSessionSpec,
};
pub use hotplug::{HotplugController, HotplugOperation, HotplugResult};
pub use media_watch::{
    MediaObservationError, MediaReadiness, MediaWatch, VolumeAttachment, VolumeObservation,
    VolumePhase,
};
pub use network::{NetworkLaunchError, NetworkLaunchEvent, TapAttachment, TapLaunchRouter};
pub use process_builder::{
    AttachmentKind, AttachmentSlot, LaunchTicket, ProcessSpec, ProcessSpecError,
    build_process_spec, validate_process_spec,
};
pub use reconcile::{
    QemuMediaController, QemuMediaDependencies, QemuMediaEffectPort, QemuMediaError,
    QemuMediaPhase, QemuMediaReconcileOutcome, QemuMediaRecoveryState,
};
pub use status::status_for;
pub use volume::{
    LayoutEntry, RuntimeVolumeSpec, RuntimeVolumeView, VolumeLayoutType, VolumeQuota,
};
