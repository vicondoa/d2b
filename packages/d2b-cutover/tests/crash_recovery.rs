use d2b_cutover::{
    ApplyContext, ArtifactId, AuditEvidence, CandidateId, CompletionEvidence, Consent,
    CutoverEngine, CutoverPhase, CutoverPreview, Digest, EffectEvidence, EffectId, EffectKind,
    EffectRequest, FailureCode, HostInventory, HostLockContract, Journal, JournalBinding,
    JournalError, JournalRecordKind, OperationId, OperationInventory, OperationKind,
    OperationRequest, OperatorId, RecoveryAttestation, RecoveryId, ReplayClass, ReplayDecision,
    ReplayObservation, RevisionPlanId, StepId, ZoneId, ZoneInventory,
};

fn digest(label: &str) -> Digest {
    Digest::derive("d2b:test", label.as_bytes())
}

fn valid_evidence() -> Vec<u8> {
    format!(
        r#"{{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"{}","previewDigest":"{}","operatorId":"operator-1","restoreInstructionsDigest":"{}","issuedAtMs":900,"expiresAtMs":1900,"qualified":true}}"#,
        digest("host"),
        digest("preview"),
        digest("restore"),
    )
    .into_bytes()
}

fn engine() -> CutoverEngine {
    let inventory = HostInventory::build(
        [ZoneId::new("zone-a").unwrap()],
        [ZoneInventory::empty("zone-a").unwrap()],
        [],
    )
    .unwrap();
    let operation_id = OperationId::new("operation-1").unwrap();
    let candidate_id = CandidateId::new("candidate-1").unwrap();
    let revision = RevisionPlanId::new("revision-1").unwrap();
    let operator = OperatorId::new("operator-1").unwrap();
    let preview = CutoverPreview::new(
        operation_id.clone(),
        OperationKind::Cutover,
        candidate_id.clone(),
        revision.clone(),
        inventory.clone(),
        None,
    )
    .unwrap();
    let preview_digest = preview.digest().unwrap();
    let recovery = RecoveryAttestation::new(
        RecoveryId::new("recovery-1").unwrap(),
        candidate_id,
        digest("host"),
        preview_digest.clone(),
        operator.clone(),
        digest("restore"),
        900,
        1_900,
        true,
    )
    .unwrap();
    let request = OperationRequest::new_cutover(
        operation_id,
        CandidateId::new("candidate-1").unwrap(),
        revision,
        operator.clone(),
        preview_digest,
        recovery.digest().unwrap(),
        inventory.clone(),
    )
    .unwrap();
    let mut engine = CutoverEngine::new(request, &preview).unwrap();
    let mut lock = HostLockContract::new();
    engine.acquire_host_lock(&mut lock).unwrap();
    let context = ApplyContext::cutover(
        1_000,
        inventory.digest().unwrap(),
        true,
        true,
        true,
        recovery,
        digest("host"),
    );
    let mut consent = Consent::issue(engine.request().consent_binding(), 900, 1_900).unwrap();
    engine.begin_apply(&mut consent, &context).unwrap();
    for phase in [
        CutoverPhase::Preflight,
        CutoverPhase::Consent,
        CutoverPhase::Inventory,
    ] {
        engine
            .complete_read_only_phase(
                phase,
                d2b_cutover::ReadOnlyEvidence {
                    predicates_hold: true,
                    audit: AuditEvidence::durable(format!("audit-{phase:?}")).unwrap(),
                },
            )
            .unwrap();
    }
    engine
}

fn journal_fixture() -> (JournalBinding, Journal) {
    let binding = JournalBinding::new(
        OperationId::new("operation-1").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        digest("request"),
    );
    let mut journal = Journal::new(binding.clone());
    journal.append_consent(CutoverPhase::Preflight).unwrap();
    journal
        .append_started(
            CutoverPhase::Drain,
            StepId::new("step-1").unwrap(),
            EffectId::new("effect-1").unwrap(),
            EffectKind::HostDrain,
            Some(CutoverPhase::Disposition),
            ReplayClass::Repeatable,
            None,
        )
        .unwrap();
    journal
        .append_completed(
            CutoverPhase::Drain,
            StepId::new("step-1").unwrap(),
            EffectId::new("effect-1").unwrap(),
            EffectKind::HostDrain,
            Some(CutoverPhase::Disposition),
            ReplayClass::Repeatable,
            None,
            d2b_cutover::AuditRecordId::new("audit-1").unwrap(),
        )
        .unwrap();
    (binding, journal)
}

