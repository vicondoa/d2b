//! TPM Device Provider contracts.
//!
//! The Provider owns the Device controller and the semantic swtpm launch
//! plan. Core resolves every opaque ticket into a broker operation; this
//! crate never receives a state-directory path, socket path, executable path,
//! or broker connection.

#![deny(missing_docs)]

mod controller;
mod migration;
mod resource_controller;
mod resource_effect;
mod resources;
mod runner;
mod state;
mod status;
pub mod swtpm_argv;

pub use controller::{
    TpmController, TpmControllerError, TpmEffectError, TpmEffectPort, TpmPhase,
    TpmReconcileDisposition, TpmReconcileOutcome, TpmStatePreparationResult,
};
pub use migration::LegacyMigrationOutcome;
pub use resource_controller::{
    TPM_MAX_REPAIR_INTERVAL_SECS, TPM_REPAIR_INTERVAL_SECS, TpmResourceController,
    TpmResourceControllerError, TpmResourceOutcome, TpmResourcePhase, TpmRunnerContract,
    tpm_runner_contract,
};
pub use resource_effect::{TpmResourceEffectError, TpmResourceEffectPort};
pub use resources::{
    build_swtpm_flush_spec, build_swtpm_process_spec, build_tpm_state_volume_resource,
    build_tpm_state_volume_spec,
};
pub use runner::{
    BinaryKind, FlushLaunchTicket, SignedBinaryRef, SwtpmArgv, SwtpmArgvError, SwtpmSettings,
    SwtpmStartLaunchTicket, validate_start_ticket,
};
pub use state::{
    StateDirIntent, StateDirectoryToken, StateOwnerToken, TamperMarkerToken, TpmStateObservation,
    TpmStateObservationKind, TpmStatePreparation, TpmStateValidationError,
};
pub use status::{TpmMarkerStatus, TpmStatusReport};
pub use swtpm_argv::{
    SwtpmArgvInput, SwtpmIoctlFlushInput, exec_arg0, exec_arg0_flush, generate_swtpm_argv,
    generate_swtpm_ioctl_flush_argv,
};

/// Provider identity.
pub const PROVIDER_REF: &str = "Provider/device-tpm";
/// Device Provider schema identifier.
pub const DEVICE_TPM_SCHEMA_ID: &str = "device-tpm.d2bus.org/Device/spec";
/// Device Provider finalizer.
pub const DEVICE_TPM_FINALIZER: &str = "device-tpm.d2bus.org/state-preserved";
/// Endpoint purpose of the swtpm worker's server socket (`--server`) - the
/// endpoint the owning Guest's VMM consumes.
///
/// The declaring rows live in this Provider's `nix/default.nix`; the daemon's
/// Endpoint driver derives its realized device-worker family from these
/// constants, so the declared spelling and the admitted shape cannot drift
/// apart silently.
pub const TPM_SERVER_ENDPOINT_PURPOSE: &str = "swtpm-tpm-socket";
/// Endpoint purpose of the swtpm worker's control socket (`--ctrl`) - the
/// endpoint the one-shot pre-start flush connects to.
pub const TPM_CONTROL_ENDPOINT_PURPOSE: &str = "swtpm-control-socket";
/// Device Provider observe interval from the Device dossier.
pub const DEVICE_TPM_OBSERVE_INTERVAL_SECS: u64 = 30;
/// Minimum swtpm log level.
pub const MIN_SWTPM_LOG_LEVEL: u8 = 1;
/// Maximum swtpm log level.
pub const MAX_SWTPM_LOG_LEVEL: u8 = 20;
