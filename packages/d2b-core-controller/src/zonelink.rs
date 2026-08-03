//! ZoneLink cursor ownership and restart adoption.
//!
//! The transport state machine remains in [`crate::zone_links`].  This module
//! adds the authority-owned durable cursor envelope required by the
//! per-Zone coordinator: a cursor is usable only when its `ownerProof` is
//! unique and matches the handler's expected owner.  Ambiguous observations
//! are quarantined rather than guessed.

use crate::zone_links::{ZoneLinkCursor, ZoneLinkError};
use d2b_contracts::v3::SchemaFingerprint;

pub use crate::zone_links::{
    BootstrapPsk, SealedEnrollment, ZONE_LINK_METRIC_LABEL_KEYS, ZoneLinkEffect, ZoneLinkEvent,
    ZoneLinkHandler, ZoneLinkKeyPolicy, ZoneLinkLimits, ZoneLinkMetricSample, ZoneLinkPhase,
    ZoneLinkRecord, ZoneLinkSessionState, ZoneLinkStatus,
};
pub use d2b_contracts::v3::zone_routing::ZoneLinkControllerGeneration;

/// Closed failure from owner-proof cursor adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneLinkAdoptionError {
    /// No durable owner proof was available.
    OwnerProofMissing,
    /// More than one owner proof or cursor observation was present.
    AmbiguousOwner,
    /// The observed proof does not match the registered handler owner.
    OwnerProofMismatch,
    /// The cursor record cannot be used for this ZoneLink.
    CursorInvalid,
}

impl ZoneLinkAdoptionError {
    /// Return the stable fail-closed label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::OwnerProofMissing => "zonelink-owner-proof-missing",
            Self::AmbiguousOwner => "zonelink-owner-proof-ambiguous",
            Self::OwnerProofMismatch => "zonelink-owner-proof-mismatch",
            Self::CursorInvalid => "zonelink-cursor-invalid",
        }
    }
}

impl core::fmt::Display for ZoneLinkAdoptionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for ZoneLinkAdoptionError {}

/// Exact opaque owner proof issued by the Zone authority index.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneLinkOwnerProof {
    authority_generation: u64,
    owner_digest: SchemaFingerprint,
}

impl ZoneLinkOwnerProof {
    /// Bind a cursor owner to one authority generation and digest.
    pub fn new(
        authority_generation: u64,
        owner_digest: SchemaFingerprint,
    ) -> Result<Self, ZoneLinkAdoptionError> {
        if authority_generation == 0 {
            return Err(ZoneLinkAdoptionError::CursorInvalid);
        }
        Ok(Self {
            authority_generation,
            owner_digest,
        })
    }

    /// Build an owner proof from a canonical digest string.
    pub fn from_digest(
        authority_generation: u64,
        digest: impl Into<String>,
    ) -> Result<Self, ZoneLinkAdoptionError> {
        let digest = SchemaFingerprint::parse(digest.into())
            .map_err(|_| ZoneLinkAdoptionError::CursorInvalid)?;
        Self::new(authority_generation, digest)
    }

    /// Return the authority generation.
    pub const fn authority_generation(&self) -> u64 {
        self.authority_generation
    }

    /// Borrow the opaque owner digest.
    pub const fn owner_digest(&self) -> &SchemaFingerprint {
        &self.owner_digest
    }
}

impl core::fmt::Debug for ZoneLinkOwnerProof {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneLinkOwnerProof")
            .field("authority_generation", &self.authority_generation)
            .finish_non_exhaustive()
    }
}

/// One durable cursor row paired with the authority owner proof.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneLinkCursorRecord {
    owner_proof: ZoneLinkOwnerProof,
    cursor: ZoneLinkCursor,
}

impl ZoneLinkCursorRecord {
    /// Construct a cursor record after owner admission.
    pub const fn new(owner_proof: ZoneLinkOwnerProof, cursor: ZoneLinkCursor) -> Self {
        Self {
            owner_proof,
            cursor,
        }
    }

    /// Borrow the exact owner proof.
    pub const fn owner_proof(&self) -> &ZoneLinkOwnerProof {
        &self.owner_proof
    }

    /// Return the durable route cursor.
    pub const fn cursor(&self) -> ZoneLinkCursor {
        self.cursor
    }
}

