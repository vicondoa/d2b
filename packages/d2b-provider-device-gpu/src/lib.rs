//! Combined GPU/video Device Provider contracts.
//!
//! Core resolves the opaque GPU effect-token set into broker `OpenDevice` and
//! `SpawnRunner` operations. This crate never receives a device path, socket,
//! capability, or ambient host permission.

#![deny(missing_docs)]

mod controller;
mod effects;
mod process;
mod settings;
mod wire;

pub use controller::{GpuController, GpuControllerError, GpuPhase, GpuReconcileOutcome};
pub use effects::{
    GpuEffectError, GpuEffectPort, GpuEffectToken, GpuEffectTokenSet, GpuLaunchTicket,
};
pub use process::{
    GpuProcessDeclaration, GpuProcessRole, GpuProcessSelectionError, gpu_process_name,
};
pub use settings::{ContextType, DisplayConfig, GpuSettings, GpuSettingsError};
pub use wire::{
    VHOST_USER_MEDIA_NUM_QUEUES, VHOST_USER_MEDIA_PROTOCOL_FLAGS, VHOST_USER_MEDIA_QUEUE_SIZE,
    VHOST_USER_MEDIA_SHM_REGION_BYTES, VHOST_USER_MEDIA_VRING_BASE, VIRTIO_ID_MEDIA,
    wire_contract_snapshot,
};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-gpu";
/// Device extension schema identifier.
pub const DEVICE_GPU_SCHEMA_ID: &str = "device-gpu.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_GPU_FINALIZER: &str = "device-gpu.d2bus.org/worker-stopped";
