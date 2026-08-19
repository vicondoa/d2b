//! Digest-bound append-only journal model.

use std::fmt;

use d2b_contracts::v3::{
    CanonicalJsonError, CanonicalJsonValue, canonical_digest, canonical_json_bytes,
};
use serde::{Deserialize, Serialize};

use crate::model::{
    ArtifactId, AuditRecordId, CutoverPhase, Digest, EffectId, EffectKind, FailureCode,
    OperationId, ReplayClass, RevisionPlanId, StepId, TerminalOutcomeKind,
};

/// Domain separator for journal record digests.
pub const JOURNAL_DOMAIN: &str = "d2b:cutover:journal-record:v1";

/// The closed kinds of journal record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JournalRecordKind {
    /// Consent was consumed before any mutation.
    ConsentConsumed,
    /// An effect was durably marked started before mutation.
    Started,
    /// An effect completed with durable effect and audit evidence.
    Completed,
    /// A read-only phase completed.
    PhaseCompleted,
    /// A hold was requested.
    HoldRequested,
    /// A hold was cleared or resumed.
    HoldCleared,
    /// A terminal result was written once.
    Terminal,
}

/// The binding carried by every journal record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalBinding {
    operation_id: OperationId,
    revision_plan_id: RevisionPlanId,
    request_digest: Digest,
}

impl JournalBinding {
    /// Construct the immutable journal binding.
    pub fn new(
        operation_id: OperationId,
        revision_plan_id: RevisionPlanId,
        request_digest: Digest,
    ) -> Self {
        Self {
            operation_id,
            revision_plan_id,
            request_digest,
        }
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Borrow the revision-plan identity.
    pub fn revision_plan_id(&self) -> &RevisionPlanId {
        &self.revision_plan_id
    }

    /// Borrow the request digest.
    pub fn request_digest(&self) -> &Digest {
        &self.request_digest
    }
}

/// One tamper-evident journal record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JournalRecord {
    sequence: u64,
    operation_id: OperationId,
    revision_plan_id: RevisionPlanId,
    request_digest: Digest,
    previous_record_digest: Option<Digest>,
    kind: JournalRecordKind,
    phase: CutoverPhase,
    step_id: Option<StepId>,
    effect_id: Option<EffectId>,
    effect_kind: Option<EffectKind>,
    next_phase: Option<CutoverPhase>,
    replay_class: Option<ReplayClass>,
    identity: Option<ArtifactId>,
    audit_record_id: Option<AuditRecordId>,
    terminal_outcome: Option<TerminalOutcomeKind>,
    record_digest: Digest,
}

impl JournalRecord {
    /// Return the sequence number.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Return the record kind.
    pub const fn kind(&self) -> JournalRecordKind {
        self.kind
    }

    /// Return the phase attached to the record.
    pub const fn phase(&self) -> CutoverPhase {
        self.phase
    }

    /// Borrow the effect identity, if this record carries one.
    pub fn effect_id(&self) -> Option<&EffectId> {
        self.effect_id.as_ref()
    }

    /// Return the closed effect kind, if this is an effect record.
    pub const fn effect_kind(&self) -> Option<EffectKind> {
        self.effect_kind
    }

    /// Return the phase selected after successful effect completion.
    pub const fn next_phase(&self) -> Option<CutoverPhase> {
        self.next_phase
    }

    /// Borrow the step identity, if this record carries one.
    pub fn step_id(&self) -> Option<&StepId> {
        self.step_id.as_ref()
    }

    /// Return the replay class, if this is an effect record.
    pub const fn replay_class(&self) -> Option<ReplayClass> {
        self.replay_class
    }

    /// Borrow the journaled identity, if present.
    pub fn identity(&self) -> Option<&ArtifactId> {
        self.identity.as_ref()
    }

    /// Borrow the durable audit identity, if present.
    pub fn audit_record_id(&self) -> Option<&AuditRecordId> {
        self.audit_record_id.as_ref()
    }

    /// Return the terminal outcome, if this is a terminal record.
    pub const fn terminal_outcome(&self) -> Option<TerminalOutcomeKind> {
        self.terminal_outcome
    }

    /// Borrow this record's digest.
    pub fn record_digest(&self) -> &Digest {
        &self.record_digest
    }