impl core::fmt::Debug for ZoneLinkCursorRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneLinkCursorRecord")
            .field("owner_proof", &self.owner_proof)
            .field("has_cursor", &true)
            .finish()
    }
}

/// Restart result for a ZoneLink cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneLinkAdoption {
    /// One unique, matching proof owns the cursor.
    Adopted(ZoneLinkCursorRecord),
    /// The cursor must remain quarantined until an operator or authority
    /// resolves the ambiguity.
    Quarantined(ZoneLinkAdoptionError),
}

impl ZoneLinkAdoption {
    /// Whether the cursor is safe to use.
    pub const fn is_adopted(&self) -> bool {
        matches!(self, Self::Adopted(_))
    }

    /// Borrow an adopted record.
    pub const fn record(&self) -> Option<&ZoneLinkCursorRecord> {
        match self {
            Self::Adopted(record) => Some(record),
            Self::Quarantined(_) => None,
        }
    }

    /// Return the quarantine reason, if any.
    pub const fn quarantine_reason(&self) -> Option<ZoneLinkAdoptionError> {
        match self {
            Self::Adopted(_) => None,
            Self::Quarantined(error) => Some(*error),
        }
    }
}

/// Authority-owned ZoneLink cursor store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneLinkCursorAuthority {
    expected_owner: ZoneLinkOwnerProof,
    adoption: ZoneLinkAdoption,
}

impl ZoneLinkCursorAuthority {
    /// Restore one authority with no cursor observation.
    pub fn restore(expected_owner: ZoneLinkOwnerProof) -> Self {
        Self {
            expected_owner,
            adoption: ZoneLinkAdoption::Quarantined(ZoneLinkAdoptionError::OwnerProofMissing),
        }
    }

    /// Adopt observations from the durable ZoneLink cursor table.
    ///
    /// More than one durable observation is ambiguous, even when observations
    /// happen to carry the same proof and cursor. The method never chooses a
    /// cursor by recency or map iteration order.
    pub fn adopt(
        &mut self,
        observations: impl IntoIterator<Item = ZoneLinkCursorRecord>,
    ) -> ZoneLinkAdoption {
        let mut observed = observations.into_iter();
        let Some(first) = observed.next() else {
            self.adoption = ZoneLinkAdoption::Quarantined(ZoneLinkAdoptionError::OwnerProofMissing);
            return self.adoption.clone();
        };
        if first.owner_proof() != &self.expected_owner {
            self.adoption =
                ZoneLinkAdoption::Quarantined(ZoneLinkAdoptionError::OwnerProofMismatch);
            return self.adoption.clone();
        }
        if observed.next().is_some() {
            self.adoption = ZoneLinkAdoption::Quarantined(ZoneLinkAdoptionError::AmbiguousOwner);
            return self.adoption.clone();
        }
        self.adoption = ZoneLinkAdoption::Adopted(first);
        self.adoption.clone()
    }

    /// Borrow the owner proof fixed at handler registration.
    pub const fn expected_owner(&self) -> &ZoneLinkOwnerProof {
        &self.expected_owner
    }

    /// Return the current restart adoption state.
    pub const fn adoption(&self) -> &ZoneLinkAdoption {
        &self.adoption
    }

    /// Borrow the adopted cursor or fail closed while quarantined.
    pub fn cursor(&self) -> Result<ZoneLinkCursor, ZoneLinkAdoptionError> {
        self.adoption
            .record()
            .map(ZoneLinkCursorRecord::cursor)
            .ok_or_else(|| {
                self.adoption
                    .quarantine_reason()
                    .unwrap_or(ZoneLinkAdoptionError::OwnerProofMissing)
            })
    }
}

/// ZoneLink handler facade that owns cursor adoption for its link.
///
/// The transport handler and cursor authority are kept together so restart
/// recovery cannot be performed by a generic store caller. The caller supplies
/// only durable observations; the owner proof is fixed when this handler is
/// constructed.
pub struct ZoneLinkController {
    handler: ZoneLinkHandler,
    cursor_authority: ZoneLinkCursorAuthority,
}

