//! Pure resumable operation state machine.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts::v3::{CanonicalJsonError, canonical_digest, canonical_json_bytes};
use serde::Serialize;

use crate::{
    consent::{
        Consent, ConsentBinding, ConsentError, FinalizationBinding, FinalizationConsent,
        RecoveryAttestation,
    },
    finalize::{FinalizationError, require_cutover_finalization},
    hold::{HoldError, HoldState},
    inventory::{HostInventory, InventoryError},
    journal::{Journal, JournalBinding, JournalError},
    model::{
        ArtifactId, AuditRecordId, CandidateId, CompletionEvidence, CutoverPhase, Digest, EffectId,
        EffectKind, EffectOutcome, FailureCode, HoldReason, HostLockContract, LockAcquire,
        OperationId, OperationKind, OperationState, OperatorId, ReplayClass, ReplayDecision,
        ReplayObservation, RevisionPlanId, StepId, TerminalOutcomeKind,
    },
    preview::{CutoverPreview, PreviewError, PreviewInventory},
    reset::{ResetError, ResetInventory},
    rollback::{RollbackError, RollbackResult, plan_native_rollback},
    verify::{VerificationError, VerificationInput, VerificationReport, verify_cutover},
};

/// Domain separator for operation request digests.
pub const REQUEST_DOMAIN: &str = "d2b:cutover:request:v1";

/// Inventory carried by one operation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum OperationInventory {
    /// Host-wide all-Zone cutover inventory.
    Host(HostInventory),
    /// Scoped reset inventory.
    Reset(ResetInventory),
}

/// Immutable operation request binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationRequest {
    operation_id: OperationId,
    operation_kind: OperationKind,
    candidate_id: CandidateId,
    revision_plan_id: RevisionPlanId,
    operator_id: OperatorId,
    preview_digest: Digest,
    recovery_digest: Option<Digest>,
    inventory: OperationInventory,
    inventory_digest: Digest,
    request_digest: Digest,
}

