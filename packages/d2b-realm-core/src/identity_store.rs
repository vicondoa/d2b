//! Pure in-memory realm identity metadata store.
//!
//! The store owns lifecycle metadata only: opaque refs, fingerprints, ids,
//! realm paths, statuses, low-cardinality reasons, and bounded audit/teardown
//! records. It never generates keys, signs data, writes files, stores secret
//! bytes, or tears down live runtime sessions.

use crate::capability::Capability;
use crate::enrollment::{
    ChildKeyPin, ControllerGenerationMetadata, ControllerGenerationStatus, EnrollmentReason,
    EnrollmentRecord, EnrollmentStatus, IdentityAuditEventKind, IdentityAuditEventMetadata,
    KeyRotationEvent, KeyRotationEventKind, KeyRotationPlan, KeyRotationStatus, KeyRotationSubject,
    MAX_TEARDOWN_WORKLOADS, ParentTrustAnchor, RealmIdentityStatus, RealmKeyRole,
    RecoveryProcedure, RecoveryStatus, RevocationList, RevocationRecord, RevocationStatus,
    RevocationTarget, SessionTeardownDirective, SessionTeardownReason,
};
use crate::ids::{
    ControllerGenerationCredentialRef, ControllerGenerationId, EnrollmentId, KeyRotationId,
    RealmIdentityRef, RecoveryProcedureId, RevocationId, RevocationListId, WorkloadId,
};
use crate::realm::RealmPath;
use crate::token::ProtocolToken;
use std::collections::{BTreeMap, BTreeSet};

/// Maximum audit records emitted by one pure metadata transition.
pub const MAX_IDENTITY_STORE_AUDIT_EVENTS: usize = 64;
/// Maximum teardown directives emitted by one pure metadata transition.
pub const MAX_IDENTITY_STORE_TEARDOWNS: usize = 64;

/// Result alias for pure identity-store transitions.
pub type IdentityStoreResult<T> = Result<T, IdentityStoreError>;

/// Fail-closed validation errors for identity metadata state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityStoreError {
    DuplicateEnrollment,
    DuplicateGeneration,
    DuplicateRevocation,
    DuplicateRecovery,
    UnknownEnrollment,
    UnknownGeneration,
    UnknownRecovery,
    UnknownRevocationList,
    StaleGeneration,
    RevokedGeneration,
    RevokedParentTrust,
    KeyMaterialRef,
    InvalidRealmRelationship,
    InvalidTransition,
    TooManyAuditEvents,
    TooManyTeardownDirectives,
}

impl IdentityStoreError {
    /// Stable, low-cardinality diagnostic code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DuplicateEnrollment => "duplicate-enrollment",
            Self::DuplicateGeneration => "duplicate-generation",
            Self::DuplicateRevocation => "duplicate-revocation",
            Self::DuplicateRecovery => "duplicate-recovery",
            Self::UnknownEnrollment => "unknown-enrollment",
            Self::UnknownGeneration => "unknown-generation",
            Self::UnknownRecovery => "unknown-recovery",
            Self::UnknownRevocationList => "unknown-revocation-list",
            Self::StaleGeneration => "stale-generation",
            Self::RevokedGeneration => "revoked-generation",
            Self::RevokedParentTrust => "revoked-parent-trust",
            Self::KeyMaterialRef => "key-material-ref",
            Self::InvalidRealmRelationship => "invalid-realm-relationship",
            Self::InvalidTransition => "invalid-transition",
            Self::TooManyAuditEvents => "too-many-audit-events",
            Self::TooManyTeardownDirectives => "too-many-teardown-directives",
        }
    }
}

impl core::fmt::Display for IdentityStoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for IdentityStoreError {}

/// Bounded, redacted metadata emitted by a store transition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityStoreChange {
    audit_events: Vec<IdentityAuditEventMetadata>,
    teardown_directives: Vec<SessionTeardownDirective>,
}

impl IdentityStoreChange {
    /// No-op transition metadata.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Redacted audit metadata emitted by the transition.
    pub fn audit_events(&self) -> &[IdentityAuditEventMetadata] {
        &self.audit_events
    }

    /// Metadata-only session teardown directives computed by the transition.
    pub fn teardown_directives(&self) -> &[SessionTeardownDirective] {
        &self.teardown_directives
    }

    fn push_audit(&mut self, event: IdentityAuditEventMetadata) -> IdentityStoreResult<()> {
        if self.audit_events.len() >= MAX_IDENTITY_STORE_AUDIT_EVENTS {
            return Err(IdentityStoreError::TooManyAuditEvents);
        }
        self.audit_events.push(event);
        Ok(())
    }

    fn push_teardown(&mut self, directive: SessionTeardownDirective) -> IdentityStoreResult<()> {
        if self.teardown_directives.len() >= MAX_IDENTITY_STORE_TEARDOWNS {
            return Err(IdentityStoreError::TooManyTeardownDirectives);
        }
        if directive.affected_workloads.len() > MAX_TEARDOWN_WORKLOADS {
            return Err(IdentityStoreError::TooManyTeardownDirectives);
        }
        self.teardown_directives.push(directive);
        Ok(())
    }

    fn extend(&mut self, other: IdentityStoreChange) -> IdentityStoreResult<()> {
        for event in other.audit_events {
            self.push_audit(event)?;
        }
        for directive in other.teardown_directives {
            self.push_teardown(directive)?;
        }
        Ok(())
    }
}

/// Pure in-memory metadata store for realm identity lifecycle state.
#[derive(Debug, Clone, Default)]
pub struct RealmIdentityStore {
    enrollments: BTreeMap<EnrollmentId, EnrollmentRecord>,
    enrollment_by_edge: BTreeMap<(RealmPath, RealmPath), EnrollmentId>,
    parent_trust_anchors: BTreeMap<(RealmPath, RealmPath), ParentTrustAnchor>,
    child_key_pins: BTreeMap<(RealmPath, RealmPath), ChildKeyPin>,
    controller_generations:
        BTreeMap<(RealmPath, ControllerGenerationId), ControllerGenerationMetadata>,
    active_generations: BTreeMap<RealmPath, ControllerGenerationId>,
    rotations: BTreeMap<KeyRotationId, KeyRotationPlan>,
    revocations: BTreeMap<RevocationId, RevocationRecord>,
    revocation_lists: BTreeMap<RevocationListId, RevocationList>,
    recoveries: BTreeMap<RecoveryProcedureId, RecoveryProcedure>,
}

impl RealmIdentityStore {
    /// Construct an empty pure metadata store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return an enrollment by id.
    pub fn enrollment(&self, id: &EnrollmentId) -> Option<&EnrollmentRecord> {
        self.enrollments.get(id)
    }

    /// Return a parent trust anchor for a parent/child edge.
    pub fn parent_trust_anchor(
        &self,
        parent: &RealmPath,
        child: &RealmPath,
    ) -> Option<&ParentTrustAnchor> {
        self.parent_trust_anchors
            .get(&(parent.clone(), child.clone()))
    }

    /// Return a child key pin for a parent/child edge.
    pub fn child_key_pin(&self, parent: &RealmPath, child: &RealmPath) -> Option<&ChildKeyPin> {
        self.child_key_pins.get(&(parent.clone(), child.clone()))
    }

