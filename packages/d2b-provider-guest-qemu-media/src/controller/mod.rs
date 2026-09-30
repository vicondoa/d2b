//! Controller-side dependency and process projections.

pub mod attachments;
pub mod device_watch;
pub mod process_builder;
pub mod reconcile;
pub mod volume;

pub use attachments::{
    AdmittedAttachment, AdmittedAttachments, AdmittedRelationship, AttachmentSlot,
    DeclaredAttachments, GuestMediaBindings, ImplementationLeg, LaunchAttachments,
    MediaAdmissionError, MediaRequest, RelationshipEvidence, SlotRequirements,
    DISPLAY_PURPOSE, DISPLAY_SLOT, KVM_FUNCTION, KVM_SLOT, MEDIA_SLOT_PREFIX,
    QEMU_MEDIA_REALIZATION_FACETS, TAP_SLOT, qemu_media_realization_support,
};
pub use device_watch::{
    DeviceAdmission, DeviceAdmissionError, DeviceObservation, DevicePhase, PlatformClass,
};
pub use process_builder::{
    AttachmentKind, LaunchTicket, ProcessSpec, ProcessSpecError, build_process_spec,
    validate_process_spec,
};
pub use reconcile::{
    QemuMediaController, QemuMediaDependencies, QemuMediaEffectPort, QemuMediaError,
    QemuMediaPhase, QemuMediaReconcileOutcome, QemuMediaRecoveryState,
};
pub use volume::{
    LayoutEntry, RuntimeVolumeSpec, RuntimeVolumeView, VolumeLayoutType, VolumeQuota,
    RUNTIME_VOLUME_MOUNT_PATH, RUNTIME_VOLUME_RUNNER_VIEW, RUNTIME_VOLUME_SLOT,
};