impl OperationRequest {
    /// Construct a request and derive its digest from the exact inventory.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: OperationId,
        operation_kind: OperationKind,
        candidate_id: CandidateId,
        revision_plan_id: RevisionPlanId,
        operator_id: OperatorId,
        preview_digest: Digest,
        recovery_digest: Option<Digest>,
        inventory: OperationInventory,
    ) -> Result<Self, OperationError> {
        match (operation_kind, &inventory) {
            (OperationKind::Cutover, OperationInventory::Host(_))
            | (OperationKind::ScopedReset(_), OperationInventory::Reset(_)) => {}
            _ => return Err(OperationError::InventoryKindMismatch),
        }
        let inventory_digest = match &inventory {
            OperationInventory::Host(inventory) => inventory.digest()?,
            OperationInventory::Reset(inventory) => inventory.digest()?,
        };
        let payload = OperationRequestPayload {
            operation_id: operation_id.clone(),
            operation_kind,
            candidate_id: candidate_id.clone(),
            revision_plan_id: revision_plan_id.clone(),
            operator_id: operator_id.clone(),
            preview_digest: preview_digest.clone(),
            recovery_digest: recovery_digest.clone(),
            inventory_digest: inventory_digest.clone(),
        };
        let bytes = canonical_json_bytes(&payload).map_err(OperationError::CanonicalJson)?;
        let request_digest = Digest::parse(canonical_digest(REQUEST_DOMAIN, &bytes))
            .map_err(|_| OperationError::Digest)?;
        Ok(Self {
            operation_id,
            operation_kind,
            candidate_id,
            revision_plan_id,
            operator_id,
            preview_digest,
            recovery_digest,
            inventory,
            inventory_digest,
            request_digest,
        })
    }

    /// Construct a host-wide cutover request.
    #[allow(clippy::too_many_arguments)]
    pub fn new_cutover(
        operation_id: OperationId,
        candidate_id: CandidateId,
        revision_plan_id: RevisionPlanId,
        operator_id: OperatorId,
        preview_digest: Digest,
        recovery_digest: Digest,
        inventory: HostInventory,
    ) -> Result<Self, OperationError> {
        Self::new(
            operation_id,
            OperationKind::Cutover,
            candidate_id,
            revision_plan_id,
            operator_id,
            preview_digest,
            Some(recovery_digest),
            OperationInventory::Host(inventory),
        )
    }

    /// Construct a scoped reset request.
    #[allow(clippy::too_many_arguments)]
    pub fn new_reset(
        operation_id: OperationId,
        scope: crate::model::ResetScope,
        candidate_id: CandidateId,
        revision_plan_id: RevisionPlanId,
        operator_id: OperatorId,
        preview_digest: Digest,
        inventory: ResetInventory,
    ) -> Result<Self, OperationError> {
        if inventory.scope() != scope {
            return Err(OperationError::Reset(ResetError::ScopeMismatch));
        }
        Self::new(
            operation_id,
            OperationKind::ScopedReset(scope),
            candidate_id,
            revision_plan_id,
            operator_id,
            preview_digest,
            None,
            OperationInventory::Reset(inventory),
        )
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Return the operation kind.
    pub const fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    /// Borrow the candidate identity.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Borrow the revision-plan identity.
    pub fn revision_plan_id(&self) -> &RevisionPlanId {
        &self.revision_plan_id
    }

    /// Borrow the bound operator.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Borrow the preview digest.
    pub fn preview_digest(&self) -> &Digest {
        &self.preview_digest
    }

    /// Borrow the optional recovery digest.
    pub fn recovery_digest(&self) -> Option<&Digest> {
        self.recovery_digest.as_ref()
    }

    /// Borrow the operation inventory.
    pub fn inventory(&self) -> &OperationInventory {
        &self.inventory
    }

    /// Borrow the normalized inventory digest.
    pub fn inventory_digest(&self) -> &Digest {
        &self.inventory_digest
    }

    /// Borrow the request digest.
    pub fn request_digest(&self) -> &Digest {
        &self.request_digest
    }

    /// Build the exact apply-consent binding.
    pub fn consent_binding(&self) -> ConsentBinding {
        ConsentBinding::new(
            self.operation_id.clone(),
            self.operation_kind,
            self.candidate_id.clone(),
            self.preview_digest.clone(),
            self.recovery_digest.clone(),
            self.operator_id.clone(),
        )
    }

    /// Build the distinct phase-10 binding.
    pub fn finalization_binding(&self) -> FinalizationBinding {
        FinalizationBinding::new(
            self.operation_id.clone(),
            self.operation_kind,
            self.candidate_id.clone(),
            self.preview_digest.clone(),
            self.operator_id.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationRequestPayload {
    operation_id: OperationId,
    operation_kind: OperationKind,
    candidate_id: CandidateId,
    revision_plan_id: RevisionPlanId,
    operator_id: OperatorId,
    preview_digest: Digest,
    recovery_digest: Option<Digest>,
    inventory_digest: Digest,
}

/// Validation context sampled at apply or resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyContext {
    now_ms: u64,
    inventory_digest: Digest,
    candidate_current: bool,
    markers_valid: bool,
    ownership_valid: bool,
    recovery: Option<RecoveryAttestation>,
    host_digest: Option<Digest>,
}

impl ApplyContext {
    /// Construct a cutover validation context.
    pub fn cutover(
        now_ms: u64,
        inventory_digest: Digest,
        candidate_current: bool,
        markers_valid: bool,
        ownership_valid: bool,
        recovery: RecoveryAttestation,
        host_digest: Digest,
    ) -> Self {
        Self {
            now_ms,
            inventory_digest,
            candidate_current,
            markers_valid,
            ownership_valid,
            recovery: Some(recovery),
            host_digest: Some(host_digest),
        }
    }

    /// Construct a reset validation context.
    pub fn reset(
        now_ms: u64,
        inventory_digest: Digest,
        candidate_current: bool,
        markers_valid: bool,
        ownership_valid: bool,
    ) -> Self {
        Self {
            now_ms,
            inventory_digest,
            candidate_current,
            markers_valid,
            ownership_valid,
            recovery: None,
            host_digest: None,
        }
    }
}

/// Evidence required to complete a read-only phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyEvidence {
    /// Whether the phase predicates held at the sampled boundary.
    pub predicates_hold: bool,
    /// Durable audit publication for the phase.
    pub audit: crate::model::AuditEvidence,
}

/// One effect request handed to a typed external adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRequest {
    effect_id: EffectId,
    step_id: StepId,
    kind: EffectKind,
    replay_class: ReplayClass,
    identity_bearing: bool,
    journaled_identity: Option<ArtifactId>,
    destination: Option<ArtifactId>,
    advance_to: Option<CutoverPhase>,
}

impl EffectRequest {
    /// Construct an effect request with a closed replay class.
    pub fn new(
        effect_id: EffectId,
        step_id: StepId,
        kind: EffectKind,
        replay_class: ReplayClass,
        advance_to: Option<CutoverPhase>,
    ) -> Self {
        Self {
            effect_id,
            step_id,
            kind,
            replay_class,
            identity_bearing: false,
            journaled_identity: None,
            destination: None,
            advance_to,
        }
    }

    /// Mark the effect as identity-bearing and bind an optional known identity.
    pub fn with_identity(
        mut self,
        journaled_identity: Option<ArtifactId>,
        destination: Option<ArtifactId>,
    ) -> Self {
        self.identity_bearing = true;
        self.journaled_identity = journaled_identity;
        self.destination = destination;
        self
    }

    /// Borrow the effect identity.
    pub fn effect_id(&self) -> &EffectId {
        &self.effect_id
    }

    /// Borrow the step identity.
    pub fn step_id(&self) -> &StepId {
        &self.step_id
    }

    /// Return the effect kind.
    pub const fn kind(&self) -> EffectKind {
        self.kind
    }

    /// Return the replay class.
    pub const fn replay_class(&self) -> ReplayClass {
        self.replay_class
    }

    /// Return whether the effect creates or adopts an identity-bearing object.
    pub const fn identity_bearing(&self) -> bool {
        self.identity_bearing
    }