#[test]
fn strict_evidence_rejects_duplicate_unknown_trailing_fractional_negative_and_large_values() {
    let valid = valid_evidence();
    assert!(RecoveryAttestation::decode_json(&valid).is_ok());
    let duplicate = valid
        .strip_suffix(b"}")
        .unwrap()
        .iter()
        .copied()
        .chain(br#","qualified":true}"#.iter().copied())
        .collect::<Vec<_>>();
    assert!(matches!(
        RecoveryAttestation::decode_json(&duplicate),
        Err(d2b_cutover::ConsentError::CanonicalJson(_))
    ));

    for malformed in [
        &br#"{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","previewDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","operatorId":"operator-1","restoreInstructionsDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","issuedAtMs":900,"expiresAtMs":1900,"qualified":true,"unknown":1}"#[..],
        &br#"{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","previewDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","operatorId":"operator-1","restoreInstructionsDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","issuedAtMs":900.5,"expiresAtMs":1900,"qualified":true}"#[..],
        &br#"{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","previewDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","operatorId":"operator-1","restoreInstructionsDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","issuedAtMs":-1,"expiresAtMs":1900,"qualified":true}"#[..],
        &br#"{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","previewDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","operatorId":"operator-1","restoreInstructionsDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","issuedAtMs":900,"expiresAtMs":9223372036854775808,"qualified":true}"#[..],
        &br#"{"recoveryId":"recovery-1","candidateId":"candidate-1","hostDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","previewDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","operatorId":"operator-1","restoreInstructionsDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","issuedAtMs":900,"expiresAtMs":1900,"qualified":true} trailing"#[..],
    ] {
        assert!(RecoveryAttestation::decode_json(malformed).is_err());
    }
}

#[test]
fn journal_detects_bit_flip_truncation_reorder_and_request_mismatch() {
    let (binding, journal) = journal_fixture();
    let bytes = journal.to_bytes().unwrap();
    assert_eq!(
        Journal::from_bytes(binding.clone(), &bytes)
            .unwrap()
            .records()
            .len(),
        3
    );

    let mut bit_flip = bytes.clone();
    let offset = bit_flip
        .windows(b"recordDigest\":\"sha256:".len())
        .position(|window| window == b"recordDigest\":\"sha256:")
        .unwrap()
        + b"recordDigest\":\"sha256:".len();
    bit_flip[offset] = if bit_flip[offset] == b'0' { b'1' } else { b'0' };
    assert_eq!(
        Journal::from_bytes(binding.clone(), &bit_flip),
        Err(JournalError::Tampered)
    );
    assert_eq!(
        Journal::from_bytes(binding.clone(), &bytes[..bytes.len() - 1]),
        Err(JournalError::Truncated)
    );

    let mut lines = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    lines.pop();
    lines.reverse();
    let reordered = lines
        .into_iter()
        .flat_map(|line| line.iter().copied().chain(*b"\n"))
        .collect::<Vec<_>>();
    assert!(matches!(
        Journal::from_bytes(binding.clone(), &reordered),
        Err(JournalError::RequestMismatch | JournalError::Reordered | JournalError::Tampered)
    ));

    let other_binding = JournalBinding::new(
        OperationId::new("operation-other").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        digest("request"),
    );
    assert_eq!(
        Journal::from_bytes(other_binding, &bytes),
        Err(JournalError::RequestMismatch)
    );
}

#[test]
fn replay_classes_repeat_reopen_or_quarantine_without_creating_identity() {
    let mut repeatable = engine();
    let repeatable_request = EffectRequest::new(
        EffectId::new("effect-repeat").unwrap(),
        StepId::new("step-repeat").unwrap(),
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        None,
    );
    repeatable.start_effect(repeatable_request).unwrap();
    assert_eq!(
        repeatable
            .replay_decision(ReplayObservation::Absent)
            .unwrap(),
        ReplayDecision::Repeat
    );

    let mut identity = engine();
    let identity_request = EffectRequest::new(
        EffectId::new("effect-store").unwrap(),
        StepId::new("step-store").unwrap(),
        EffectKind::ResourceStoreCreate,
        ReplayClass::ReopenByJournaledIdentity,
        None,
    )
    .with_identity(
        Some(ArtifactId::new("store-uuid").unwrap()),
        Some(ArtifactId::new("store-stage").unwrap()),
    );
    identity.start_effect(identity_request).unwrap();
    assert_eq!(
        identity
            .replay_decision(ReplayObservation::JournaledIdentity(
                ArtifactId::new("store-uuid").unwrap()
            ))
            .unwrap(),
        ReplayDecision::Reopen(ArtifactId::new("store-uuid").unwrap())
    );
    for observation in [
        ReplayObservation::WrongIdentity,
        ReplayObservation::DuplicateIdentity,
        ReplayObservation::InvalidMarker,
        ReplayObservation::PartialDestination,
        ReplayObservation::ReplacedDestination,
        ReplayObservation::ForeignOwner,
        ReplayObservation::Ambiguous,
        ReplayObservation::Absent,
    ] {
        assert_eq!(
            identity.replay_decision(observation).unwrap(),
            ReplayDecision::Quarantine(FailureCode::IdentityMismatch)
        );
    }

    let mut quarantine = engine();
    quarantine
        .start_effect(EffectRequest::new(
            EffectId::new("effect-final").unwrap(),
            StepId::new("step-final").unwrap(),
            EffectKind::ClosureActivation,
            ReplayClass::QuarantineOnly,
            None,
        ))
        .unwrap();
    assert_eq!(
        quarantine
            .replay_decision(ReplayObservation::Absent)
            .unwrap(),
        ReplayDecision::Quarantine(FailureCode::DestinationAmbiguous)
    );
}

#[test]
fn identity_completion_reopens_the_journaled_identity_and_never_creates_again() {
    let mut engine = engine();
    let request = EffectRequest::new(
        EffectId::new("effect-store").unwrap(),
        StepId::new("step-store").unwrap(),
        EffectKind::ResourceStoreCreate,
        ReplayClass::ReopenByJournaledIdentity,
        None,
    )
    .with_identity(
        Some(ArtifactId::new("store-uuid").unwrap()),
        Some(ArtifactId::new("store-stage").unwrap()),
    );
    engine.start_effect(request.clone()).unwrap();
    engine
        .complete_effect(
            request.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded_with_identity("store-uuid").unwrap(),
                audit: AuditEvidence::durable("store-audit").unwrap(),
            },
        )
        .unwrap();
    let started = engine
        .journal()
        .records()
        .iter()
        .find(|record| record.kind() == JournalRecordKind::Started)
        .unwrap();
    let completed = engine
        .journal()
        .records()
        .iter()
        .find(|record| record.kind() == JournalRecordKind::Completed)
        .unwrap();
    assert_eq!(started.effect_kind(), Some(EffectKind::ResourceStoreCreate));
    assert_eq!(completed.kind(), JournalRecordKind::Completed);
    assert_eq!(
        completed.identity(),
        Some(&ArtifactId::new("store-uuid").unwrap())
    );
}