impl ZoneLinkController {
    /// Restore one handler and bind its cursor authority to one owner proof.
    pub fn restore(
        limits: ZoneLinkLimits,
        key_policy: ZoneLinkKeyPolicy,
        record: ZoneLinkRecord,
        owner_proof: ZoneLinkOwnerProof,
    ) -> Self {
        Self {
            handler: ZoneLinkHandler::restore(limits, key_policy, record),
            cursor_authority: ZoneLinkCursorAuthority::restore(owner_proof),
        }
    }

    /// Borrow the transport/session handler.
    pub const fn handler(&self) -> &ZoneLinkHandler {
        &self.handler
    }

    /// Mutably borrow the transport/session handler.
    pub fn handler_mut(&mut self) -> &mut ZoneLinkHandler {
        &mut self.handler
    }

    /// Adopt exactly one owner-proof-bound cursor after restart.
    pub fn adopt_cursor(
        &mut self,
        observations: impl IntoIterator<Item = ZoneLinkCursorRecord>,
    ) -> ZoneLinkAdoption {
        self.cursor_authority.adopt(observations)
    }

    /// Borrow the handler-owned cursor authority.
    pub const fn cursor_authority(&self) -> &ZoneLinkCursorAuthority {
        &self.cursor_authority
    }
}

/// Map the existing ZoneLink state machine's transport refusal into the
/// authority-owned quarantine vocabulary where appropriate.
pub const fn transport_error_is_quarantine(error: ZoneLinkError) -> bool {
    matches!(
        error,
        ZoneLinkError::StaleCommitProof | ZoneLinkError::ReconcileInFlight
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(digit: char) -> ZoneLinkOwnerProof {
        ZoneLinkOwnerProof::from_digest(1, format!("sha256:{}", digit.to_string().repeat(64)))
            .unwrap()
    }

    #[test]
    fn matching_owner_proof_adopts_one_cursor() {
        let proof = owner('a');
        let cursor = ZoneLinkCursor::default();
        let mut authority = ZoneLinkCursorAuthority::restore(proof.clone());
        let result = authority.adopt([ZoneLinkCursorRecord::new(proof, cursor)]);
        assert!(result.is_adopted());
        assert_eq!(authority.cursor(), Ok(cursor));
    }

    #[test]
    fn missing_or_ambiguous_owner_is_quarantined() {
        let proof = owner('a');
        let mut authority = ZoneLinkCursorAuthority::restore(proof.clone());
        assert_eq!(
            authority.adopt([]).quarantine_reason(),
            Some(ZoneLinkAdoptionError::OwnerProofMissing)
        );
        let other = owner('b');
        let result = authority.adopt([
            ZoneLinkCursorRecord::new(proof.clone(), ZoneLinkCursor::default()),
            ZoneLinkCursorRecord::new(other, ZoneLinkCursor::default()),
        ]);
        assert_eq!(
            result.quarantine_reason(),
            Some(ZoneLinkAdoptionError::AmbiguousOwner)
        );
        assert!(authority.cursor().is_err());
    }

    #[test]
    fn duplicate_owner_observations_are_quarantined() {
        let proof = owner('a');
        let cursor = ZoneLinkCursor::default();
        let mut authority = ZoneLinkCursorAuthority::restore(proof.clone());
        let result = authority.adopt([
            ZoneLinkCursorRecord::new(proof.clone(), cursor),
            ZoneLinkCursorRecord::new(proof, cursor),
        ]);
        assert_eq!(
            result.quarantine_reason(),
            Some(ZoneLinkAdoptionError::AmbiguousOwner)
        );
        assert!(authority.cursor().is_err());
    }

    #[test]
    fn handler_owns_restart_cursor_adoption() {
        let owner = owner('a');
        let mut controller = ZoneLinkController::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(
                ZoneLinkControllerGeneration::parse("link-generation-1").unwrap(),
            ),
            owner.clone(),
        );
        let result =
            controller.adopt_cursor([ZoneLinkCursorRecord::new(owner, ZoneLinkCursor::default())]);
        assert!(result.is_adopted());
        assert_eq!(
            controller.cursor_authority().cursor(),
            Ok(ZoneLinkCursor::default())
        );
        assert_eq!(
            controller.handler().record().cursor(),
            ZoneLinkCursor::default()
        );
    }
}