    /// Borrow the journaled identity, if already known.
    pub fn journaled_identity(&self) -> Option<&ArtifactId> {
        self.journaled_identity.as_ref()
    }

    /// Borrow the staged destination identity, if any.
    pub fn destination(&self) -> Option<&ArtifactId> {
        self.destination.as_ref()
    }

    /// Return the next phase after durable completion.
    pub const fn advance_to(&self) -> Option<CutoverPhase> {
        self.advance_to
    }
}

/// The resumable pure engine.
#[derive(Debug, Clone)]
pub struct Operation {
    request: OperationRequest,
    phase: CutoverPhase,
    state: OperationState,
    journal: Journal,
    hold: HoldState,
    current_effect: Option<EffectRequest>,
    staged_destinations: BTreeSet<ArtifactId>,
    terminal: Option<TerminalOutcomeKind>,
    rollback: Option<RollbackResult>,
    lock_held: bool,
}

/// Alias for the host-wide cutover engine.
pub type CutoverEngine = Operation;
/// Alias for the scoped reset engine.
pub type ResetEngine = Operation;

impl Operation {
    /// Create a planned operation after preview/request identity checks.
    pub fn new(
        request: OperationRequest,
        preview: &CutoverPreview,
    ) -> Result<Self, OperationError> {
        let preview_digest = preview.digest()?;
        if preview.operation_id() != request.operation_id()
            || preview.operation_kind() != request.operation_kind()
            || preview.candidate_id() != request.candidate_id()
            || preview.revision_plan_id() != request.revision_plan_id()
            || &preview_digest != request.preview_digest()
        {
            return Err(OperationError::PreviewMismatch);
        }
        match (&request.inventory, preview.inventory()) {
            (
                OperationInventory::Host(request_inventory),
                PreviewInventory::Host(preview_inventory),
            ) if request_inventory != preview_inventory => {
                return Err(OperationError::PreviewMismatch);
            }
            (
                OperationInventory::Reset(request_inventory),
                PreviewInventory::Reset(preview_inventory),
            ) if request_inventory != preview_inventory => {
                return Err(OperationError::PreviewMismatch);
            }
            (OperationInventory::Host(_), PreviewInventory::Reset(_))
            | (OperationInventory::Reset(_), PreviewInventory::Host(_)) => {
                return Err(OperationError::PreviewMismatch);
            }
            _ => {}
        }
        let binding = JournalBinding::new(
            request.operation_id.clone(),
            request.revision_plan_id.clone(),
            request.request_digest.clone(),
        );
        Ok(Self {
            request,
            phase: CutoverPhase::Preflight,
            state: OperationState::Planned,
            journal: Journal::new(binding),
            hold: HoldState::Clear,
            current_effect: None,
            staged_destinations: BTreeSet::new(),
            terminal: None,
            rollback: None,
            lock_held: false,
        })
    }

    /// Reopen an operation from a verified journal without replaying mutation.
    pub fn reopen(
        request: OperationRequest,
        preview: &CutoverPreview,
        journal: Journal,
    ) -> Result<Self, OperationError> {
        let mut operation = Self::new(request, preview)?;
        if journal.binding() != operation.journal.binding() {
            return Err(OperationError::Journal(JournalError::RequestMismatch));
        }
        for record in journal.records() {
            match record.kind() {
                crate::journal::JournalRecordKind::ConsentConsumed => {
                    operation.phase = record.phase();
                    operation.state = OperationState::Applying(operation.phase);
                }
                crate::journal::JournalRecordKind::PhaseCompleted => {
                    operation.phase = record
                        .phase()
                        .next()
                        .ok_or(OperationError::InvalidTransition)?;
                    operation.state = OperationState::Applying(operation.phase);
                }
                crate::journal::JournalRecordKind::Started => {
                    let (Some(effect_id), Some(step_id), Some(effect_kind), Some(replay_class)) = (
                        record.effect_id(),
                        record.step_id(),
                        record.effect_kind(),
                        record.replay_class(),
                    ) else {
                        return Err(OperationError::Journal(JournalError::Malformed));
                    };
                    operation.phase = record.phase();
                    let mut effect = EffectRequest::new(
                        effect_id.clone(),
                        step_id.clone(),
                        effect_kind,
                        replay_class,
                        record.next_phase(),
                    );
                    if let Some(identity) = record.identity() {
                        effect = effect.with_identity(Some(identity.clone()), None);
                    }
                    operation.current_effect = Some(effect);
                    operation.state = OperationState::Applying(operation.phase);
                }
                crate::journal::JournalRecordKind::Completed => {
                    let Some(record_effect_id) = record.effect_id() else {
                        return Err(OperationError::Journal(JournalError::Malformed));
                    };
                    if operation
                        .current_effect
                        .as_ref()
                        .is_some_and(|effect| effect.effect_id() == record_effect_id)
                    {
                        operation.current_effect = None;
                    }
                    operation.phase = record.next_phase().unwrap_or(record.phase());
                    if let HoldState::Pending {
                        requested_by,
                        reason,
                    } = std::mem::replace(&mut operation.hold, HoldState::Clear)
                    {
                        operation.hold = HoldState::Active {
                            requested_by,
                            reason,
                        };
                        operation.state = OperationState::Held;
                    } else {
                        operation.state = OperationState::Applying(operation.phase);
                    }
                }
                crate::journal::JournalRecordKind::HoldRequested => {
                    let reason =
                        HoldReason::new("journaled-hold").map_err(|_| OperationError::Digest)?;
                    let operator = operation.request.operator_id().clone();
                    if operation.current_effect.is_some() {
                        operation.hold = HoldState::Pending {
                            requested_by: operator,
                            reason,
                        };
                    } else {
                        operation.hold = HoldState::Active {
                            requested_by: operator,
                            reason,
                        };
                    }
                    operation.state = OperationState::Held;
                }
                crate::journal::JournalRecordKind::HoldCleared => {
                    operation.hold = HoldState::Clear;
                    operation.phase = record.phase();
                    operation.state = OperationState::Applying(operation.phase);
                }
                crate::journal::JournalRecordKind::Terminal => {
                    let Some(outcome) = record.terminal_outcome() else {
                        return Err(OperationError::Journal(JournalError::Malformed));
                    };
                    operation.terminal = Some(outcome);
                    operation.state = match outcome {
                        TerminalOutcomeKind::RolledBack => OperationState::RolledBack,
                        TerminalOutcomeKind::RestoreRequired => OperationState::RestoreRequired,
                        TerminalOutcomeKind::Closed => OperationState::Closed,
                        TerminalOutcomeKind::Failed => OperationState::Failed,
                    };
                }
            }
        }
        operation.journal = journal;
        Ok(operation)
    }