    fn unsigned(&self) -> JournalRecordUnsigned {
        JournalRecordUnsigned {
            sequence: self.sequence,
            operation_id: self.operation_id.clone(),
            revision_plan_id: self.revision_plan_id.clone(),
            request_digest: self.request_digest.clone(),
            previous_record_digest: self.previous_record_digest.clone(),
            kind: self.kind,
            phase: self.phase,
            step_id: self.step_id.clone(),
            effect_id: self.effect_id.clone(),
            effect_kind: self.effect_kind,
            next_phase: self.next_phase,
            replay_class: self.replay_class,
            identity: self.identity.clone(),
            audit_record_id: self.audit_record_id.clone(),
            terminal_outcome: self.terminal_outcome,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct JournalRecordUnsigned {
    sequence: u64,
    operation_id: OperationId,
    revision_plan_id: RevisionPlanId,
    request_digest: Digest,
    previous_record_digest: Option<Digest>,
    kind: JournalRecordKind,
    phase: CutoverPhase,
    step_id: Option<StepId>,
    effect_id: Option<EffectId>,
    effect_kind: Option<EffectKind>,
    next_phase: Option<CutoverPhase>,
    replay_class: Option<ReplayClass>,
    identity: Option<ArtifactId>,
    audit_record_id: Option<AuditRecordId>,
    terminal_outcome: Option<TerminalOutcomeKind>,
}

/// A pure append-only journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    binding: JournalBinding,
    records: Vec<JournalRecord>,
}

impl Journal {
    /// Start an empty journal for one immutable request binding.
    pub const fn new(binding: JournalBinding) -> Self {
        Self {
            binding,
            records: Vec::new(),
        }
    }

    /// Borrow the immutable journal binding.
    pub fn binding(&self) -> &JournalBinding {
        &self.binding
    }

    /// Borrow records in sequence order.
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    /// Return the canonical JSONL representation.
    pub fn to_bytes(&self) -> Result<Vec<u8>, JournalError> {
        let mut bytes = Vec::new();
        for record in &self.records {
            bytes.extend(canonical_json_bytes(record).map_err(JournalError::CanonicalJson)?);
            bytes.push(b'\n');
        }
        Ok(bytes)
    }

    /// Reopen a journal only after proving its chain and request binding.
    pub fn from_bytes(binding: JournalBinding, bytes: &[u8]) -> Result<Self, JournalError> {
        if bytes.is_empty() {
            return Ok(Self::new(binding));
        }
        if !bytes.ends_with(b"\n") {
            return Err(JournalError::Truncated);
        }

        let mut journal = Self::new(binding);
        let lines = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if index == lines.len() - 1 {
                continue;
            }
            if line.is_empty() {
                return Err(JournalError::Malformed);
            }
            let parsed = CanonicalJsonValue::parse(line).map_err(JournalError::CanonicalJson)?;
            if parsed.to_canonical_bytes() != *line {
                return Err(JournalError::NonCanonical);
            }
            let record: JournalRecord =
                serde_json::from_slice(line).map_err(|_| JournalError::Malformed)?;
            journal.validate_record(&record)?;
            journal.records.push(record);
        }
        Ok(journal)
    }

    /// Append a consent-consumed record.
    pub fn append_consent(&mut self, phase: CutoverPhase) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::ConsentConsumed,
            phase,
            step_id: None,
            effect_id: None,
            effect_kind: None,
            next_phase: None,
            replay_class: None,
            identity: None,
            audit_record_id: None,
            terminal_outcome: None,
        })
    }

    /// Append a started record before an effect is invoked.
    #[allow(clippy::too_many_arguments)]
    pub fn append_started(
        &mut self,
        phase: CutoverPhase,
        step_id: StepId,
        effect_id: EffectId,
        effect_kind: EffectKind,
        next_phase: Option<CutoverPhase>,
        replay_class: ReplayClass,
        identity: Option<ArtifactId>,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::Started,
            phase,
            step_id: Some(step_id),
            effect_id: Some(effect_id),
            effect_kind: Some(effect_kind),
            next_phase,
            replay_class: Some(replay_class),
            identity,
            audit_record_id: None,
            terminal_outcome: None,
        })
    }

    /// Append a completed record after both effect and audit durability.
    #[allow(clippy::too_many_arguments)]
    pub fn append_completed(
        &mut self,
        phase: CutoverPhase,
        step_id: StepId,
        effect_id: EffectId,
        effect_kind: EffectKind,
        next_phase: Option<CutoverPhase>,
        replay_class: ReplayClass,
        identity: Option<ArtifactId>,
        audit_record_id: AuditRecordId,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::Completed,
            phase,
            step_id: Some(step_id),
            effect_id: Some(effect_id),
            effect_kind: Some(effect_kind),
            next_phase,
            replay_class: Some(replay_class),
            identity,
            audit_record_id: Some(audit_record_id),
            terminal_outcome: None,
        })
    }

    /// Append a read-only phase completion record.
    pub fn append_phase_completed(
        &mut self,
        phase: CutoverPhase,
        audit_record_id: AuditRecordId,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::PhaseCompleted,
            phase,
            step_id: None,
            effect_id: None,
            effect_kind: None,
            next_phase: None,
            replay_class: None,
            identity: None,
            audit_record_id: Some(audit_record_id),
            terminal_outcome: None,
        })
    }

    /// Append a hold request.
    pub fn append_hold_requested(
        &mut self,
        phase: CutoverPhase,
        audit_record_id: AuditRecordId,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::HoldRequested,
            phase,
            step_id: None,
            effect_id: None,
            effect_kind: None,
            next_phase: None,
            replay_class: None,
            identity: None,
            audit_record_id: Some(audit_record_id),
            terminal_outcome: None,
        })
    }

    /// Append a hold clear or resume record.
    pub fn append_hold_cleared(
        &mut self,
        phase: CutoverPhase,
        audit_record_id: AuditRecordId,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::HoldCleared,
            phase,
            step_id: None,
            effect_id: None,
            effect_kind: None,
            next_phase: None,
            replay_class: None,
            identity: None,
            audit_record_id: Some(audit_record_id),
            terminal_outcome: None,
        })
    }

    /// Append one write-once terminal record.
    pub fn append_terminal(
        &mut self,
        phase: CutoverPhase,
        outcome: TerminalOutcomeKind,
        audit_record_id: AuditRecordId,
    ) -> Result<(), JournalError> {
        self.append(JournalRecordInput {
            kind: JournalRecordKind::Terminal,
            phase,
            step_id: None,
            effect_id: None,
            effect_kind: None,
            next_phase: None,
            replay_class: None,
            identity: None,
            audit_record_id: Some(audit_record_id),
            terminal_outcome: Some(outcome),
        })
    }

    /// Return the most recent started effect without a later completion.
    pub fn incomplete_effect(&self) -> Option<&JournalRecord> {
        self.records.iter().rev().find(|record| {
            record.kind == JournalRecordKind::Started
                && record.effect_id.as_ref().is_some_and(|effect_id| {
                    !self.records.iter().any(|completed| {
                        completed.kind == JournalRecordKind::Completed
                            && completed.effect_id.as_ref() == Some(effect_id)
                            && completed.sequence > record.sequence
                    })
                })
        })
    }

    /// Return whether a terminal record has been written.
    pub fn is_terminal(&self) -> bool {
        self.records
            .iter()
            .any(|record| record.kind == JournalRecordKind::Terminal)
    }

    fn append(&mut self, input: JournalRecordInput) -> Result<(), JournalError> {
        if self.is_terminal() {
            return Err(JournalError::TerminalAlreadyWritten);
        }
        if input.kind == JournalRecordKind::Completed
            && !self.has_open_effect(input.effect_id.as_ref())
        {
            return Err(JournalError::Reordered);
        }
        if input.kind == JournalRecordKind::Started
            && self.has_completed_effect(input.effect_id.as_ref())
        {
            return Err(JournalError::Reordered);
        }
        if input.kind == JournalRecordKind::Terminal && input.terminal_outcome.is_none() {
            return Err(JournalError::Malformed);
        }
        let sequence = self.records.len() as u64;
        let previous_record_digest = self
            .records
            .last()
            .map(|record| record.record_digest.clone());
        let unsigned = JournalRecordUnsigned {
            sequence,
            operation_id: self.binding.operation_id.clone(),
            revision_plan_id: self.binding.revision_plan_id.clone(),
            request_digest: self.binding.request_digest.clone(),
            previous_record_digest,
            kind: input.kind,
            phase: input.phase,
            step_id: input.step_id,
            effect_id: input.effect_id,
            effect_kind: input.effect_kind,
            next_phase: input.next_phase,
            replay_class: input.replay_class,
            identity: input.identity,
            audit_record_id: input.audit_record_id,
            terminal_outcome: input.terminal_outcome,
        };
        let bytes = canonical_json_bytes(&unsigned).map_err(JournalError::CanonicalJson)?;
        let record_digest = Digest::parse(canonical_digest(JOURNAL_DOMAIN, &bytes))
            .map_err(|_| JournalError::Digest)?;
        self.records.push(JournalRecord {
            sequence,
            operation_id: unsigned.operation_id,
            revision_plan_id: unsigned.revision_plan_id,
            request_digest: unsigned.request_digest,
            previous_record_digest: unsigned.previous_record_digest,
            kind: unsigned.kind,
            phase: unsigned.phase,
            step_id: unsigned.step_id,
            effect_id: unsigned.effect_id,
            effect_kind: unsigned.effect_kind,
            next_phase: unsigned.next_phase,
            replay_class: unsigned.replay_class,
            identity: unsigned.identity,
            audit_record_id: unsigned.audit_record_id,
            terminal_outcome: unsigned.terminal_outcome,
            record_digest,
        });
        Ok(())
    }

    fn has_open_effect(&self, effect_id: Option<&EffectId>) -> bool {
        let Some(effect_id) = effect_id else {
            return false;
        };
        let started = self.records.iter().any(|record| {
            record.kind == JournalRecordKind::Started
                && record.effect_id.as_ref() == Some(effect_id)
        });
        let completed = self.records.iter().any(|record| {
            record.kind == JournalRecordKind::Completed
                && record.effect_id.as_ref() == Some(effect_id)
        });
        started && !completed
    }

    fn has_completed_effect(&self, effect_id: Option<&EffectId>) -> bool {
        let Some(effect_id) = effect_id else {
            return false;
        };
        self.records.iter().any(|record| {
            record.kind == JournalRecordKind::Completed
                && record.effect_id.as_ref() == Some(effect_id)
        })
    }

    fn validate_record(&self, record: &JournalRecord) -> Result<(), JournalError> {
        let expected_sequence = self.records.len() as u64;
        if record.sequence != expected_sequence
            || record.operation_id != self.binding.operation_id
            || record.revision_plan_id != self.binding.revision_plan_id
            || record.request_digest != self.binding.request_digest
        {
            return Err(JournalError::RequestMismatch);
        }
        let expected_previous = self
            .records
            .last()
            .map(|previous| previous.record_digest.clone());
        if record.previous_record_digest != expected_previous {
            return Err(JournalError::Reordered);
        }
        let bytes =
            canonical_json_bytes(&record.unsigned()).map_err(JournalError::CanonicalJson)?;
        let expected_digest = Digest::parse(canonical_digest(JOURNAL_DOMAIN, &bytes))
            .map_err(|_| JournalError::Digest)?;
        if record.record_digest != expected_digest {
            return Err(JournalError::Tampered);
        }
        if record.kind == JournalRecordKind::Completed
            && !self.has_open_effect(record.effect_id.as_ref())
        {
            return Err(JournalError::Reordered);
        }
        if record.kind == JournalRecordKind::Started
            && self.has_completed_effect(record.effect_id.as_ref())
        {
            return Err(JournalError::Reordered);
        }
        if record.kind == JournalRecordKind::Terminal && record.terminal_outcome.is_none() {
            return Err(JournalError::Malformed);
        }
        if self.is_terminal() {
            return Err(JournalError::TerminalAlreadyWritten);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct JournalRecordInput {
    kind: JournalRecordKind,
    phase: CutoverPhase,
    step_id: Option<StepId>,
    effect_id: Option<EffectId>,
    effect_kind: Option<EffectKind>,
    next_phase: Option<CutoverPhase>,
    replay_class: Option<ReplayClass>,
    identity: Option<ArtifactId>,
    audit_record_id: Option<AuditRecordId>,
    terminal_outcome: Option<TerminalOutcomeKind>,
}

/// Journal integrity failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// Canonical JSON rejected a record.
    CanonicalJson(CanonicalJsonError),
    /// The line was not terminated by the append-only newline.
    Truncated,
    /// The JSON shape was not the closed record shape.
    Malformed,
    /// The line was valid JSON but not canonical bytes.
    NonCanonical,
    /// A record's request binding did not match.
    RequestMismatch,
    /// A record's previous digest or sequence was reordered.
    Reordered,
    /// A record digest was changed.
    Tampered,
    /// A digest could not be represented canonically.
    Digest,
    /// A terminal record was followed by another record.
    TerminalAlreadyWritten,
}

impl JournalError {
    /// Return the corresponding fail-closed failure class.
    pub const fn code(&self) -> FailureCode {
        match self {
            Self::CanonicalJson(_) | Self::Malformed | Self::NonCanonical | Self::Digest => {
                FailureCode::JournalTampered
            }
            Self::Truncated | Self::RequestMismatch => FailureCode::RequestMismatch,
            Self::Reordered => FailureCode::JournalTampered,
            Self::Tampered => FailureCode::JournalTampered,
            Self::TerminalAlreadyWritten => FailureCode::TerminalAlreadyWritten,
        }
    }
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalJson(_) => "journal canonical JSON rejected",
            Self::Truncated => "journal is truncated",
            Self::Malformed => "journal record shape rejected",
            Self::NonCanonical => "journal record is not canonical",
            Self::RequestMismatch => "journal request binding mismatch",
            Self::Reordered => "journal record order mismatch",
            Self::Tampered => "journal record digest mismatch",
            Self::Digest => "journal digest rejected",
            Self::TerminalAlreadyWritten => "journal terminal outcome already written",
        })
    }
}

impl std::error::Error for JournalError {}
