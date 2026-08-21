//! Pure verification and finalization eligibility contracts.

use std::collections::BTreeSet;
use std::fmt;

use crate::{
    inventory::HostInventory,
    model::{FailureCode, ZoneId},
};

/// Verification result for one Zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneVerification {
    zone_id: ZoneId,
    healthy: bool,
}

impl ZoneVerification {
    /// Construct a Zone verification observation.
    pub fn new(zone_id: ZoneId, healthy: bool) -> Self {
        Self { zone_id, healthy }
    }

    /// Borrow the Zone identity.
    pub fn zone_id(&self) -> &ZoneId {
        &self.zone_id
    }

    /// Return whether this Zone passed.
    pub const fn healthy(&self) -> bool {
        self.healthy
    }
}

/// Pure inputs to the phase-9 verification contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationInput {
    zones: Vec<ZoneVerification>,
    sources_preserved: bool,
    identity_digests_match: bool,
    audit_durable: bool,
    candidate_current: bool,
}

impl VerificationInput {
    /// Construct verification inputs.
    pub fn new(
        zones: impl IntoIterator<Item = ZoneVerification>,
        sources_preserved: bool,
        identity_digests_match: bool,
        audit_durable: bool,
        candidate_current: bool,
    ) -> Self {
        Self {
            zones: zones.into_iter().collect(),
            sources_preserved,
            identity_digests_match,
            audit_durable,
            candidate_current,
        }
    }
}

/// Successful verification proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    zone_count: usize,
    sources_preserved: bool,
    identity_digests_match: bool,
    audit_durable: bool,
    candidate_current: bool,
}

impl VerificationReport {
    /// Return the number of verified Zones.
    pub const fn zone_count(&self) -> usize {
        self.zone_count
    }

    /// Return whether source preservation passed.
    pub const fn sources_preserved(&self) -> bool {
        self.sources_preserved
    }

    /// Return whether identity digests matched.
    pub const fn identity_digests_match(&self) -> bool {
        self.identity_digests_match
    }

    /// Return whether the audit evidence was durable.
    pub const fn audit_durable(&self) -> bool {
        self.audit_durable
    }

    /// Return whether the candidate remained current.
    pub const fn candidate_current(&self) -> bool {
        self.candidate_current
    }
}

/// Verify all configured Zones and identity-preservation invariants.
pub fn verify_cutover(
    inventory: &HostInventory,
    input: &VerificationInput,
) -> Result<VerificationReport, VerificationError> {
    let expected = inventory.zone_ids().collect::<BTreeSet<_>>();
    let observed = input
        .zones
        .iter()
        .map(|zone| zone.zone_id())
        .collect::<BTreeSet<_>>();
    if expected != observed {
        return Err(VerificationError::ZoneSetMismatch);
    }
    if input.zones.iter().any(|zone| !zone.healthy()) {
        return Err(VerificationError::ZoneUnhealthy);
    }
    if !input.sources_preserved {
        return Err(VerificationError::SourcesNotPreserved);
    }
    if !input.identity_digests_match {
        return Err(VerificationError::IdentityDigestMismatch);
    }
    if !input.audit_durable {
        return Err(VerificationError::AuditNotDurable);
    }
    if !input.candidate_current {
        return Err(VerificationError::CandidateDrift);
    }
    Ok(VerificationReport {
        zone_count: expected.len(),
        sources_preserved: input.sources_preserved,
        identity_digests_match: input.identity_digests_match,
        audit_durable: input.audit_durable,
        candidate_current: input.candidate_current,
    })
}

/// Verification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationError {
    /// The observed Zone set did not exactly match configured Zones.
    ZoneSetMismatch,
    /// A Zone failed health verification.
    ZoneUnhealthy,
    /// A source artifact was not preserved.
    SourcesNotPreserved,
    /// An identity digest changed.
    IdentityDigestMismatch,
    /// The verification audit was not durable.
    AuditNotDurable,
    /// The candidate drifted before verification.
    CandidateDrift,
}

impl VerificationError {
    /// Return the stable failure class.
    pub const fn code(self) -> FailureCode {
        match self {
            Self::ZoneSetMismatch | Self::ZoneUnhealthy => FailureCode::VerificationIncomplete,
            Self::SourcesNotPreserved => FailureCode::SourceNotPreserved,
            Self::IdentityDigestMismatch => FailureCode::IdentityMismatch,
            Self::AuditNotDurable => FailureCode::AuditNotDurable,
            Self::CandidateDrift => FailureCode::CandidateDrift,
        }
    }
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZoneSetMismatch => "verification Zone set mismatch",
            Self::ZoneUnhealthy => "verification Zone unhealthy",
            Self::SourcesNotPreserved => "verification source was not preserved",
            Self::IdentityDigestMismatch => "verification identity digest mismatch",
            Self::AuditNotDurable => "verification audit is not durable",
            Self::CandidateDrift => "verification candidate drift",
        })
    }
}

impl std::error::Error for VerificationError {}