    /// Borrow the immutable request.
    pub fn request(&self) -> &OperationRequest {
        &self.request
    }

    /// Return the current phase.
    pub const fn phase(&self) -> CutoverPhase {
        self.phase
    }

    /// Return the current state.
    pub const fn state(&self) -> OperationState {
        self.state
    }

    /// Borrow the journal.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Render the current journal bytes.
    pub fn journal_bytes(&self) -> Result<Vec<u8>, OperationError> {
        Ok(self.journal.to_bytes()?)
    }

    /// Borrow the current hold state.
    pub fn hold(&self) -> &HoldState {
        &self.hold
    }

    /// Borrow the in-flight effect.
    pub fn current_effect(&self) -> Option<&EffectRequest> {
        self.current_effect.as_ref()
    }

    /// Borrow staged destinations that are retained only as non-authoritative data.
    pub fn staged_destinations(&self) -> impl Iterator<Item = &ArtifactId> {
        self.staged_destinations.iter()
    }

    /// Return the write-once terminal outcome, if present.
    pub const fn terminal_outcome(&self) -> Option<TerminalOutcomeKind> {
        self.terminal
    }

    /// Borrow the native rollback result, if rollback completed.
    pub fn rollback_result(&self) -> Option<&RollbackResult> {
        self.rollback.as_ref()
    }

    /// Attempt to acquire the one host-wide linearization lock.
    pub fn acquire_host_lock(&mut self, lock: &mut HostLockContract) -> Result<(), OperationError> {
        match lock.acquire(self.request.operation_id()) {
            LockAcquire::Acquired => {
                self.lock_held = true;
                Ok(())
            }
            LockAcquire::AlreadyOwned => Ok(()),
            LockAcquire::Contended(owner) => Err(OperationError::LockContended(owner)),
        }
    }

    /// Release the model lock after the operation is terminal.
    pub fn release_host_lock(&mut self, lock: &mut HostLockContract) -> Result<(), OperationError> {
        if !self.state.is_terminal() && self.state != OperationState::CutoverSucceeded {
            return Err(OperationError::InvalidTransition);
        }
        self.lock_held = false;
        if lock.release(self.request.operation_id()) {
            Ok(())
        } else {
            Err(OperationError::LockNotHeld)
        }
    }

    /// Consume exact apply consent after revalidating all boundary evidence.
    pub fn begin_apply(
        &mut self,
        consent: &mut Consent,
        context: &ApplyContext,
    ) -> Result<(), OperationError> {
        if self.state != OperationState::Planned || !self.lock_held {
            return Err(OperationError::InvalidTransition);
        }
        self.validate_context(context)?;
        consent
            .consume(&self.request.consent_binding(), context.now_ms)
            .map_err(OperationError::Consent)?;
        self.journal.append_consent(self.phase)?;
        self.state = OperationState::Applying(self.phase);
        Ok(())
    }

    /// Complete a read-only phase only after its audit record is durable.
    pub fn complete_read_only_phase(
        &mut self,
        phase: CutoverPhase,
        evidence: ReadOnlyEvidence,
    ) -> Result<(), OperationError> {
        if self.state != OperationState::Applying(phase)
            || phase.number() > CutoverPhase::Inventory.number()
        {
            return Err(OperationError::InvalidTransition);
        }
        if !evidence.predicates_hold {
            return Err(OperationError::PhaseEvidence);
        }
        let audit_id = durable_audit_id(&evidence.audit)?;
        self.journal.append_phase_completed(phase, audit_id)?;
        let next = phase.next().ok_or(OperationError::InvalidTransition)?;
        self.phase = next;
        self.state = OperationState::Applying(next);
        Ok(())
    }

