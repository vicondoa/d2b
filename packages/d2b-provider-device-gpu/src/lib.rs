//! Combined GPU/video Device Provider contracts.
//!
//! Core resolves the opaque GPU effect-token set into broker `OpenDevice` and
//! `SpawnRunner` operations. This crate never receives a device path, socket,
//! capability, or ambient host permission.

#![deny(missing_docs)]

mod authority;
mod controller;
pub mod effects_service;
mod effects;
pub mod facets;
mod gpu_argv;
mod process;
mod settings;
mod video_argv;
pub mod vocabulary;
mod workers;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

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
    generate_gpu_argv,
};
pub use process::{
    GpuProcessDeclaration, GpuProcessRole, GpuProcessSelectionError, gpu_process_name,
};
pub use settings::{ContextType, DisplayConfig, GpuSettings, GpuSettingsError};
pub use video_argv::{
    VideoArgvError, VideoArgvInput, VideoBackend,
    generate_video_argv, wire_contract_snapshot as video_wire_contract_snapshot,
};
pub use workers::{GpuWorkerSpec, VideoWorkerSpec};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-gpu";
/// Device extension schema identifier.
pub const DEVICE_GPU_SCHEMA_ID: &str = "device-gpu.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_GPU_FINALIZER: &str = "device-gpu.d2bus.org/worker-stopped";
