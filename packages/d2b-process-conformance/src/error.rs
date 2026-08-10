//! The closed Process conformance error set.

use std::fmt;

/// Every failure a Process Provider or its effect port may report.
///
/// The set is closed and each variant renders one stable
/// `^[a-z][a-z0-9-]*$` code, matching the condition and outcome `code`
/// grammar frozen by D108. A code never echoes caller input, a path, a unit
/// name, or argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ProcessConformanceError {
    /// The ticket does not satisfy the frozen launch-ticket bounds.
    InvalidTicket,
    /// The ticket names an execution domain this Provider does not support.
    DomainNotSupported,
    /// A user-domain ticket carries no exact `userRef`.
    UserRefRequired,
    /// The ticket selects a different Process Provider.
    ProviderMismatch,
    /// A required identity binding was not verified by the effect adapter.
    IdentityUnverified,
    /// The effect adapter returned no verified pidfd evidence.
    PidfdUnavailable,
    /// The effect adapter could not launch the process.
    LaunchFailed,
    /// Observed identity is ambiguous; the process is quarantined.
    AdoptionAmbiguous,
    /// The effect adapter reported a wait and reap owner the Provider does
    /// not own.
    WaitOwnerMismatch,
    /// The launch did not complete inside the ticket deadline.
    DeadlineExceeded,
    /// A terminal result was not bound to one process and operation.
    TerminalEvidenceMismatch,
    /// A terminal result carried an invalid exit status or identity.
    InvalidTerminalResult,
    /// The semantic sandbox plan is not admitted by the Provider.
    SandboxRejected,
    /// An intentional stop lacks one of its required terminal proofs.
    StopProofMissing,
    /// Linux/cgroup platform prerequisites are not satisfied.
    PlatformGateRejected,
    /// The launch operation was cancelled before the effect boundary.
    Cancelled,
    /// The provider-specific stop effect was not available.
    StopUnavailable,
}

impl ProcessConformanceError {
    /// Return the stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidTicket => "invalid-ticket",
            Self::DomainNotSupported => "domain-not-supported",
            Self::UserRefRequired => "user-ref-required",
            Self::ProviderMismatch => "provider-mismatch",
            Self::IdentityUnverified => "identity-unverified",
            Self::PidfdUnavailable => "pidfd-unavailable",
            Self::LaunchFailed => "launch-failed",
            Self::AdoptionAmbiguous => "adoption-ambiguous",
            Self::WaitOwnerMismatch => "wait-owner-mismatch",
            Self::DeadlineExceeded => "deadline-exceeded",
            Self::TerminalEvidenceMismatch => "terminal-evidence-mismatch",
            Self::InvalidTerminalResult => "invalid-terminal-result",
            Self::SandboxRejected => "sandbox-rejected",
            Self::StopProofMissing => "stop-proof-missing",
            Self::PlatformGateRejected => "platform-gate-rejected",
            Self::Cancelled => "cancelled",
            Self::StopUnavailable => "stop-unavailable",
        }
    }

    /// The complete closed code set, for conformance assertions.
    pub const ALL: [Self; 17] = [
        Self::InvalidTicket,
        Self::DomainNotSupported,
        Self::UserRefRequired,
        Self::ProviderMismatch,
        Self::IdentityUnverified,
        Self::PidfdUnavailable,
        Self::LaunchFailed,
        Self::AdoptionAmbiguous,
        Self::WaitOwnerMismatch,
        Self::DeadlineExceeded,
        Self::TerminalEvidenceMismatch,
        Self::InvalidTerminalResult,
        Self::SandboxRejected,
        Self::StopProofMissing,
        Self::PlatformGateRejected,
        Self::Cancelled,
        Self::StopUnavailable,
    ];
}

impl fmt::Display for ProcessConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ProcessConformanceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_unique_and_matches_the_frozen_grammar() {
        let mut codes: Vec<&str> = ProcessConformanceError::ALL
            .iter()
            .map(|error| error.code())
            .collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total);
        for code in codes {
            assert!((1..=64).contains(&code.len()));
            let mut bytes = code.bytes();
            assert!(matches!(bytes.next(), Some(b'a'..=b'z')));
            assert!(
                bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            );
        }
    }
}