    /// Start one effect, writing `started` before the adapter may mutate.
    pub fn start_effect(&mut self, effect: EffectRequest) -> Result<(), OperationError> {
        if !matches!(
            self.state,
            OperationState::Applying(_) | OperationState::Finalizing
        ) || self.current_effect.is_some()
            || self.hold.blocks_new_effects()
        {
            return Err(OperationError::InvalidTransition);
        }
        if !self
            .request
            .operation_kind()
            .allowlist()
            .permits(effect.kind())
        {
            return Err(OperationError::EffectNotAllowed(effect.kind()));
        }
        if effect.kind() == EffectKind::CutoverFinalization
            && self.state != OperationState::Finalizing
        {
            return Err(OperationError::FinalizationConsentRequired);
        }
        if self.state == OperationState::Finalizing
            && effect.kind() != EffectKind::CutoverFinalization
        {
            return Err(OperationError::EffectNotAllowed(effect.kind()));
        }
        if effect.kind() == EffectKind::DestroyDurableVolume
            && matches!(
                &self.request.inventory,
                OperationInventory::Reset(inventory)
                    if !inventory.allows_destroy_durable_volumes()
            )
        {
            return Err(OperationError::EffectNotAllowed(effect.kind()));
        }
        if effect.identity_bearing()
            && effect.replay_class() != ReplayClass::ReopenByJournaledIdentity
        {
            return Err(OperationError::IdentityReplayClassRequired);
        }
        if effect.identity_bearing() && effect.journaled_identity().is_none() {
            return Err(OperationError::IdentityMismatch);
        }
        self.journal.append_started(
            self.phase,
            effect.step_id.clone(),
            effect.effect_id.clone(),
            effect.kind,
            effect.advance_to,
            effect.replay_class,
            effect.journaled_identity.clone(),
        )?;
        self.current_effect = Some(effect);
        Ok(())
    }

    /// Complete an effect only when both effect and audit evidence are durable.
    pub fn complete_effect(
        &mut self,
        effect_id: &EffectId,
        evidence: CompletionEvidence,
    ) -> Result<(), OperationError> {
        let Some(effect) = self.current_effect.clone() else {
            return Err(OperationError::NoCurrentEffect);
        };
        if effect.effect_id() != effect_id {
            return Err(OperationError::EffectMismatch);
        }
        match evidence.effect.outcome() {
            EffectOutcome::Succeeded => {}
            EffectOutcome::Failed => return Err(OperationError::EffectFailed),
            EffectOutcome::Ambiguous => return Err(OperationError::DestinationAmbiguous),
        }
        let audit_id = durable_audit_id(&evidence.audit)?;
        if effect.identity_bearing() {
            let Some(observed) = evidence.effect.identity() else {
                return Err(OperationError::IdentityMismatch);
            };
            if effect
                .journaled_identity()
                .is_some_and(|expected| expected != observed)
            {
                return Err(OperationError::IdentityMismatch);
            }
        }
        self.journal.append_completed(
            self.phase,
            effect.step_id.clone(),
            effect.effect_id.clone(),
            effect.kind,
            effect.advance_to,
            effect.replay_class,
            evidence.effect.identity().cloned(),
            audit_id,
        )?;
        if let Some(destination) = effect.destination() {
            self.staged_destinations.insert(destination.clone());
        }
        self.current_effect = None;
        if self.state == OperationState::Finalizing {
            self.journal.append_terminal(
                self.phase,
                TerminalOutcomeKind::Closed,
                AuditRecordId::new("phase-10-terminal").map_err(|_| OperationError::Digest)?,
            )?;
            self.terminal = Some(TerminalOutcomeKind::Closed);
            self.state = OperationState::Closed;
            return Ok(());
        }
        if let Some(next) = effect.advance_to() {
            self.phase = next;
        }
        if let HoldState::Pending {
            requested_by,
            reason,
        } = std::mem::replace(&mut self.hold, HoldState::Clear)
        {
            self.hold = HoldState::Active {
                requested_by,
                reason,
            };
            self.state = OperationState::Held;
        } else {
            self.state = OperationState::Applying(self.phase);
        }
        Ok(())
    }

    /// Request a hold; an in-flight atomic effect is allowed to finish first.
    pub fn request_hold(
        &mut self,
        requested_by: OperatorId,
        reason: crate::model::HoldReason,
        audit: crate::model::AuditEvidence,
    ) -> Result<(), OperationError> {
        if self.state.is_terminal() {
            return Err(OperationError::Hold(HoldError::Terminal));
        }
        let audit_id = durable_audit_id(&audit)?;
        self.journal.append_hold_requested(self.phase, audit_id)?;
        if self.current_effect.is_some() {
            self.hold = HoldState::Pending {
                requested_by,
                reason,
            };
        } else {
            self.hold = HoldState::Active {
                requested_by,
                reason,
            };
            self.state = OperationState::Held;
        }
        Ok(())
    }

