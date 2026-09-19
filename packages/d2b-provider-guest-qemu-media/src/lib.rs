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
pub use controller::process_builder::PROCESS_TEMPLATE;
pub use controller::reconcile::QEMU_MEDIA_REPAIR_INTERVAL_SECS;
pub use qmp::{
    QmpCommand, QmpError, QmpGreeting, QmpReply, QmpSession, QmpTransport, QmpVmStatus,
    ScriptedQmpTransport,
};
pub use types::{
    Bios, CpuModel, DeviceAttachment, ExtraFeature, GuestProviderSpecSettings,
    GuestResourceSpecError, GuestSpecError, MachineType, MINIMAL_GUEST_BASE_JSON,
    NetworkAttachment, RemovableVolumeRef, RtcBase, audio_capability,
    build_guest_resource_spec, runtime_volume_name,
};

/// The device-admission media contract id this Provider's controller
/// validates observations against.
///
/// The daemon reads this id instead of spelling the contract itself.
pub const MEDIA_CONTRACT_ID: &str = "qemu-media/v1";

/// Stable Provider implementation identifier.
pub const QEMU_MEDIA_IMPLEMENTATION_ID: &str = "qemu-media";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-qemu-media";
/// Stable Guest finalizer.
pub const FINALIZER: &str = "runtime-qemu-media.d2bus.org/guest-cleanup";
