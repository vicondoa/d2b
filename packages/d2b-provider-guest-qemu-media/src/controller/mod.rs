//! Controller-side dependency and process projections.

pub mod device_watch;
pub mod process_builder;
pub mod reconcile;
pub mod volume;

pub use device_watch::{
    DeviceAdmission, DeviceAdmissionError, DeviceObservation, DevicePhase, PlatformClass,
};
pub use process_builder::{
    AttachmentKind, AttachmentSlot, LaunchTicket, ProcessSpec, ProcessSpecError,
    build_process_spec, validate_process_spec,
};
pub use reconcile::{
    QemuMediaController, QemuMediaDependencies, QemuMediaEffectPort, QemuMediaError,
    QemuMediaPhase, QemuMediaReconcileOutcome, QemuMediaRecoveryState,
};
pub use volume::{
    LayoutEntry, RuntimeVolumeSpec, RuntimeVolumeView, VolumeLayoutType, VolumeQuota,
};
