//! The closed volume-virtiofs error set.

use std::fmt;

/// Every failure the volume-virtiofs controller or its effect port may
/// report.
///
/// The set is closed and each variant renders one stable
/// `^[a-z][a-z0-9-]*$` code. A code never echoes an export socket path,
/// a shared directory, a unit name, argv, or a numeric identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum VirtiofsExportError {
    /// The Export does not satisfy a frozen conformance bound.
    InvalidExport,
    /// The Export names a Volume view the referenced Volume does not
    /// declare.
    ViewNotFound,
    /// The requested access exceeds the rights the selected view grants.
    ViewRightsInsufficient,
    /// The worker plan would violate the frozen virtiofsd sandbox
    /// posture.
    SandboxInvariantViolated,
    /// The worker could not be launched through the effect port.
    WorkerLaunchFailed,
    /// The export socket did not become ready inside the deadline.
    ExportNotReady,
    /// The guest did not report the mount present inside the deadline.
    GuestMountNotReady,
    /// The Export could not drain because a child is still present.
    DrainIncomplete,
    /// A store-view readiness marker is absent or non-empty.
    StoreViewMarkerMissing,
    /// Shared-write is not a supported Export access mode.
    SharedWriteUnsupported,
}

impl VirtiofsExportError {
    /// Return the stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidExport => "invalid-export",
            Self::ViewNotFound => "view-not-found",
            Self::ViewRightsInsufficient => "view-rights-insufficient",
            Self::SandboxInvariantViolated => "sandbox-invariant-violated",
            Self::WorkerLaunchFailed => "worker-launch-failed",
            Self::ExportNotReady => "export-not-ready",
            Self::GuestMountNotReady => "guest-mount-not-ready",
            Self::DrainIncomplete => "drain-incomplete",
            Self::StoreViewMarkerMissing => "store-view-marker-missing",
            Self::SharedWriteUnsupported => "shared-write-unsupported",
        }
    }

    /// The complete closed code set, for conformance assertions.
    pub const ALL: [Self; 10] = [
        Self::InvalidExport,
        Self::ViewNotFound,
        Self::ViewRightsInsufficient,
        Self::SandboxInvariantViolated,
        Self::WorkerLaunchFailed,
        Self::ExportNotReady,
        Self::GuestMountNotReady,
        Self::DrainIncomplete,
        Self::StoreViewMarkerMissing,
        Self::SharedWriteUnsupported,
    ];
}

impl fmt::Display for VirtiofsExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for VirtiofsExportError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_unique_and_matches_the_frozen_grammar() {
        let mut codes: Vec<&str> = VirtiofsExportError::ALL
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
