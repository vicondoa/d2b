//! Stable systemd Provider error labels.

/// Closed systemd Provider error catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemdProviderError {
    /// Transient start exceeded its deadline.
    LaunchTimeout,
    /// User manager could not be reached.
    UserManagerUnavailable,
    /// Stable identity tuple did not match.
    IdentityMismatch,
    /// Effect port was unavailable.
    EffectPortUnavailable,
    /// Finalization lacked a terminal proof.
    FinalizeProofMissing,
}

impl SystemdProviderError {
    /// Return the stable lower-kebab code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::LaunchTimeout => "systemd-launch-timeout",
            Self::UserManagerUnavailable => "systemd-user-manager-unavailable",
            Self::IdentityMismatch => "systemd-identity-mismatch",
            Self::EffectPortUnavailable => "systemd-effect-port-unavailable",
            Self::FinalizeProofMissing => "systemd-finalize-proof-missing",
        }
    }
}

impl core::fmt::Display for SystemdProviderError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SystemdProviderError {}