    /// Return controller-generation metadata for `realm` and `generation`.
    pub fn controller_generation(
        &self,
        realm: &RealmPath,
        generation: &ControllerGenerationId,
    ) -> Option<&ControllerGenerationMetadata> {
        self.controller_generations
            .get(&(realm.clone(), generation.clone()))
    }

    /// Return the active controller-generation id for `realm`.
    pub fn active_generation(&self, realm: &RealmPath) -> Option<&ControllerGenerationId> {
        self.active_generations.get(realm)
    }

    /// Return a revocation-list snapshot by id.
    pub fn revocation_list(&self, id: &RevocationListId) -> Option<&RevocationList> {
        self.revocation_lists.get(id)
    }

    /// Return a recovery procedure by id.
    pub fn recovery(&self, id: &RecoveryProcedureId) -> Option<&RecoveryProcedure> {
        self.recoveries.get(id)
    }

    /// Insert a metadata-only enrollment record and its parent/child pins.
    pub fn enroll(&mut self, record: EnrollmentRecord) -> IdentityStoreResult<IdentityStoreChange> {
        self.validate_enrollment(&record)?;
        let id = record.enrollment_id.clone();
        let parent = record.parent_realm.clone();
        let child = record.child_realm.clone();
        if self.enrollments.contains_key(&id)
            || self
                .enrollment_by_edge
                .contains_key(&(parent.clone(), child.clone()))
        {
            return Err(IdentityStoreError::DuplicateEnrollment);
        }

        self.issue_controller_generation(record.controller_generation.clone())?;
        self.parent_trust_anchors.insert(
            (parent.clone(), child.clone()),
            record.parent_trust_anchor.clone(),
        );
        self.child_key_pins.insert(
            (parent.clone(), child.clone()),
            record.child_key_pin.clone(),
        );
        self.enrollment_by_edge
            .insert((parent.clone(), child.clone()), id.clone());
        self.enrollments.insert(id.clone(), record.clone());

        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: match record.status {
                EnrollmentStatus::Accepted => IdentityAuditEventKind::EnrollmentAccepted,
                EnrollmentStatus::Rejected => IdentityAuditEventKind::EnrollmentRejected,
                _ => IdentityAuditEventKind::EnrollmentRequested,
            },
            realm: child,
            enrollment_id: Some(id),
            rotation_id: None,
            revocation_id: None,
            recovery_id: None,
            enrollment_status: Some(record.status),
            rotation_status: None,
            revocation_status: None,
            recovery_status: None,
            correlation_id: record.correlation_id,
        })?;
        Ok(change)
    }

    /// Insert controller-generation metadata without generating or storing keys.
    pub fn issue_controller_generation(
        &mut self,
        generation: ControllerGenerationMetadata,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        validate_generation_refs(&generation)?;
        if generation.realm != generation.realm_identity.realm {
            return Err(IdentityStoreError::InvalidRealmRelationship);
        }
        if matches!(generation.status, ControllerGenerationStatus::Revoked) {
            return Err(IdentityStoreError::RevokedGeneration);
        }
        let key = (generation.realm.clone(), generation.generation_id.clone());
        if self.controller_generations.contains_key(&key) {
            return Err(IdentityStoreError::DuplicateGeneration);
        }
        if matches!(generation.status, ControllerGenerationStatus::Active) {
            match self.active_generations.get(&generation.realm) {
                Some(active) if active != &generation.generation_id => {
                    return Err(IdentityStoreError::StaleGeneration);
                }
                _ => {
                    self.active_generations
                        .insert(generation.realm.clone(), generation.generation_id.clone());
                }
            }
        }
        self.controller_generations.insert(key, generation.clone());

        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RotationActivated,
            realm: generation.realm,
            enrollment_id: None,
            rotation_id: None,
            revocation_id: generation.revoked_by,
            recovery_id: None,
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: None,
            correlation_id: None,
        })?;
        Ok(change)
    }

    /// Rotate a controller generation to an already-described replacement.
    /// The replacement must preserve the realm identity metadata.
    pub fn rotate_controller_generation(
        &mut self,
        mut plan: KeyRotationPlan,
        mut replacement: ControllerGenerationMetadata,
        activated_at_unix_seconds: u64,
    ) -> IdentityStoreResult<(KeyRotationEvent, IdentityStoreChange)> {
        validate_generation_refs(&replacement)?;
        validate_rotation_refs(&plan)?;
        let KeyRotationSubject::ControllerGeneration {
            realm,
            current_generation,
            current_credential_ref,
            current_fingerprint,
        } = &plan.subject
        else {
            return Err(IdentityStoreError::InvalidTransition);
        };
        self.ensure_active_generation(realm, current_generation)?;
        let current_key = (realm.clone(), current_generation.clone());
        let current = self
            .controller_generations
            .get(&current_key)
            .ok_or(IdentityStoreError::UnknownGeneration)?
            .clone();
        if current.credential_ref != *current_credential_ref
            || current.credential_fingerprint != *current_fingerprint
        {
            return Err(IdentityStoreError::StaleGeneration);
        }
        if replacement.realm != *realm || replacement.realm_identity != current.realm_identity {
            return Err(IdentityStoreError::InvalidTransition);
        }
        let replacement_key = (replacement.realm.clone(), replacement.generation_id.clone());
        if self.controller_generations.contains_key(&replacement_key) {
            return Err(IdentityStoreError::DuplicateGeneration);
        }

        if let Some(current) = self.controller_generations.get_mut(&current_key) {
            current.status = ControllerGenerationStatus::Superseded;
            current.not_after_unix_seconds = Some(activated_at_unix_seconds);
        }
        replacement.status = ControllerGenerationStatus::Active;
        replacement.not_before_unix_seconds = activated_at_unix_seconds;
        self.active_generations
            .insert(realm.clone(), replacement.generation_id.clone());
        self.controller_generations
            .insert(replacement_key, replacement.clone());

        plan.status = KeyRotationStatus::Active;
        plan.replacement_credential_ref = Some(replacement.credential_ref.clone());
        plan.replacement_fingerprint = Some(replacement.credential_fingerprint.clone());
        plan.activate_after_unix_seconds = Some(activated_at_unix_seconds);
        self.rotations
            .insert(plan.rotation_id.clone(), plan.clone());

        let event = KeyRotationEvent {
            rotation_id: plan.rotation_id.clone(),
            realm: realm.clone(),
            subject_kind: plan.subject.kind(),
            event: KeyRotationEventKind::Activated,
            status: KeyRotationStatus::Active,
            observed_at_unix_seconds: activated_at_unix_seconds,
            correlation_id: plan.correlation_id.clone(),
        };
        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RotationActivated,
            realm: realm.clone(),
            enrollment_id: None,
            rotation_id: Some(plan.rotation_id),
            revocation_id: None,
            recovery_id: None,
            enrollment_status: None,
            rotation_status: Some(KeyRotationStatus::Active),
            revocation_status: None,
            recovery_status: None,
            correlation_id: event.correlation_id.clone(),
        })?;
        Ok((event, change))
    }

    /// Revoke a controller generation and compute teardown metadata.
    pub fn revoke_generation(
        &mut self,
        revocation: RevocationRecord,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        match &revocation.target {
            RevocationTarget::ControllerGeneration { .. } => {
                self.revoke(revocation, affected_workloads)
            }
            _ => Err(IdentityStoreError::InvalidTransition),
        }
    }

    /// Revoke an entire realm and compute teardown metadata.
    pub fn revoke_realm(
        &mut self,
        revocation: RevocationRecord,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        match &revocation.target {
            RevocationTarget::Realm { .. } => self.revoke(revocation, affected_workloads),
            _ => Err(IdentityStoreError::InvalidTransition),
        }
    }

    /// Revoke a policy grant and compute teardown metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn revoke_policy_grant(
        &mut self,
        revocation_id: RevocationId,
        issuer_realm: RealmPath,
        issuer_controller_generation: ControllerGenerationId,
        realm: RealmPath,
        grant: ProtocolToken,
        issued_at_unix_seconds: u64,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        let record = RevocationRecord {
            revocation_id,
            issuer_realm,
            issuer_controller_generation,
            target: RevocationTarget::PolicyGrant { realm, grant },
            status: RevocationStatus::Effective,
            reason: crate::enrollment::RevocationReason::ParentPolicyRevoked,
            issued_at_unix_seconds,
            effective_at_unix_seconds: Some(issued_at_unix_seconds),
            correlation_id: None,
        };
        self.revoke(record, affected_workloads)
    }

    /// Revoke a stream/runtime capability and compute teardown metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn revoke_capability(
        &mut self,
        revocation_id: RevocationId,
        issuer_realm: RealmPath,
        issuer_controller_generation: ControllerGenerationId,
        realm: RealmPath,
        capability: Capability,
        issued_at_unix_seconds: u64,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        let record = RevocationRecord {
            revocation_id,
            issuer_realm,
            issuer_controller_generation,
            target: RevocationTarget::CapabilityGrant { realm, capability },
            status: RevocationStatus::Effective,
            reason: crate::enrollment::RevocationReason::ParentPolicyRevoked,
            issued_at_unix_seconds,
            effective_at_unix_seconds: Some(issued_at_unix_seconds),
            correlation_id: None,
        };
        self.revoke(record, affected_workloads)
    }

    /// Apply a metadata-only revocation record and compute teardown directives.
    pub fn revoke(
        &mut self,
        revocation: RevocationRecord,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        self.ensure_active_generation(
            &revocation.issuer_realm,
            &revocation.issuer_controller_generation,
        )?;
        self.apply_revocation(revocation, affected_workloads, false)
    }

    /// Merge a parent-pushed revocation list. Existing records are treated as
    /// idempotent replays, while new records are applied fail-closed.
    pub fn merge_revocation_list(
        &mut self,
        list: RevocationList,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        if self.revocation_lists.contains_key(&list.list_id) {
            return Err(IdentityStoreError::DuplicateRevocation);
        }
        self.ensure_parent_trust_not_revoked(&list.issuer_realm)?;
        let mut change = IdentityStoreChange::empty();
        let mut seen = BTreeSet::new();
        for record in &list.records {
            if !seen.insert(record.revocation_id.clone()) {
                return Err(IdentityStoreError::DuplicateRevocation);
            }
        }
        for record in list.records.iter().cloned() {
            if self.revocations.contains_key(&record.revocation_id) {
                continue;
            }
            change.extend(self.apply_revocation(record, affected_workloads.clone(), true)?)?;
        }
        self.revocation_lists
            .insert(list.list_id.clone(), list.clone());
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RevocationPropagated,
            realm: list.issuer_realm,
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: None,
            enrollment_status: None,
            rotation_status: None,
            revocation_status: Some(RevocationStatus::Propagated),
            recovery_status: None,
            correlation_id: list.correlation_id,
        })?;
        Ok(change)
    }

    /// Open a recovery procedure for a known generation.
    pub fn open_recovery(
        &mut self,
        mut recovery: RecoveryProcedure,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        if self.recoveries.contains_key(&recovery.recovery_id) {
            return Err(IdentityStoreError::DuplicateRecovery);
        }
        let key = (
            recovery.child_realm.clone(),
            recovery.affected_generation.clone(),
        );
        let generation = self
            .controller_generations
            .get_mut(&key)
            .ok_or(IdentityStoreError::UnknownGeneration)?;
        if matches!(generation.status, ControllerGenerationStatus::Revoked) {
            return Err(IdentityStoreError::RevokedGeneration);
        }
        recovery.status = RecoveryStatus::Requested;
        generation.status = ControllerGenerationStatus::Recovering;
        self.recoveries
            .insert(recovery.recovery_id.clone(), recovery.clone());
        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RecoveryOpened,
            realm: recovery.child_realm,
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: Some(recovery.recovery_id),
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: Some(RecoveryStatus::Requested),
            correlation_id: recovery.correlation_id,
        })?;
        Ok(change)
    }

    /// Mark parent approval for a requested recovery procedure.
    pub fn approve_recovery(
        &mut self,
        recovery_id: &RecoveryProcedureId,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        self.set_recovery_status(
            recovery_id,
            RecoveryStatus::Requested,
            RecoveryStatus::ParentApproved,
        )
    }

    /// Isolate sessions for an approved recovery procedure.
    pub fn isolate_recovery(
        &mut self,
        recovery_id: &RecoveryProcedureId,
        issued_at_unix_seconds: u64,
        affected_workloads: Vec<WorkloadId>,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        let recovery = self
            .recoveries
            .get_mut(recovery_id)
            .ok_or(IdentityStoreError::UnknownRecovery)?;
        if recovery.status != RecoveryStatus::ParentApproved {
            return Err(IdentityStoreError::InvalidTransition);
        }
        recovery.status = RecoveryStatus::Isolating;
        let mut change = IdentityStoreChange::empty();
        change.push_teardown(SessionTeardownDirective {
            revocation_id: RevocationId::parse(format!("recovery-{}", recovery.recovery_id))
                .map_err(|_| IdentityStoreError::InvalidTransition)?,
            issuer_realm: recovery.parent_realm.clone(),
            affected_realm: recovery.child_realm.clone(),
            reason: SessionTeardownReason::RecoveryIsolation,
            affected_workloads,
            issued_at_unix_seconds,
            correlation_id: recovery.correlation_id.clone(),
        })?;
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RecoveryOpened,
            realm: recovery.child_realm.clone(),
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: Some(recovery.recovery_id.clone()),
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: Some(RecoveryStatus::Isolating),
            correlation_id: recovery.correlation_id.clone(),
        })?;
        Ok(change)
    }

    /// Attach replacement generation metadata to an isolating recovery.
    pub fn reissue_recovery(
        &mut self,
        recovery_id: &RecoveryProcedureId,
        mut replacement: ControllerGenerationMetadata,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        validate_generation_refs(&replacement)?;
        let recovery = self
            .recoveries
            .get_mut(recovery_id)
            .ok_or(IdentityStoreError::UnknownRecovery)?;
        if recovery.status != RecoveryStatus::Isolating
            && recovery.status != RecoveryStatus::ParentApproved
        {
            return Err(IdentityStoreError::InvalidTransition);
        }
        let affected_key = (
            recovery.child_realm.clone(),
            recovery.affected_generation.clone(),
        );
        let affected = self
            .controller_generations
            .get(&affected_key)
            .ok_or(IdentityStoreError::UnknownGeneration)?
            .clone();
        if replacement.realm != recovery.child_realm
            || replacement.realm_identity != affected.realm_identity
        {
            return Err(IdentityStoreError::InvalidTransition);
        }
        let replacement_key = (replacement.realm.clone(), replacement.generation_id.clone());
        if self.controller_generations.contains_key(&replacement_key) {
            return Err(IdentityStoreError::DuplicateGeneration);
        }
        replacement.status = ControllerGenerationStatus::Recovering;
        self.controller_generations
            .insert(replacement_key, replacement.clone());
        recovery.replacement_generation = Some(replacement);
        recovery.status = RecoveryStatus::Reissued;
        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RecoveryOpened,
            realm: recovery.child_realm.clone(),
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: Some(recovery.recovery_id.clone()),
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: Some(RecoveryStatus::Reissued),
            correlation_id: recovery.correlation_id.clone(),
        })?;
        Ok(change)
    }

    /// Complete a recovery by activating its replacement generation and
    /// revoking the affected generation metadata.
    pub fn complete_recovery(
        &mut self,
        recovery_id: &RecoveryProcedureId,
        closed_at_unix_seconds: u64,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        let recovery = self
            .recoveries
            .get_mut(recovery_id)
            .ok_or(IdentityStoreError::UnknownRecovery)?;
        if recovery.status != RecoveryStatus::Reissued {
            return Err(IdentityStoreError::InvalidTransition);
        }
        let replacement = recovery
            .replacement_generation
            .clone()
            .ok_or(IdentityStoreError::InvalidTransition)?;
        let affected_key = (
            recovery.child_realm.clone(),
            recovery.affected_generation.clone(),
        );
        if let Some(affected) = self.controller_generations.get_mut(&affected_key) {
            affected.status = ControllerGenerationStatus::Revoked;
            affected.not_after_unix_seconds = Some(closed_at_unix_seconds);
        } else {
            return Err(IdentityStoreError::UnknownGeneration);
        }
        let replacement_key = (replacement.realm.clone(), replacement.generation_id.clone());
        let replacement_record = self
            .controller_generations
            .get_mut(&replacement_key)
            .ok_or(IdentityStoreError::UnknownGeneration)?;
        replacement_record.status = ControllerGenerationStatus::Active;
        replacement_record.not_before_unix_seconds = closed_at_unix_seconds;
        self.active_generations
            .insert(replacement.realm.clone(), replacement.generation_id.clone());
        recovery.status = RecoveryStatus::Completed;
        recovery.closed_at_unix_seconds = Some(closed_at_unix_seconds);

        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RecoveryCompleted,
            realm: recovery.child_realm.clone(),
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: Some(recovery.recovery_id.clone()),
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: Some(RecoveryStatus::Completed),
            correlation_id: recovery.correlation_id.clone(),
        })?;
        Ok(change)
    }

    fn set_recovery_status(
        &mut self,
        recovery_id: &RecoveryProcedureId,
        expected: RecoveryStatus,
        next: RecoveryStatus,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        let recovery = self
            .recoveries
            .get_mut(recovery_id)
            .ok_or(IdentityStoreError::UnknownRecovery)?;
        if recovery.status != expected {
            return Err(IdentityStoreError::InvalidTransition);
        }
        recovery.status = next;
        let mut change = IdentityStoreChange::empty();
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RecoveryOpened,
            realm: recovery.child_realm.clone(),
            enrollment_id: None,
            rotation_id: None,
            revocation_id: None,
            recovery_id: Some(recovery.recovery_id.clone()),
            enrollment_status: None,
            rotation_status: None,
            revocation_status: None,
            recovery_status: Some(next),
            correlation_id: recovery.correlation_id.clone(),
        })?;
        Ok(change)
    }

    fn validate_enrollment(&self, record: &EnrollmentRecord) -> IdentityStoreResult<()> {
        if !record.child_realm.is_direct_child_of(&record.parent_realm) {
            return Err(IdentityStoreError::InvalidRealmRelationship);
        }
        if record.parent_trust_anchor.parent_realm != record.parent_realm
            || record.parent_trust_anchor.child_realm != record.child_realm
            || record.child_key_pin.parent_realm != record.parent_realm
            || record.child_key_pin.child_realm != record.child_realm
            || record.child_key_pin.enrollment_id != record.enrollment_id
            || record.controller_generation.realm != record.child_realm
        {
            return Err(IdentityStoreError::InvalidRealmRelationship);
        }
        validate_identity_ref(&record.parent_trust_anchor.parent_identity_ref)?;
        validate_identity_ref(&record.child_key_pin.child_identity_ref)?;
        validate_generation_refs(&record.controller_generation)?;
        if self.is_parent_trust_revoked(&record.parent_trust_anchor) {
            return Err(IdentityStoreError::RevokedParentTrust);
        }
        Ok(())
    }

    fn ensure_active_generation(
        &self,
        realm: &RealmPath,
        generation: &ControllerGenerationId,
    ) -> IdentityStoreResult<()> {
        let metadata = self
            .controller_generations
            .get(&(realm.clone(), generation.clone()))
            .ok_or(IdentityStoreError::UnknownGeneration)?;
        if matches!(metadata.status, ControllerGenerationStatus::Revoked) {
            return Err(IdentityStoreError::RevokedGeneration);
        }
        match self.active_generations.get(realm) {
            Some(active) if active == generation => Ok(()),
            Some(_) => Err(IdentityStoreError::StaleGeneration),
            None => Err(IdentityStoreError::UnknownGeneration),
        }
    }

    fn ensure_parent_trust_not_revoked(&self, parent: &RealmPath) -> IdentityStoreResult<()> {
        let trusted = self
            .parent_trust_anchors
            .values()
            .any(|anchor| &anchor.parent_realm == parent && !self.is_parent_trust_revoked(anchor));
        if trusted {
            Ok(())
        } else {
            Err(IdentityStoreError::RevokedParentTrust)
        }
    }

    fn is_parent_trust_revoked(&self, anchor: &ParentTrustAnchor) -> bool {
        self.revocations.values().any(|record| {
            matches!(
                record.status,
                RevocationStatus::Effective | RevocationStatus::Propagated
            ) && match &record.target {
                RevocationTarget::Realm { realm } => realm == &anchor.parent_realm,
                RevocationTarget::RealmIdentity {
                    realm,
                    identity_ref,
                    fingerprint,
                } => {
                    realm == &anchor.parent_realm
                        && identity_ref == &anchor.parent_identity_ref
                        && fingerprint == &anchor.parent_fingerprint
                }
                RevocationTarget::RealmKey {
                    realm,
                    role: RealmKeyRole::ParentTrustAnchor | RealmKeyRole::RealmIdentity,
                    fingerprint,
                } => {
                    realm == &anchor.parent_realm
                        && fingerprint.as_str() == anchor.parent_fingerprint.as_str()
                }
                _ => false,
            }
        })
    }

    fn apply_revocation(
        &mut self,
        mut revocation: RevocationRecord,
        affected_workloads: Vec<WorkloadId>,
        parent_pushed: bool,
    ) -> IdentityStoreResult<IdentityStoreChange> {
        if self.revocations.contains_key(&revocation.revocation_id) {
            return Err(IdentityStoreError::DuplicateRevocation);
        }
        if affected_workloads.len() > MAX_TEARDOWN_WORKLOADS {
            return Err(IdentityStoreError::TooManyTeardownDirectives);
        }
        if !matches!(
            revocation.status,
            RevocationStatus::Effective | RevocationStatus::Propagated
        ) {
            revocation.status = if parent_pushed {
                RevocationStatus::Propagated
            } else {
                RevocationStatus::Effective
            };
        }
        if revocation.effective_at_unix_seconds.is_none() {
            revocation.effective_at_unix_seconds = Some(revocation.issued_at_unix_seconds);
        }

        let (affected_realm, teardown_reason) = self.apply_revocation_target(&revocation)?;
        self.revocations
            .insert(revocation.revocation_id.clone(), revocation.clone());

        let mut change = IdentityStoreChange::empty();
        if let Some((realm, reason)) = affected_realm.zip(teardown_reason) {
            change.push_teardown(SessionTeardownDirective {
                revocation_id: revocation.revocation_id.clone(),
                issuer_realm: revocation.issuer_realm.clone(),
                affected_realm: realm,
                reason,
                affected_workloads,
                issued_at_unix_seconds: revocation.issued_at_unix_seconds,
                correlation_id: revocation.correlation_id.clone(),
            })?;
        }
        change.push_audit(IdentityAuditEventMetadata {
            event: IdentityAuditEventKind::RevocationIssued,
            realm: revocation.issuer_realm,
            enrollment_id: None,
            rotation_id: None,
            revocation_id: Some(revocation.revocation_id),
            recovery_id: None,
            enrollment_status: None,
            rotation_status: None,
            revocation_status: Some(revocation.status),
            recovery_status: None,
            correlation_id: revocation.correlation_id,
        })?;
        Ok(change)
    }

    fn apply_revocation_target(
        &mut self,
        revocation: &RevocationRecord,
    ) -> IdentityStoreResult<(Option<RealmPath>, Option<SessionTeardownReason>)> {
        match &revocation.target {
            RevocationTarget::Realm { realm } => {
                let keys = self
                    .controller_generations
                    .keys()
                    .filter(|(candidate, _)| candidate == realm)
                    .cloned()
                    .collect::<Vec<_>>();
                if keys.is_empty() {
                    return Err(IdentityStoreError::UnknownGeneration);
                }
                for key in keys {
                    if let Some(generation) = self.controller_generations.get_mut(&key) {
                        generation.status = ControllerGenerationStatus::Revoked;
                        generation.revoked_by = Some(revocation.revocation_id.clone());
                        generation.realm_identity.status = RealmIdentityStatus::Revoked;
                    }
                }
                self.active_generations.remove(realm);
                Ok((
                    Some(realm.clone()),
                    Some(SessionTeardownReason::RealmIdentityRevoked),
                ))
            }
            RevocationTarget::RealmIdentity {
                realm,
                identity_ref,
                fingerprint,
            } => {
                validate_identity_ref(identity_ref)?;
                let keys = self
                    .controller_generations
                    .iter()
                    .filter(|((candidate, _), generation)| {
                        candidate == realm
                            && generation.realm_identity.identity_ref == *identity_ref
                            && generation.realm_identity.fingerprint == *fingerprint
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                if keys.is_empty() {
                    return Err(IdentityStoreError::UnknownGeneration);
                }
                for key in keys {
                    if let Some(generation) = self.controller_generations.get_mut(&key) {
                        generation.realm_identity.status = RealmIdentityStatus::Revoked;
                        generation.status = ControllerGenerationStatus::Revoked;
                        generation.revoked_by = Some(revocation.revocation_id.clone());
                    }
                }
                self.active_generations.remove(realm);
                Ok((
                    Some(realm.clone()),
                    Some(SessionTeardownReason::RealmIdentityRevoked),
                ))
            }
            RevocationTarget::RealmKey { realm, role, .. } => {
                let reason = match role {
                    RealmKeyRole::RealmIdentity
                    | RealmKeyRole::ParentTrustAnchor
                    | RealmKeyRole::ChildIdentity => SessionTeardownReason::RealmIdentityRevoked,
                    RealmKeyRole::ControllerGeneration => {
                        SessionTeardownReason::ControllerGenerationRevoked
                    }
                };
                Ok((Some(realm.clone()), Some(reason)))
            }
            RevocationTarget::ControllerGeneration {
                realm,
                controller_generation,
            } => {
                let key = (realm.clone(), controller_generation.clone());
                let generation = self
                    .controller_generations
                    .get_mut(&key)
                    .ok_or(IdentityStoreError::UnknownGeneration)?;
                generation.status = ControllerGenerationStatus::Revoked;
                generation.revoked_by = Some(revocation.revocation_id.clone());
                generation.not_after_unix_seconds = Some(
                    revocation
                        .effective_at_unix_seconds
                        .unwrap_or(revocation.issued_at_unix_seconds),
                );
                if self.active_generations.get(realm) == Some(controller_generation) {
                    self.active_generations.remove(realm);
                }
                Ok((
                    Some(realm.clone()),
                    Some(SessionTeardownReason::ControllerGenerationRevoked),
                ))
            }
            RevocationTarget::ControllerCredential {
                realm,
                credential_ref,
                fingerprint,
            } => {
                validate_credential_ref(credential_ref)?;
                let key = self
                    .controller_generations
                    .iter()
                    .find(|((candidate, _), generation)| {
                        candidate == realm
                            && generation.credential_ref == *credential_ref
                            && generation.credential_fingerprint == *fingerprint
                    })
                    .map(|(key, _)| key.clone())
                    .ok_or(IdentityStoreError::UnknownGeneration)?;
                let generation = self
                    .controller_generations
                    .get_mut(&key)
                    .ok_or(IdentityStoreError::UnknownGeneration)?;
                generation.status = ControllerGenerationStatus::Revoked;
                generation.revoked_by = Some(revocation.revocation_id.clone());
                if self.active_generations.get(realm) == Some(&generation.generation_id) {
                    self.active_generations.remove(realm);
                }
                Ok((
                    Some(realm.clone()),
                    Some(SessionTeardownReason::ControllerGenerationRevoked),
                ))
            }
            RevocationTarget::Enrollment { enrollment_id } => {
                let enrollment = self
                    .enrollments
                    .get_mut(enrollment_id)
                    .ok_or(IdentityStoreError::UnknownEnrollment)?;
                enrollment.status = EnrollmentStatus::Revoked;
                enrollment.reason = Some(EnrollmentReason::ParentPolicyDenied);
                Ok((
                    Some(enrollment.child_realm.clone()),
                    Some(SessionTeardownReason::RealmIdentityRevoked),
                ))
            }
            RevocationTarget::PolicyGrant { realm, .. } => Ok((
                Some(realm.clone()),
                Some(SessionTeardownReason::PolicyGrantRevoked),
            )),
            RevocationTarget::CapabilityGrant { realm, .. } => Ok((
                Some(realm.clone()),
                Some(SessionTeardownReason::StreamCapabilityRevoked),
            )),
        }
    }
}

fn validate_generation_refs(generation: &ControllerGenerationMetadata) -> IdentityStoreResult<()> {
    validate_identity_ref(&generation.realm_identity.identity_ref)?;
    validate_credential_ref(&generation.credential_ref)?;
    Ok(())
}

fn validate_rotation_refs(plan: &KeyRotationPlan) -> IdentityStoreResult<()> {
    match &plan.subject {
        KeyRotationSubject::RealmIdentity {
            current_identity_ref,
            ..
        } => validate_identity_ref(current_identity_ref)?,
        KeyRotationSubject::ControllerGeneration {
            current_credential_ref,
            ..
        } => validate_credential_ref(current_credential_ref)?,
    }
    if let Some(identity_ref) = &plan.replacement_identity_ref {
        validate_identity_ref(identity_ref)?;
    }
    if let Some(credential_ref) = &plan.replacement_credential_ref {
        validate_credential_ref(credential_ref)?;
    }
    Ok(())
}

fn validate_identity_ref(identity_ref: &RealmIdentityRef) -> IdentityStoreResult<()> {
    validate_opaque_ref(identity_ref.as_str())
}

fn validate_credential_ref(
    credential_ref: &ControllerGenerationCredentialRef,
) -> IdentityStoreResult<()> {
    validate_opaque_ref(credential_ref.as_str())
}

fn validate_opaque_ref(raw: &str) -> IdentityStoreResult<()> {
    let lower = raw.to_ascii_lowercase();
    let compact = lower.replace(['-', '_', '.'], "");
    let key_markers = [
        "begin",
        "publickey",
        "privatekey",
        "ssh-rsa",
        "sshrsa",
        "ssh-ed25519",
        "sshed25519",
        "ecdsa",
        "sha256",
    ];
    if key_markers
        .iter()
        .any(|marker| lower.contains(marker) || compact.contains(marker))
    {
        return Err(IdentityStoreError::KeyMaterialRef);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrollment::{
        KeyFingerprint, KeyRotationReason, KeyRotationSubjectKind, ParentTrustAnchor,
        RealmIdentityFingerprint, RealmIdentityMetadata, RevocationListStatus, RevocationReason,
    };
    use crate::ids::{CorrelationId, RealmId};

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(
            labels
                .iter()
                .map(|label| RealmId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn work() -> RealmPath {
        realm(&["work"])
    }

    fn child() -> RealmPath {
        realm(&["payments", "work"])
    }

    fn fp(ch: char) -> KeyFingerprint {
        KeyFingerprint::parse(format!("sha256:{}", ch.to_string().repeat(64))).unwrap()
    }

    fn id_fp(ch: char) -> RealmIdentityFingerprint {
        RealmIdentityFingerprint::parse(format!("sha256:{}", ch.to_string().repeat(64))).unwrap()
    }

    fn identity(realm: RealmPath, id: &str, ch: char) -> RealmIdentityMetadata {
        RealmIdentityMetadata {
            realm,
            identity_ref: RealmIdentityRef::parse(id).unwrap(),
            fingerprint: id_fp(ch),
            status: RealmIdentityStatus::Active,
            created_at_unix_seconds: 1,
            not_after_unix_seconds: None,
        }
    }

    fn generation(
        realm: RealmPath,
        generation: &str,
        credential: &str,
        ch: char,
    ) -> ControllerGenerationMetadata {
        ControllerGenerationMetadata {
            realm: realm.clone(),
            generation_id: ControllerGenerationId::parse(generation).unwrap(),
            realm_identity: identity(realm, "idref-work", 'b'),
            credential_ref: ControllerGenerationCredentialRef::parse(credential).unwrap(),
            credential_fingerprint: fp(ch),
            status: ControllerGenerationStatus::Active,
            issued_at_unix_seconds: 10,
            not_before_unix_seconds: 10,
            not_after_unix_seconds: None,
            revoked_by: None,
        }
    }

    fn child_generation(
        generation_id: &str,
        credential: &str,
        ch: char,
    ) -> ControllerGenerationMetadata {
        let child = child();
        ControllerGenerationMetadata {
            realm: child.clone(),
            generation_id: ControllerGenerationId::parse(generation_id).unwrap(),
            realm_identity: identity(child, "idref-child", 'c'),
            credential_ref: ControllerGenerationCredentialRef::parse(credential).unwrap(),
            credential_fingerprint: fp(ch),
            status: ControllerGenerationStatus::Active,
            issued_at_unix_seconds: 20,
            not_before_unix_seconds: 20,
            not_after_unix_seconds: None,
            revoked_by: None,
        }
    }

    fn enrollment(id: &str) -> EnrollmentRecord {
        EnrollmentRecord {
            enrollment_id: EnrollmentId::parse(id).unwrap(),
            parent_realm: work(),
            child_realm: child(),
            controller_generation: child_generation("gen-child-1", "cgref-child-1", 'd'),
            parent_trust_anchor: ParentTrustAnchor {
                parent_realm: work(),
                child_realm: child(),
                parent_identity_ref: RealmIdentityRef::parse("idref-work").unwrap(),
                parent_fingerprint: id_fp('b'),
                accepted_by_generation: ControllerGenerationId::parse("gen-child-1").unwrap(),
                pinned_at_unix_seconds: 21,
            },
            child_key_pin: ChildKeyPin {
                parent_realm: work(),
                child_realm: child(),
                child_identity_ref: RealmIdentityRef::parse("idref-child").unwrap(),
                child_fingerprint: id_fp('c'),
                accepted_by_generation: ControllerGenerationId::parse("gen-child-1").unwrap(),
                enrollment_id: EnrollmentId::parse(id).unwrap(),
                pinned_at_unix_seconds: 21,
            },
            status: EnrollmentStatus::Accepted,
            reason: None,
            bootstrap_method: ProtocolToken::parse("host-local").unwrap(),
            created_at_unix_seconds: 20,
            updated_at_unix_seconds: 21,
            correlation_id: Some(CorrelationId::parse("corr-1").unwrap()),
        }
    }

    fn issuer_store() -> RealmIdentityStore {
        let mut store = RealmIdentityStore::new();
        store
            .issue_controller_generation(generation(work(), "gen-work-1", "cgref-work-1", 'a'))
            .unwrap();
        store
    }

    #[test]
    fn enrollment_records_and_pins_are_indexed_and_duplicate_fail_closed() {
        let mut store = issuer_store();
        let change = store.enroll(enrollment("enroll-1")).unwrap();
        assert_eq!(
            change.audit_events()[0].event,
            IdentityAuditEventKind::EnrollmentAccepted
        );
        assert!(
            store
                .enrollment(&EnrollmentId::parse("enroll-1").unwrap())
                .is_some()
        );
        assert!(store.parent_trust_anchor(&work(), &child()).is_some());
        assert!(store.child_key_pin(&work(), &child()).is_some());
        assert_eq!(
            store.enroll(enrollment("enroll-1")).unwrap_err(),
            IdentityStoreError::DuplicateEnrollment
        );
    }

    #[test]
    fn rotation_preserves_realm_identity_and_rejects_stale_generation() {
        let mut store = issuer_store();
        let current = store
            .controller_generation(
                &work(),
                &ControllerGenerationId::parse("gen-work-1").unwrap(),
            )
            .unwrap()
            .clone();
        let mut replacement = generation(work(), "gen-work-2", "cgref-work-2", 'e');
        replacement.realm_identity = current.realm_identity.clone();
        replacement.status = ControllerGenerationStatus::Pending;
        let plan = KeyRotationPlan {
            rotation_id: KeyRotationId::parse("rotate-1").unwrap(),
            realm: work(),
            subject: KeyRotationSubject::ControllerGeneration {
                realm: work(),
                current_generation: current.generation_id.clone(),
                current_credential_ref: current.credential_ref.clone(),
                current_fingerprint: current.credential_fingerprint.clone(),
            },
            reason: KeyRotationReason::Routine,
            status: KeyRotationStatus::Planned,
            replacement_identity_ref: None,
            replacement_credential_ref: None,
            replacement_fingerprint: None,
            planned_at_unix_seconds: 30,
            activate_after_unix_seconds: None,
            correlation_id: Some(CorrelationId::parse("corr-rotate").unwrap()),
        };
        let (event, change) = store
            .rotate_controller_generation(plan.clone(), replacement, 31)
            .unwrap();
        assert_eq!(
            event.subject_kind,
            KeyRotationSubjectKind::ControllerGeneration
        );
        assert_eq!(event.event, KeyRotationEventKind::Activated);
        let active = store
            .controller_generation(&work(), store.active_generation(&work()).unwrap())
            .unwrap();
        assert_eq!(active.realm_identity, current.realm_identity);
        assert_eq!(
            change.audit_events()[0].rotation_status,
            Some(KeyRotationStatus::Active)
        );

        let mut stale_replacement = generation(work(), "gen-work-3", "cgref-work-3", 'f');
        stale_replacement.realm_identity = active.realm_identity.clone();
        assert_eq!(
            store
                .rotate_controller_generation(plan, stale_replacement, 32)
                .unwrap_err(),
            IdentityStoreError::StaleGeneration
        );
    }

    #[test]
    fn revocation_forces_teardown_metadata_and_unknown_generation_fails_closed() {
        let mut store = issuer_store();
        let revocation = RevocationRecord {
            revocation_id: RevocationId::parse("rev-1").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: work(),
                controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            },
            status: RevocationStatus::Pending,
            reason: RevocationReason::ControllerCompromised,
            issued_at_unix_seconds: 40,
            effective_at_unix_seconds: None,
            correlation_id: Some(CorrelationId::parse("corr-revoke").unwrap()),
        };
        let change = store
            .revoke_generation(revocation, vec![WorkloadId::parse("vm-a").unwrap()])
            .unwrap();
        assert_eq!(change.teardown_directives().len(), 1);
        assert_eq!(
            change.teardown_directives()[0].reason,
            SessionTeardownReason::ControllerGenerationRevoked
        );
        assert!(store.active_generation(&work()).is_none());

        let mut unknown = issuer_store();
        let bad = RevocationRecord {
            revocation_id: RevocationId::parse("rev-unknown").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: work(),
                controller_generation: ControllerGenerationId::parse("missing-gen").unwrap(),
            },
            status: RevocationStatus::Effective,
            reason: RevocationReason::ControllerCompromised,
            issued_at_unix_seconds: 41,
            effective_at_unix_seconds: Some(41),
            correlation_id: None,
        };
        assert_eq!(
            unknown.revoke_generation(bad, vec![]).unwrap_err(),
            IdentityStoreError::UnknownGeneration
        );
    }

    #[test]
    fn realm_revocation_revokes_identity_and_forces_teardown() {
        let mut store = issuer_store();
        let revocation = RevocationRecord {
            revocation_id: RevocationId::parse("rev-realm").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            target: RevocationTarget::Realm { realm: work() },
            status: RevocationStatus::Pending,
            reason: RevocationReason::KeyCompromised,
            issued_at_unix_seconds: 45,
            effective_at_unix_seconds: None,
            correlation_id: None,
        };
        let change = store
            .revoke_realm(revocation, vec![WorkloadId::parse("vm-a").unwrap()])
            .unwrap();
        assert_eq!(
            change.teardown_directives()[0].reason,
            SessionTeardownReason::RealmIdentityRevoked
        );
        let revoked = store
            .controller_generation(
                &work(),
                &ControllerGenerationId::parse("gen-work-1").unwrap(),
            )
            .unwrap();
        assert_eq!(revoked.status, ControllerGenerationStatus::Revoked);
        assert_eq!(revoked.realm_identity.status, RealmIdentityStatus::Revoked);
    }

    #[test]
    fn parent_pushed_revocation_list_merge_revokes_child_generation() {
        let mut store = issuer_store();
        store.enroll(enrollment("enroll-1")).unwrap();
        let record = RevocationRecord {
            revocation_id: RevocationId::parse("rev-child").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("parent-gen-opaque")
                .unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: child(),
                controller_generation: ControllerGenerationId::parse("gen-child-1").unwrap(),
            },
            status: RevocationStatus::Effective,
            reason: RevocationReason::ParentPolicyRevoked,
            issued_at_unix_seconds: 50,
            effective_at_unix_seconds: Some(50),
            correlation_id: None,
        };
        let list = RevocationList {
            list_id: RevocationListId::parse("rvlist-1").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("parent-gen-opaque")
                .unwrap(),
            status: RevocationListStatus::Published,
            records: vec![record],
            propagated_to: vec![child()],
            generated_at_unix_seconds: 51,
            supersedes: None,
            correlation_id: None,
        };
        let change = store
            .merge_revocation_list(list, vec![WorkloadId::parse("vm-a").unwrap()])
            .unwrap();
        assert_eq!(change.teardown_directives()[0].affected_realm, child());
        assert_eq!(
            store
                .controller_generation(
                    &child(),
                    &ControllerGenerationId::parse("gen-child-1").unwrap()
                )
                .unwrap()
                .status,
            ControllerGenerationStatus::Revoked
        );
    }

    #[test]
    fn revocation_list_merge_skips_existing_records_and_rejects_duplicates() {
        let mut store = issuer_store();
        store.enroll(enrollment("enroll-1")).unwrap();
        let record = RevocationRecord {
            revocation_id: RevocationId::parse("rev-child").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: child(),
                controller_generation: ControllerGenerationId::parse("gen-child-1").unwrap(),
            },
            status: RevocationStatus::Effective,
            reason: RevocationReason::ParentPolicyRevoked,
            issued_at_unix_seconds: 52,
            effective_at_unix_seconds: Some(52),
            correlation_id: None,
        };
        store
            .revoke(record.clone(), vec![WorkloadId::parse("vm-a").unwrap()])
            .unwrap();
        let replay = RevocationList {
            list_id: RevocationListId::parse("rvlist-replay").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            status: RevocationListStatus::Published,
            records: vec![record.clone()],
            propagated_to: vec![child()],
            generated_at_unix_seconds: 53,
            supersedes: None,
            correlation_id: None,
        };
        let change = store.merge_revocation_list(replay, vec![]).unwrap();
        assert!(
            change.teardown_directives().is_empty(),
            "existing revocation records are metadata replays, not duplicate teardown"
        );
        assert_eq!(
            change.audit_events()[0].event,
            IdentityAuditEventKind::RevocationPropagated
        );

        let duplicate = RevocationList {
            list_id: RevocationListId::parse("rvlist-duplicate").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            status: RevocationListStatus::Published,
            records: vec![record.clone(), record],
            propagated_to: vec![child()],
            generated_at_unix_seconds: 54,
            supersedes: None,
            correlation_id: None,
        };
        assert_eq!(
            store.merge_revocation_list(duplicate, vec![]).unwrap_err(),
            IdentityStoreError::DuplicateRevocation
        );
    }

    #[test]
    fn revoked_parent_trust_blocks_new_enrollment() {
        let mut store = issuer_store();
        store.enroll(enrollment("enroll-1")).unwrap();
        let record = RevocationRecord {
            revocation_id: RevocationId::parse("rev-parent").unwrap(),
            issuer_realm: work(),
            issuer_controller_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            target: RevocationTarget::RealmIdentity {
                realm: work(),
                identity_ref: RealmIdentityRef::parse("idref-work").unwrap(),
                fingerprint: id_fp('b'),
            },
            status: RevocationStatus::Effective,
            reason: RevocationReason::KeyCompromised,
            issued_at_unix_seconds: 60,
            effective_at_unix_seconds: Some(60),
            correlation_id: None,
        };
        store.revoke(record, vec![]).unwrap();
        let mut replacement = enrollment("enroll-2");
        replacement.child_realm = realm(&["billing", "work"]);
        replacement.controller_generation.realm = replacement.child_realm.clone();
        replacement.controller_generation.realm_identity.realm = replacement.child_realm.clone();
        replacement.parent_trust_anchor.child_realm = replacement.child_realm.clone();
        replacement.child_key_pin.child_realm = replacement.child_realm.clone();
        replacement.child_key_pin.enrollment_id = EnrollmentId::parse("enroll-2").unwrap();
        assert_eq!(
            store.enroll(replacement).unwrap_err(),
            IdentityStoreError::RevokedParentTrust
        );
    }

    #[test]
    fn recovery_flow_states_activate_replacement() {
        let mut store = issuer_store();
        let recovery = RecoveryProcedure {
            recovery_id: RecoveryProcedureId::parse("recover-1").unwrap(),
            parent_realm: work(),
            child_realm: work(),
            reason: crate::enrollment::RecoveryReason::LostControllerKey,
            status: RecoveryStatus::Requested,
            affected_generation: ControllerGenerationId::parse("gen-work-1").unwrap(),
            replacement_generation: None,
            evidence_refs: vec![ProtocolToken::parse("ticket-1").unwrap()],
            opened_at_unix_seconds: 70,
            closed_at_unix_seconds: None,
            correlation_id: None,
        };
        store.open_recovery(recovery).unwrap();
        assert_eq!(
            store
                .controller_generation(
                    &work(),
                    &ControllerGenerationId::parse("gen-work-1").unwrap()
                )
                .unwrap()
                .status,
            ControllerGenerationStatus::Recovering
        );
        store
            .approve_recovery(&RecoveryProcedureId::parse("recover-1").unwrap())
            .unwrap();
        let isolation = store
            .isolate_recovery(
                &RecoveryProcedureId::parse("recover-1").unwrap(),
                71,
                vec![WorkloadId::parse("vm-a").unwrap()],
            )
            .unwrap();
        assert_eq!(
            isolation.teardown_directives()[0].reason,
            SessionTeardownReason::RecoveryIsolation
        );
        let mut replacement = generation(work(), "gen-work-2", "cgref-work-2", 'e');
        replacement.realm_identity = store
            .controller_generation(
                &work(),
                &ControllerGenerationId::parse("gen-work-1").unwrap(),
            )
            .unwrap()
            .realm_identity
            .clone();
        store
            .reissue_recovery(
                &RecoveryProcedureId::parse("recover-1").unwrap(),
                replacement,
            )
            .unwrap();
        let done = store
            .complete_recovery(&RecoveryProcedureId::parse("recover-1").unwrap(), 72)
            .unwrap();
        assert_eq!(
            done.audit_events()[0].event,
            IdentityAuditEventKind::RecoveryCompleted
        );
        assert_eq!(
            store.active_generation(&work()).unwrap(),
            &ControllerGenerationId::parse("gen-work-2").unwrap()
        );
    }

    #[test]
    fn policy_and_capability_revocations_compute_specific_teardown_reasons() {
        let mut store = issuer_store();
        let policy = store
            .revoke_policy_grant(
                RevocationId::parse("rev-policy").unwrap(),
                work(),
                ControllerGenerationId::parse("gen-work-1").unwrap(),
                work(),
                ProtocolToken::parse("policy-a").unwrap(),
                80,
                vec![],
            )
            .unwrap();
        assert_eq!(
            policy.teardown_directives()[0].reason,
            SessionTeardownReason::PolicyGrantRevoked
        );

        let capability = store
            .revoke_capability(
                RevocationId::parse("rev-capability").unwrap(),
                work(),
                ControllerGenerationId::parse("gen-work-1").unwrap(),
                work(),
                Capability::Exec,
                81,
                vec![],
            )
            .unwrap();
        assert_eq!(
            capability.teardown_directives()[0].reason,
            SessionTeardownReason::StreamCapabilityRevoked
        );
    }

    #[test]
    fn key_material_shaped_refs_fail_closed_and_debug_is_redacted() {
        let mut store = RealmIdentityStore::new();
        let material_ref = ControllerGenerationMetadata {
            realm: work(),
            generation_id: ControllerGenerationId::parse("gen-work-1").unwrap(),
            realm_identity: RealmIdentityMetadata {
                realm: work(),
                identity_ref: RealmIdentityRef::parse("ssh-ed25519-key").unwrap(),
                fingerprint: id_fp('b'),
                status: RealmIdentityStatus::Active,
                created_at_unix_seconds: 1,
                not_after_unix_seconds: None,
            },
            credential_ref: ControllerGenerationCredentialRef::parse("cgref-work-1").unwrap(),
            credential_fingerprint: fp('a'),
            status: ControllerGenerationStatus::Active,
            issued_at_unix_seconds: 10,
            not_before_unix_seconds: 10,
            not_after_unix_seconds: None,
            revoked_by: None,
        };
        assert_eq!(
            store.issue_controller_generation(material_ref).unwrap_err(),
            IdentityStoreError::KeyMaterialRef
        );

        store
            .issue_controller_generation(generation(work(), "gen-work-1", "cgref-work-1", 'a'))
            .unwrap();
        let debug = format!("{store:?}");
        assert!(!debug.contains("cgref-work-1"));
        assert!(!debug.contains("idref-work"));
        assert!(!debug.contains(id_fp('b').as_str()));
        assert!(debug.contains("ControllerGenerationCredentialRef(<12 bytes>)"));
    }
}