#[test]
fn restart_reopens_the_verified_journal_and_resumes_after_last_durable_success() {
    let mut original = engine();
    let request = EffectRequest::new(
        EffectId::new("effect-repeat").unwrap(),
        StepId::new("step-repeat").unwrap(),
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    original.start_effect(request).unwrap();
    let journal_bytes = original.journal_bytes().unwrap();
    let binding = original.journal().binding().clone();
    let journal = Journal::from_bytes(binding, &journal_bytes).unwrap();
    let request = original.request().clone();
    let inventory = match request.inventory() {
        OperationInventory::Host(inventory) => inventory.clone(),
        OperationInventory::Reset(_) => panic!("cutover fixture must use host inventory"),
    };
    let preview = CutoverPreview::new(
        request.operation_id().clone(),
        OperationKind::Cutover,
        request.candidate_id().clone(),
        request.revision_plan_id().clone(),
        inventory,
        None,
    )
    .unwrap();
    let mut reopened = CutoverEngine::reopen(request, &preview, journal).unwrap();
    assert!(reopened.current_effect().is_some());
    assert_eq!(
        reopened.replay_decision(ReplayObservation::Absent).unwrap(),
        ReplayDecision::Repeat
    );
    let effect_id = reopened.current_effect().unwrap().effect_id().clone();
    reopened
        .complete_effect(
            &effect_id,
            CompletionEvidence {
                effect: EffectEvidence::succeeded(),
                audit: AuditEvidence::durable("replay-audit").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(reopened.phase(), CutoverPhase::Disposition);
}
