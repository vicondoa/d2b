//! Combined GPU/video Device Provider contracts.
//!
//! Core resolves the opaque GPU effect-token set into broker `OpenDevice` and
//! `SpawnRunner` operations. This crate never receives a device path, socket,
//! capability, or ambient host permission.

#![deny(missing_docs)]

mod authority;
mod controller;
mod effects;
pub mod gpu_argv;
mod process;
mod settings;
pub mod video_argv;
mod workers;

pub use authority::{
    GpuAuthorityAdmission, GpuAuthorityError, GpuAuthorityLease, GpuBackingToken, GpuClosureProof,
    GpuOwnerProof, GpuPlatformToken, GpuPrincipalToken, GpuProcessIdentity, GpuProcessObservation,
};
pub use controller::{GpuController, GpuControllerError, GpuPhase, GpuReconcileOutcome};
pub use effects::{
    GpuEffectError, GpuEffectToken, GpuEffectTokenSet, GpuLaunchTicket, GpuLifecycleEffectPort,
};
pub use gpu_argv::{
    GpuArgvError, GpuArgvInput, GpuContextType, GpuDisplayConfig, GpuParams,
    exec_arg0 as gpu_exec_arg0, generate_gpu_argv,
};
pub use process::{
    GpuProcessDeclaration, GpuProcessRole, GpuProcessSelectionError, gpu_process_name,
};
pub use settings::{ContextType, DisplayConfig, GpuSettings, GpuSettingsError};
pub use video_argv::{
    VideoArgvError, VideoArgvInput, VideoBackend, exec_arg0 as video_exec_arg0,
    generate_video_argv, wire_contract_snapshot as video_wire_contract_snapshot,
};
pub use workers::{GpuWorkerSpec, VideoWorkerSpec};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-gpu";
/// Device extension schema identifier.
pub const DEVICE_GPU_SCHEMA_ID: &str = "device-gpu.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_GPU_FINALIZER: &str = "device-gpu.d2bus.org/worker-stopped";
