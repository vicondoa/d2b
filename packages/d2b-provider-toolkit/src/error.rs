//! The closed Provider toolkit error set.

use std::fmt;

/// Every failure the Provider agent bootstrap, dispatch accounting, or
/// audit log may report.
///
/// The set is closed and each variant renders one stable
/// `^[a-z][a-z0-9-]*$` code, matching the condition and outcome `code`
/// grammar frozen by D108. A code never echoes caller input, a Provider
/// name, a Zone label, a digest, or a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ProviderToolkitError {
    /// The allocator-issued bootstrap binding names a different Provider
    /// than the agent entrypoint was compiled for.
    BootstrapProviderMismatch,
    /// The bootstrap binding names a Zone other than the one the agent
    /// was placed in.
    BootstrapZoneMismatch,
    /// The bootstrap binding does not reference a `Provider` resource.
    BootstrapRefWrongType,
    /// The bootstrap binding carries a session purpose the Provider agent
    /// entrypoint does not accept.
    BootstrapPurposeMismatch,
    /// The bootstrap binding is not carried on a session the Zone
    /// allocator issued locally.
    BootstrapLocalityRejected,
    /// A requested bounded capacity is zero or above its frozen ceiling.
    CapacityOutOfRange,
    /// No dispatch slot is available; the agent is already serving its
    /// frozen in-flight maximum.
    DispatchSaturated,
}

impl ProviderToolkitError {
    /// Return the stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BootstrapProviderMismatch => "bootstrap-provider-mismatch",
            Self::BootstrapZoneMismatch => "bootstrap-zone-mismatch",
            Self::BootstrapRefWrongType => "bootstrap-ref-wrong-type",
            Self::BootstrapPurposeMismatch => "bootstrap-purpose-mismatch",
            Self::BootstrapLocalityRejected => "bootstrap-locality-rejected",
            Self::CapacityOutOfRange => "capacity-out-of-range",
            Self::DispatchSaturated => "dispatch-saturated",
        }
    }

    /// The complete closed code set, for conformance assertions.
    pub const ALL: [Self; 7] = [
        Self::BootstrapProviderMismatch,
        Self::BootstrapZoneMismatch,
        Self::BootstrapRefWrongType,
        Self::BootstrapPurposeMismatch,
        Self::BootstrapLocalityRejected,
        Self::CapacityOutOfRange,
        Self::DispatchSaturated,
    ];
}

impl fmt::Display for ProviderToolkitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ProviderToolkitError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::check_closed_code_set;

    #[test]
    fn every_code_is_unique_and_matches_the_frozen_grammar() {
        let codes: Vec<&str> = ProviderToolkitError::ALL
            .iter()
            .map(|error| error.code())
            .collect();
        assert!(check_closed_code_set(&codes).is_ok());
    }
}
