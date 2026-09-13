//! Canonical `Provider/runtime-qemu-media` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod adoption;
pub mod config;
pub mod controller;
pub mod qmp;
pub mod types;

pub use adoption::{AdoptionOutcome, ProcessIdentity, verify_identity};
pub use config::{
    ControllerConfigProjection, ProviderConfig, ProviderConfigError, WorkerConfigProjection,
};
pub use controller::{
    AttachmentKind, AttachmentSlot, DeviceAdmission, DeviceAdmissionError, DeviceObservation,
    DevicePhase, LayoutEntry, LaunchTicket, PlatformClass, ProcessSpec, ProcessSpecError,
    QemuMediaController, QemuMediaDependencies, QemuMediaEffectPort, QemuMediaError,
    QemuMediaPhase, QemuMediaReconcileOutcome, QemuMediaRecoveryState, RuntimeVolumeSpec,
    RuntimeVolumeView, VolumeLayoutType, VolumeQuota, build_process_spec, validate_process_spec,
};
pub use controller::reconcile::QEMU_MEDIA_REPAIR_INTERVAL_SECS;
pub use qmp::{
    QmpCommand, QmpError, QmpGreeting, QmpReply, QmpSession, QmpTransport, QmpVmStatus,
    ScriptedQmpTransport,
};
pub use types::{
    Bios, CpuModel, DeviceAttachment, ExtraFeature, GuestProviderSpecSettings,
    GuestResourceSpecError, GuestSpec, GuestSpecError, MachineType, NetworkAttachment,
    RemovableVolumeRef, RtcBase, build_guest_resource_spec,
};

/// Stable Provider implementation identifier.
pub const QEMU_MEDIA_IMPLEMENTATION_ID: &str = "qemu-media";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-qemu-media";
/// Stable Guest finalizer.
pub const FINALIZER: &str = "runtime-qemu-media.d2bus.org/guest-cleanup";
