//! Wayland display projection Provider.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
mod policy;
mod principal;
mod process;
mod runtime;
mod spec;
#[allow(missing_docs)]
pub mod wayland_proxy;

pub use controller::{
    AuthenticatedDisplaySession, CapabilityReadiness, CleanupState, DependencyReadiness,
    DependencyState, DisplayController, DisplayDependencyProof, DisplayRunnerContract,
    DISPLAY_MAX_REPAIR_INTERVAL_SECS, DISPLAY_REPAIR_INTERVAL_SECS, FinalizationDecision,
    FinalizationInput, GraceState, Phase, PrincipalReleaseReceipt, ReconcileResult,
    SessionCondition, StopRequest, WaylandPolicySnapshot, WaylandSessionResourceStatus,
    WaylandSessionStatus, display_runner_contract,
};
pub use policy::{
    CompiledWaylandPolicy, FilterInput, KNOWN_GLOBALS, PolicyCompileError, PolicyWarning,
    WaylandPolicy,
};
pub use principal::{PrincipalLease, PrincipalPool, PrincipalPoolError};
pub use process::DisplayLaunchBinding;
pub use process::{
    AttachmentGrantHandle, DisplayProcessRole, LaunchGrants, LaunchTicket, ProcessObservation,
    VolumeState, WorkerAction, WorkerRestartEvidence, WorkerState, WorkerSupervisor,
    WorkerSupervisorError,
};
pub use runtime::{
    DisplayProcessEffectPort, DisplayRuntime, DisplayRuntimeError, FinalizationReport,
    WorkerEffectError, WorkerLaunchReceipt,
};
pub use spec::{DisplayIdentity, DisplayLabelPosition, WaylandSessionSpec, WaylandSpecError};

/// Canonical Provider reference.
pub const PROVIDER_REF: &str = "Provider/display-wayland";
/// Canonical display ComponentSession service package.
pub const SERVICE_PACKAGE: &str = "d2b.display.v3";
/// Canonical Provider artifact identifier.
pub const ARTIFACT_ID: &str = "display-wayland";
/// Host clipboard service consumed by clipd-host.
pub const HOST_CLIPBOARD_SERVICE: &str = "d2b.display.host-clipboard.v3";
/// Internal bridge service consumed by the display proxy.
pub const CLIPBOARD_BRIDGE_SERVICE: &str = "d2b.clipboard.bridge.v3";
/// Display-session finalizer.
pub const FINALIZER: &str = "display-wayland.d2bus.org/proxy-stopped";
