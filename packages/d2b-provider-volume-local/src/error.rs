//! The closed volume-local error set.

use std::fmt;

/// Every failure the volume-local controller or its effect ports may
/// report.
///
/// The set is closed and each variant renders one stable
/// `^[a-z][a-z0-9-]*$` code. A code never echoes caller input, an entry
/// path, a host path, an ACL value, or a source policy ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum VolumeLocalError {
    /// The Volume spec does not satisfy a frozen volume-local bound.
    InvalidSpec,
    /// The Volume selects a different Volume Provider.
    ProviderMismatch,
    /// The Volume names a source kind this Provider does not support.
    SourceKindUnsupported,
    /// The effect adapter could not resolve the opaque source policy ID.
    SourceUnresolved,
    /// A declared layout entry is absent and its policy forbids creating
    /// it.
    EntryMissing,
    /// A layout entry drifted and its repair policy performs no repair.
    EntryDrift,
    /// A layout entry could not be adopted without ambiguity, so it is
    /// held rather than deleted or reused.
    EntryQuarantined,
    /// A declared fail-closed layout invariant does not hold.
    InvariantViolated,
    /// A symlink was met on a path walk that declares `noFollow`.
    SymlinkTraversalRejected,
    /// Children carry ACL entries the declared ACL does not cover while
    /// `foreignChildPolicy` is `fail`.
    ForeignAclViolation,
    /// A `create-if-never-provisioned` entry is absent after its
    /// provisioning marker was written.
    PreviouslyProvisionedStateMissing,
    /// An attachment or mount names a view the Volume does not declare.
    ViewNotFound,
    /// The requested access exceeds the rights the selected view grants.
    ViewRightsInsufficient,
    /// A second simultaneous writer was requested for the Volume.
    SingleWriterConflict,
    /// `shared-write` was requested from a Provider that does not declare
    /// `supportsSharedWrite`.
    SharedWriteUnsupported,
    /// Hard quota enforcement was requested and the backing filesystem
    /// cannot enforce it.
    QuotaUnenforceable,
    /// A write would exceed the declared byte or inode ceiling.
    QuotaExceeded,
    /// The effect adapter could not complete the requested layout effect.
    EffectFailed,
    /// The opaque source-policy ID is not present in the Provider policy
    /// catalog.
    SourcePolicyNotFound,
    /// A source policy exists, but its class or allowed Volume kind does not
    /// match the Volume spec.
    SourcePolicyMismatch,
    /// A block-image Volume did not declare a byte ceiling.
    BlockImageQuotaMissing,
    /// A tmpfs Volume did not declare both kernel-enforced ceilings.
    TmpfsQuotaMissing,
    /// A source kind was paired with an incompatible Volume kind.
    SourceKindVolumeKindMismatch,
    /// A block-image attachment selected a filesystem transport.
    BlockImageTransportMismatch,
    /// A path used a Unicode separator lookalike rather than an anchored
    /// filesystem separator.
    UnicodePathSeparator,
    /// A store-view readiness marker was not present.
    StoreViewMarkerMissing,
    /// A shared-write attachment is not supported by the selected Provider.
    SharedWriteCapabilityMissing,
}

impl VolumeLocalError {
    /// Return the stable lower-kebab code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSpec => "invalid-spec",
            Self::ProviderMismatch => "provider-mismatch",
            Self::SourceKindUnsupported => "source-kind-unsupported",
            Self::SourceUnresolved => "source-unresolved",
            Self::EntryMissing => "entry-missing",
            Self::EntryDrift => "entry-drift",
            Self::EntryQuarantined => "entry-quarantined",
            Self::InvariantViolated => "invariant-violated",
            Self::SymlinkTraversalRejected => "symlink-traversal-rejected",
            Self::ForeignAclViolation => "foreign-acl-violation",
            Self::PreviouslyProvisionedStateMissing => "previously-provisioned-state-missing",
            Self::ViewNotFound => "view-not-found",
            Self::ViewRightsInsufficient => "view-rights-insufficient",
            Self::SingleWriterConflict => "single-writer-conflict",
            Self::SharedWriteUnsupported => "shared-write-unsupported",
            Self::QuotaUnenforceable => "quota-unenforceable",
            Self::QuotaExceeded => "volume-quota-exceeded",
            Self::EffectFailed => "effect-failed",
            Self::SourcePolicyNotFound => "volume-source-policy-not-found",
            Self::SourcePolicyMismatch => "volume-source-policy-mismatch",
            Self::BlockImageQuotaMissing => "volume-block-image-quota-missing",
            Self::TmpfsQuotaMissing => "volume-tmpfs-quota-missing",
            Self::SourceKindVolumeKindMismatch => "volume-source-kind-mismatch",
            Self::BlockImageTransportMismatch => "volume-block-image-transport-mismatch",
            Self::UnicodePathSeparator => "volume-unicode-path-separator",
            Self::StoreViewMarkerMissing => "volume-store-view-marker-missing",
            Self::SharedWriteCapabilityMissing => "volume-shared-write-capability-missing",
        }
    }

    /// The complete closed code set, for conformance assertions.
    pub const ALL: [Self; 27] = [
        Self::InvalidSpec,
        Self::ProviderMismatch,
        Self::SourceKindUnsupported,
        Self::SourceUnresolved,
        Self::EntryMissing,
        Self::EntryDrift,
        Self::EntryQuarantined,
        Self::InvariantViolated,
        Self::SymlinkTraversalRejected,
        Self::ForeignAclViolation,
        Self::PreviouslyProvisionedStateMissing,
        Self::ViewNotFound,
        Self::ViewRightsInsufficient,
        Self::SingleWriterConflict,
        Self::SharedWriteUnsupported,
        Self::QuotaUnenforceable,
        Self::QuotaExceeded,
        Self::EffectFailed,
        Self::SourcePolicyNotFound,
        Self::SourcePolicyMismatch,
        Self::BlockImageQuotaMissing,
        Self::TmpfsQuotaMissing,
        Self::SourceKindVolumeKindMismatch,
        Self::BlockImageTransportMismatch,
        Self::UnicodePathSeparator,
        Self::StoreViewMarkerMissing,
        Self::SharedWriteCapabilityMissing,
    ];
}

impl fmt::Display for VolumeLocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for VolumeLocalError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_unique_and_matches_the_frozen_grammar() {
        let mut codes: Vec<&str> = VolumeLocalError::ALL
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