    /// Resume after a hold, revalidating the same candidate and inventory.
    pub fn resume(
        &mut self,
        operator: &OperatorId,
        context: &ApplyContext,
        audit: crate::model::AuditEvidence,
    ) -> Result<(), OperationError> {
        let Some(requested_by) = self.hold.requested_by() else {
            return Err(OperationError::Hold(HoldError::NotActive));
        };
        if requested_by != operator && self.request.operator_id() != operator {
            return Err(OperationError::Hold(HoldError::OperatorMismatch));
        }
        if self.state != OperationState::Held {
            return Err(OperationError::InvalidTransition);
        }
        self.validate_context(context)?;
        let audit_id = durable_audit_id(&audit)?;
        self.journal.append_hold_cleared(self.phase, audit_id)?;
        self.hold = HoldState::Clear;
        self.state = OperationState::Applying(self.phase);
        Ok(())
    }

    /// Classify replay of the current started effect without mutating a host.
    pub fn replay_decision(
        &self,
        observation: ReplayObservation,
    ) -> Result<ReplayDecision, OperationError> {
        let Some(effect) = self.current_effect.as_ref() else {
            let Some(record) = self.journal.incomplete_effect() else {
                return Err(OperationError::NoCurrentEffect);
            };
            return replay_for_record(record.replay_class(), record.identity(), observation);
        };
        replay_for_record(
            Some(effect.replay_class()),
            effect.journaled_identity(),
            observation,
        )
    }

    /// Verify all Zones and move to the separate finalization boundary.
    pub fn verify(
        &mut self,
        input: &VerificationInput,
    ) -> Result<VerificationReport, OperationError> {
        if self.request.operation_kind() != OperationKind::Cutover
            || self.state != OperationState::Applying(CutoverPhase::Verification)
        {
            return Err(OperationError::InvalidTransition);
        }
        let inventory = match self.request.inventory() {
            OperationInventory::Host(inventory) => inventory,
            OperationInventory::Reset(_) => return Err(OperationError::InventoryKindMismatch),
        };
        let report = verify_cutover(inventory, input)?;
        self.journal.append_phase_completed(
            CutoverPhase::Verification,
            AuditRecordId::new("phase-9-verification").map_err(|_| OperationError::Digest)?,
        )?;
        self.state = OperationState::CutoverSucceeded;
        Ok(report)
    }

    /// Consume the separate phase-10 consent.
    pub fn begin_finalization(
        &mut self,
        consent: &mut FinalizationConsent,
        now_ms: u64,
    ) -> Result<(), OperationError> {
        require_cutover_finalization(self.request.operation_kind(), CutoverPhase::Finalization)
            .map_err(OperationError::Finalization)?;
        if self.state != OperationState::CutoverSucceeded {
            return Err(OperationError::FinalizationConsentRequired);
        }
        consent
            .consume(&self.request.finalization_binding(), now_ms)
            .map_err(OperationError::Consent)?;
        self.phase = CutoverPhase::Finalization;
        self.journal.append_consent(self.phase)?;
        self.state = OperationState::Finalizing;
        Ok(())
    }

    /// Perform native rollback through phase 4 only.
    pub fn rollback(
        &mut self,
        audit: crate::model::AuditEvidence,
    ) -> Result<RollbackResult, OperationError> {
        if !matches!(
            self.state,
            OperationState::Applying(_) | OperationState::Held
        ) {
            return Err(OperationError::InvalidTransition);
        }
        if self.current_effect.is_some() {
            return Err(OperationError::InvalidTransition);
        }
        let inventory = match self.request.inventory() {
            OperationInventory::Host(inventory) => inventory,
            OperationInventory::Reset(_) => return Err(OperationError::InventoryKindMismatch),
        };
        let result = plan_native_rollback(
            self.phase,
            inventory,
            self.staged_destinations.iter().cloned(),
        )?;
        let audit_id = durable_audit_id(&audit)?;
        self.journal
            .append_terminal(self.phase, TerminalOutcomeKind::RolledBack, audit_id)?;
        self.terminal = Some(TerminalOutcomeKind::RolledBack);
        self.state = OperationState::RolledBack;
        self.rollback = Some(result.clone());
        Ok(result)
    }

    /// Publish a terminal external-restore outcome after phase 5 failure.
    pub fn require_external_restore(
        &mut self,
        audit: crate::model::AuditEvidence,
    ) -> Result<(), OperationError> {
        if self.phase.is_before_or_at_native_rollback_boundary() {
            return Err(OperationError::Rollback(RollbackError::BoundaryClosed(
                self.phase,
            )));
        }
        self.write_terminal(TerminalOutcomeKind::RestoreRequired, audit)
    }

    /// Publish a write-once terminal failure.
    pub fn fail_terminal(
        &mut self,
        _reason: FailureCode,
        audit: crate::model::AuditEvidence,
    ) -> Result<(), OperationError> {
        self.write_terminal(TerminalOutcomeKind::Failed, audit)
    }

