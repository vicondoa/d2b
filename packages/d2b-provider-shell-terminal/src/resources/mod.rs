//! Qualified resource schemas for shell pools and sessions.

mod pool;
mod session;

pub use pool::{
    DEFAULT_MAX_ATTACHED, DEFAULT_MAX_SESSIONS, DEFAULT_OUTPUT_RING_CAPACITY, ExecutionTarget,
    PoolSpec, ShellPool,
};
pub use session::{SessionPhase, ShellSession};

/// Stable validation errors for shell-terminal resource schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTerminalError {
    /// A qualified resource, zone, session, or user name was invalid.
    InvalidName,
    /// A bounded pool or session capacity was outside its allowed range.
    CapacityOutOfRange,
    /// A login shell artifact reference was not a bounded artifact reference.
    InvalidLoginShell,
    /// The caller lacks the Zone-scoped shell-administration role.
    NotAuthorized,
    /// The authenticated request belongs to a different Zone.
    WrongZone,
    /// A relay-authenticated request targeted a Host user-domain pool.
    RelayHostUserDomainDenied,
    /// The requested user identity was not verified for Host placement.
    WorkloadIdentityMismatch,
    /// The Guest does not admit user-domain supervisors.
    GuestUserDomainUnsupported,
    /// A supervisor request carried a stale or unknown generation.
    StaleSessionGeneration,
    /// A one-shot session capability was presented more than once.
    CapabilityReused,
    /// A request exceeded a bounded attachment or output limit.
    CapacityExceeded,
    /// Restart adoption could not identify exactly one owned supervisor.
    SupervisorAmbiguous,
}

impl std::fmt::Display for ShellTerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidName => "shell-terminal-name-invalid",
            Self::CapacityOutOfRange => "shell-terminal-capacity-out-of-range",
            Self::InvalidLoginShell => "shell-terminal-login-shell-invalid",
            Self::NotAuthorized => "shell-terminal-not-authorized",
            Self::WrongZone => "shell-terminal-zone-mismatch",
            Self::RelayHostUserDomainDenied => "shell-terminal-relay-host-user-domain-denied",
            Self::WorkloadIdentityMismatch => "shell-terminal-workload-identity-mismatch",
            Self::GuestUserDomainUnsupported => "shell-terminal-guest-user-domain-unsupported",
            Self::StaleSessionGeneration => "shell-terminal-stale-session-generation",
            Self::CapabilityReused => "shell-terminal-capability-reused",
            Self::CapacityExceeded => "shell-terminal-capacity-exceeded",
            Self::SupervisorAmbiguous => "shell-terminal-supervisor-ambiguous",
        })
    }
}

impl std::error::Error for ShellTerminalError {}

pub(crate) fn validate_name(value: &str, maximum: usize) -> Result<(), ShellTerminalError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' => true,
            b'0'..=b'9' | b'-' => index > 0,
            _ => false,
        })
    {
        return Err(ShellTerminalError::InvalidName);
    }
    Ok(())
}
