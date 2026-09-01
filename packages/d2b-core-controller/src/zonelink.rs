//! ZoneLink cursor ownership and restart adoption.
//!
//! The transport state machine remains in [`crate::zone_links`].  This module
//! adds the authority-owned durable cursor envelope required by the
//! per-Zone coordinator: a cursor is usable only when its `ownerProof` is
//! unique and matches the handler's expected owner.  Ambiguous observations
//! are quarantined rather than guessed.

use crate::zone_links::{ZoneLinkCursor, ZoneLinkError};
use d2b_contracts_resource::v3::SchemaFingerprint;

pub use crate::zone_links::{
    BootstrapPsk, SealedEnrollment, ZONE_LINK_METRIC_LABEL_KEYS, ZoneLinkEffect, ZoneLinkEvent,
    ZoneLinkHandler, ZoneLinkKeyPolicy, ZoneLinkLimits, ZoneLinkMetricSample, ZoneLinkPhase,
    ZoneLinkRecord, ZoneLinkRouteBinding, ZoneLinkSessionState, ZoneLinkStatus,
};
pub use d2b_contracts_zone_session::v3::zone_routing::{
    ZoneLinkControllerGeneration, ZoneLinkRouteAdmissionRequest,
};

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

    fn quarantine(&mut self, error: ZoneLinkAdoptionError) -> ZoneLinkAdoption {
        self.adoption = ZoneLinkAdoption::Quarantined(error);
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
        let result = self.cursor_authority.adopt(observations);
        let result = match result {
            ZoneLinkAdoption::Adopted(record)
                if record.cursor() != self.handler.record().cursor() =>
            {
                self.cursor_authority
                    .quarantine(ZoneLinkAdoptionError::CursorInvalid)
            }
            result => result,
        };
        if result.is_adopted() {
            self.handler.mark_cursor_adopted();
        } else {
            self.handler.clear_cursor_adoption();
        }
        result
    }

    /// Issue one route admission only after cursor adoption and durable commit.
    ///
    /// The callback is the hand-off to the existing runtime-owned sealed
    /// issuer. Core supplies only the operation request extracted from the
    /// committed context; the issuer owns authentication, clock, expiry, and
    /// evidence sealing.
    pub fn issue_route_admission<T>(
        &mut self,
        request: ZoneLinkRouteAdmissionRequest,
        issuer: impl FnOnce(ZoneLinkRouteAdmissionRequest) -> Result<T, ZoneLinkError>,
    ) -> Result<T, ZoneLinkError> {
        if !self.cursor_authority.adoption().is_adopted() {
            return Err(ZoneLinkError::RouteAdmissionCursorUnavailable);
        }
        let pass = self.handler.begin(ZoneLinkEvent::AdmitRoute { request })?;
        let proof = self.handler.commit(pass)?;
        self.handler.issue_route_admission(proof, issuer)
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
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_resource::v3::{ResourceUid, ZoneRevision};
    use d2b_contracts_zone_session::v3::component_session::{OperationClass, OperationId};
    use d2b_contracts_zone_session::v3::zone_routing::{
        ZoneLabelId, ZonePath, ZoneRouteCapability, ZoneSigningKeyFingerprint, ZoneTreeEdge,
    };

    fn owner(digit: char) -> ZoneLinkOwnerProof {
        ZoneLinkOwnerProof::from_digest(1, format!("sha256:{}", digit.to_string().repeat(64)))
            .unwrap()
    }

    fn route_controller() -> ZoneLinkController {
        let generation = ZoneLinkControllerGeneration::parse("link-generation-1").unwrap();
        let binding = ZoneLinkRouteBinding::new(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174010").unwrap(),
            ZoneTreeEdge::new(
                ZonePath::new(vec![ZoneLabelId::parse("parent").unwrap()]).unwrap(),
                ZonePath::new(vec![
                    ZoneLabelId::parse("child").unwrap(),
                    ZoneLabelId::parse("parent").unwrap(),
                ])
                .unwrap(),
            )
            .unwrap(),
            generation.clone(),
            ReconnectGeneration::new(7).unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174011").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174012").unwrap(),
            ZoneRevision::new(9),
            ZoneRouteCapability::parse("resource-read").unwrap(),
            OperationClass::Invoke,
        )
        .unwrap();
        ZoneLinkController::restore(
            ZoneLinkLimits::default(),
            ZoneLinkKeyPolicy::default(),
            ZoneLinkRecord::unenrolled(generation)
                .with_route_binding(binding)
                .unwrap(),
            owner('a'),
        )
    }

    fn route_request(marker: u8) -> ZoneLinkRouteAdmissionRequest {
        ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![marker; 16]).unwrap(),
            OperationClass::Invoke,
        )
        .unwrap()
    }

    fn drive_to_ready(controller: &mut ZoneLinkController) {
        let generation = ZoneLinkControllerGeneration::parse("link-generation-1").unwrap();
        let fingerprint = ZoneSigningKeyFingerprint::parse("fp-child-static-1").unwrap();
        let enrollment = SealedEnrollment::new(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            fingerprint,
        );
        let psk = |issuance| BootstrapPsk::issue(generation.clone(), issuance, 300_000);
        let apply = |controller: &mut ZoneLinkController, event| {
            let pass = controller.handler_mut().begin(event).unwrap();
            let proof = controller.handler_mut().commit(pass).unwrap();
            controller.handler_mut().release_effects(proof).unwrap();
        };
        apply(
            controller,
            ZoneLinkEvent::BootstrapAdmit {
                psk: psk(1),
                now_ms: 0,
            },
        );
        apply(controller, ZoneLinkEvent::SealEnrollment { enrollment });
        apply(controller, ZoneLinkEvent::BeginEnrolledHandshake);
        apply(
            controller,
            ZoneLinkEvent::EnrolledSessionEstablished {
                peer_key_fingerprint: ZoneSigningKeyFingerprint::parse("fp-child-static-1")
                    .unwrap(),
            },
        );
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

    #[test]
    fn controller_issues_only_after_unique_cursor_adoption_and_committed_state() {
        let mut controller = route_controller();
        drive_to_ready(&mut controller);
        let request = route_request(1);
        assert_eq!(
            controller.issue_route_admission(request.clone(), |_| Ok(())),
            Err(ZoneLinkError::RouteAdmissionCursorUnavailable)
        );

        let wrong = owner('b');
        assert!(
            !controller
                .adopt_cursor([ZoneLinkCursorRecord::new(wrong, ZoneLinkCursor::default())])
                .is_adopted()
        );
        assert_eq!(
            controller.issue_route_admission(request.clone(), |_| Ok(())),
            Err(ZoneLinkError::RouteAdmissionCursorUnavailable)
        );

        let right = owner('a');
        assert!(
            controller
                .adopt_cursor([ZoneLinkCursorRecord::new(right, ZoneLinkCursor::default())])
                .is_adopted()
        );
        let issued = controller
            .issue_route_admission(request.clone(), |request| {
                assert_eq!(request.verb(), OperationClass::Invoke);
                Ok(request)
            })
            .expect("cursor owner and durable route commit are proven");
        assert_eq!(issued.operation_id(), request.operation_id());
        assert_eq!(
            controller.issue_route_admission(request, |_| Ok(())),
            Err(ZoneLinkError::RouteAdmissionConflict)
        );
    }

    #[test]
    fn matching_owner_with_substituted_cursor_stays_quarantined() {
        let mut controller = route_controller();
        drive_to_ready(&mut controller);
        let pass = controller
            .handler_mut()
            .begin(ZoneLinkEvent::ResyncCursor {
                sent: 1,
                acked: 1,
                received: 1,
                applied: 1,
            })
            .unwrap();
        let proof = controller.handler_mut().commit(pass).unwrap();
        controller.handler_mut().release_effects(proof).unwrap();

        let result = controller.adopt_cursor([ZoneLinkCursorRecord::new(
            owner('a'),
            ZoneLinkCursor::default(),
        )]);
        assert_eq!(
            result.quarantine_reason(),
            Some(ZoneLinkAdoptionError::CursorInvalid)
        );
        assert_eq!(
            controller.issue_route_admission(route_request(2), |_| Ok(())),
            Err(ZoneLinkError::RouteAdmissionCursorUnavailable)
        );
    }
}