    fn write_terminal(
        &mut self,
        outcome: TerminalOutcomeKind,
        audit: crate::model::AuditEvidence,
    ) -> Result<(), OperationError> {
        if self.terminal.is_some() {
            return Err(OperationError::TerminalAlreadyWritten);
        }
        let audit_id = durable_audit_id(&audit)?;
        self.journal
            .append_terminal(self.phase, outcome, audit_id)?;
        self.terminal = Some(outcome);
        self.state = match outcome {
            TerminalOutcomeKind::RolledBack => OperationState::RolledBack,
            TerminalOutcomeKind::RestoreRequired => OperationState::RestoreRequired,
            TerminalOutcomeKind::Closed => OperationState::Closed,
            TerminalOutcomeKind::Failed => OperationState::Failed,
        };
        Ok(())
    }

    fn validate_context(&self, context: &ApplyContext) -> Result<(), OperationError> {
        if &context.inventory_digest != self.request.inventory_digest()
            || !context.candidate_current
            || !context.markers_valid
            || !context.ownership_valid
        {
            return Err(OperationError::ContextMismatch);
        }
        if self.request.operation_kind().is_cutover() {
            let Some(recovery) = context.recovery.as_ref() else {
                return Err(OperationError::Recovery(ConsentError::RecoveryMismatch));
            };
            let Some(host_digest) = context.host_digest.as_ref() else {
                return Err(OperationError::Recovery(ConsentError::RecoveryMismatch));
            };
            recovery
                .validate_for(
                    self.request.candidate_id(),
                    self.request.preview_digest(),
                    self.request.operator_id(),
                    host_digest,
                    context.now_ms,
                )
                .map_err(OperationError::Recovery)?;
            if Some(recovery.digest().map_err(OperationError::Consent)?)
                != self.request.recovery_digest
            {
                return Err(OperationError::Recovery(ConsentError::RecoveryMismatch));
            }
        }
        Ok(())
    }
}

fn durable_audit_id(audit: &crate::model::AuditEvidence) -> Result<AuditRecordId, OperationError> {
    if !audit.is_durable() {
        return Err(OperationError::AuditNotDurable);
    }
    audit
        .record_id()
        .cloned()
        .ok_or(OperationError::AuditNotDurable)
}

fn replay_for_record(
    replay_class: Option<ReplayClass>,
    journaled_identity: Option<&ArtifactId>,
    observation: ReplayObservation,
) -> Result<ReplayDecision, OperationError> {
    let Some(replay_class) = replay_class else {
        return Err(OperationError::NoCurrentEffect);
    };
    Ok(match replay_class {
        ReplayClass::Repeatable => match observation {
            ReplayObservation::Absent => ReplayDecision::Repeat,
            _ => ReplayDecision::Quarantine(FailureCode::DestinationAmbiguous),
        },
        ReplayClass::ReopenByJournaledIdentity => {
            let Some(expected) = journaled_identity else {
                return Ok(ReplayDecision::Quarantine(FailureCode::IdentityMismatch));
            };
            match observation {
                ReplayObservation::JournaledIdentity(observed) if &observed == expected => {
                    ReplayDecision::Reopen(observed)
                }
                ReplayObservation::WrongIdentity
                | ReplayObservation::DuplicateIdentity
                | ReplayObservation::InvalidMarker
                | ReplayObservation::PartialDestination
                | ReplayObservation::ReplacedDestination
                | ReplayObservation::ForeignOwner
                | ReplayObservation::Ambiguous
                | ReplayObservation::JournaledIdentity(_) => {
                    ReplayDecision::Quarantine(FailureCode::IdentityMismatch)
                }
                ReplayObservation::Absent => {
                    ReplayDecision::Quarantine(FailureCode::IdentityMismatch)
                }
            }
        }
        ReplayClass::QuarantineOnly => {
            ReplayDecision::Quarantine(FailureCode::DestinationAmbiguous)
        }
    })
}

/// Operation state-machine failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationError {
    /// A preview, request, or inventory did not match.
    PreviewMismatch,
    /// The operation kind and inventory kind were different.
    InventoryKindMismatch,
    /// Inventory construction failed.
    Inventory(InventoryError),
    /// Preview canonicalization failed.
    Preview(PreviewError),
    /// Consent validation failed.
    Consent(ConsentError),
    /// Recovery evidence failed.
    Recovery(ConsentError),
    /// Journal validation or append failed.
    Journal(JournalError),
    /// Canonical request encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// A digest could not be represented.
    Digest,
    /// A state transition was not valid.
    InvalidTransition,
    /// The host-wide lock is owned by another operation.
    LockContended(OperationId),
    /// This operation does not hold the host lock.
    LockNotHeld,
    /// One effect was not authorized by the operation kind.
    EffectNotAllowed(EffectKind),
    /// Identity-bearing effects must use reopen replay semantics.
    IdentityReplayClassRequired,
    /// The requested effect is not the in-flight effect.
    EffectMismatch,
    /// No effect is currently in flight.
    NoCurrentEffect,
    /// The effect result was a failure.
    EffectFailed,
    /// The effect result was ambiguous.
    DestinationAmbiguous,
    /// The effect or audit evidence was incomplete.
    AuditNotDurable,
    /// An identity-bearing result was absent or mismatched.
    IdentityMismatch,
    /// Read-only phase predicates were not proven.
    PhaseEvidence,
    /// Apply or resume boundary evidence drifted.
    ContextMismatch,
    /// Hold transition failed.
    Hold(HoldError),
    /// Rollback boundary failed.
    Rollback(RollbackError),
    /// Reset authority rejected the request.
    Reset(ResetError),
    /// Verification failed.
    Verification(VerificationError),
    /// Finalization authority rejected the request.
    Finalization(FinalizationError),
    /// A terminal outcome was already written.
    TerminalAlreadyWritten,
    /// A reset or pre-finalization path attempted finalization.
    FinalizationConsentRequired,
}

