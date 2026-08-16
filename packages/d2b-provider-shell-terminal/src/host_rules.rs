//! Host pool placement rules.

use crate::ShellTerminalError;

/// Declared isolation posture for a Host target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationPosture {
    /// The target exposes an isolated execution boundary.
    Isolated,
    /// The target is explicitly non-isolating.
    None,
}

/// Verified Host placement facts supplied by the fixed process effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPlacement {
    /// The Host target's declared posture.
    pub isolation_posture: IsolationPosture,
    /// Whether the supervisor principal exactly matched the pool workload user.
    pub workload_uid_verified: bool,
}

/// Validate that a Host session supervisor cannot use an ambient identity.
pub fn validate_host_placement(placement: &HostPlacement) -> Result<(), ShellTerminalError> {
    if !placement.workload_uid_verified {
        return Err(ShellTerminalError::WorkloadIdentityMismatch);
    }
    Ok(())
}