impl OperationError {
    /// Return the stable failure class.
    pub const fn code(&self) -> FailureCode {
        match self {
            Self::PreviewMismatch | Self::InventoryKindMismatch | Self::ContextMismatch => {
                FailureCode::CandidateDrift
            }
            Self::Inventory(error) => error.code(),
            Self::Preview(_) | Self::CanonicalJson(_) | Self::Digest => {
                FailureCode::RequestMismatch
            }
            Self::Consent(_) | Self::Recovery(_) => FailureCode::ConsentInvalid,
            Self::Journal(error) => error.code(),
            Self::InvalidTransition => FailureCode::InvalidTransition,
            Self::LockContended(_) => FailureCode::LockContended,
            Self::LockNotHeld => FailureCode::LockContended,
            Self::EffectNotAllowed(_) | Self::Reset(_) | Self::Finalization(_) => {
                FailureCode::EffectNotAllowed
            }
            Self::IdentityReplayClassRequired | Self::IdentityMismatch => {
                FailureCode::IdentityMismatch
            }
            Self::EffectMismatch
            | Self::NoCurrentEffect
            | Self::EffectFailed
            | Self::DestinationAmbiguous => FailureCode::EffectFailed,
            Self::AuditNotDurable => FailureCode::AuditNotDurable,
            Self::PhaseEvidence => FailureCode::InvalidTransition,
            Self::Hold(_) => FailureCode::HoldActive,
            Self::Rollback(error) => error.code(),
            Self::Verification(error) => error.code(),
            Self::TerminalAlreadyWritten => FailureCode::TerminalAlreadyWritten,
            Self::FinalizationConsentRequired => FailureCode::FinalizationConsentRequired,
        }
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PreviewMismatch => "operation preview mismatch",
            Self::InventoryKindMismatch => "operation inventory kind mismatch",
            Self::Inventory(_) => "operation inventory rejected",
            Self::Preview(_) => "operation preview rejected",
            Self::Consent(_) => "operation consent rejected",
            Self::Recovery(_) => "operation recovery evidence rejected",
            Self::Journal(_) => "operation journal rejected",
            Self::CanonicalJson(_) => "operation request canonicalization failed",
            Self::Digest => "operation digest failed",
            Self::InvalidTransition => "operation transition invalid",
            Self::LockContended(_) => "host operation lock contended",
            Self::LockNotHeld => "host operation lock not held",
            Self::EffectNotAllowed(_) => "operation effect not allowed",
            Self::IdentityReplayClassRequired => "identity effect replay class invalid",
            Self::EffectMismatch => "operation effect mismatch",
            Self::NoCurrentEffect => "operation has no current effect",
            Self::EffectFailed => "operation effect failed",
            Self::DestinationAmbiguous => "operation destination is ambiguous",
            Self::AuditNotDurable => "operation audit is not durable",
            Self::IdentityMismatch => "operation identity mismatch",
            Self::PhaseEvidence => "operation phase evidence failed",
            Self::ContextMismatch => "operation context drifted",
            Self::Hold(_) => "operation hold transition failed",
            Self::Rollback(_) => "operation rollback rejected",
            Self::Reset(_) => "operation reset scope rejected",
            Self::Verification(_) => "operation verification rejected",
            Self::Finalization(_) => "operation finalization rejected",
            Self::TerminalAlreadyWritten => "operation terminal outcome already written",
            Self::FinalizationConsentRequired => "operation finalization consent required",
        })
    }
}

impl std::error::Error for OperationError {}

impl From<InventoryError> for OperationError {
    fn from(error: InventoryError) -> Self {
        Self::Inventory(error)
    }
}

impl From<PreviewError> for OperationError {
    fn from(error: PreviewError) -> Self {
        Self::Preview(error)
    }
}

impl From<JournalError> for OperationError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<RollbackError> for OperationError {
    fn from(error: RollbackError) -> Self {
        Self::Rollback(error)
    }
}

impl From<VerificationError> for OperationError {
    fn from(error: VerificationError) -> Self {
        Self::Verification(error)
    }
}

impl From<ResetError> for OperationError {
    fn from(error: ResetError) -> Self {
        Self::Reset(error)
    }
}
