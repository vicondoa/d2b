use d2b_cutover::{
    ApplyContext, ArtifactId, AuditEvidence, CandidateId, CompletionEvidence, Consent,
    CutoverEngine, CutoverPhase, CutoverPreview, Digest, EffectEvidence, EffectId, EffectKind,
    EffectRequest, FailureCode, FinalizationConsent, HoldReason, HoldState, HostInventory,
    HostLockContract, Journal, JournalBinding, JournalError, JournalRecordKind, OperationId,
    OperationInventory, OperationKind, OperationRequest, OperationState, OperatorId,
    RecoveryAttestation, RecoveryId, ReplayClass, ReplayDecision, ReplayObservation,
    RevisionPlanId, StepId, VerificationInput, ZoneId, ZoneInventory, ZoneVerification,
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

#[test]
fn restart_after_verification_and_finalization_consent_preserves_finalizing_state() {
    let mut original = engine();
    for (name, kind, replay_class, identity, advance_to) in [
        (
            "host-drain",
            EffectKind::HostDrain,
            ReplayClass::Repeatable,
            None,
            Some(CutoverPhase::Disposition),
        ),
        (
            "closure-activation",
            EffectKind::ClosureActivation,
            ReplayClass::ReopenByJournaledIdentity,
            Some(ArtifactId::new("system-artifact").unwrap()),
            Some(CutoverPhase::ResourceStore),
        ),
        (
            "resource-store",
            EffectKind::ResourceStoreCreate,
            ReplayClass::ReopenByJournaledIdentity,
            Some(ArtifactId::new("store-identity").unwrap()),
            Some(CutoverPhase::ProviderInstall),
        ),
        (
            "provider-install",
            EffectKind::ProviderInstall,
            ReplayClass::Repeatable,
            None,
            Some(CutoverPhase::ZoneCutover),
        ),
        (
            "zone-activation",
            EffectKind::ZoneActivation,
            ReplayClass::Repeatable,
            None,
            Some(CutoverPhase::Activation),
        ),
        (
            "guest-activation",
            EffectKind::GuestActivation,
            ReplayClass::Repeatable,
            None,
            Some(CutoverPhase::Verification),
        ),
    ] {
        let mut effect = EffectRequest::new(
            EffectId::new(format!("effect-{name}")).unwrap(),
            StepId::new(format!("step-{name}")).unwrap(),
            kind,
            replay_class,
            advance_to,
        );
        if let Some(identity) = identity {
            effect = effect.with_identity(Some(identity), None);
        }
        let effect_id = effect.effect_id().clone();
        let observed_identity = effect.journaled_identity().cloned();
        original.start_effect(effect).unwrap();
        original
            .complete_effect(
                &effect_id,
                CompletionEvidence {
                    effect: match observed_identity {
                        Some(identity) => {
                            EffectEvidence::succeeded_with_identity(identity.as_str()).unwrap()
                        }
                        None => EffectEvidence::succeeded(),
                    },
                    audit: AuditEvidence::durable(format!("audit-{name}")).unwrap(),
                },
            )
            .unwrap();
    }
    original
        .verify(&VerificationInput::new(
            [ZoneVerification::new(ZoneId::new("zone-a").unwrap(), true)],
            true,
            true,
            true,
            true,
        ))
        .unwrap();
    assert_eq!(original.state(), OperationState::CutoverSucceeded);
    let mut consent =
        FinalizationConsent::issue(original.request().finalization_binding(), 1_000, 1_900)
            .unwrap();
    original.begin_finalization(&mut consent, 1_100).unwrap();
    assert_eq!(original.state(), OperationState::Finalizing);

    let finalization = EffectRequest::new(
        EffectId::new("effect-finalization").unwrap(),
        StepId::new("step-finalization").unwrap(),
        EffectKind::CutoverFinalization,
        ReplayClass::QuarantineOnly,
        None,
    );
    original.start_effect(finalization).unwrap();
    let journal = Journal::from_bytes(
        original.journal().binding().clone(),
        &original.journal_bytes().unwrap(),
    )
    .unwrap();
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
    assert_eq!(reopened.state(), OperationState::Finalizing);
    let effect_id = reopened.current_effect().unwrap().effect_id().clone();
    reopened
        .complete_effect(
            &effect_id,
            CompletionEvidence {
                effect: EffectEvidence::succeeded(),
                audit: AuditEvidence::durable("finalization-audit").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(reopened.state(), OperationState::Closed);
}

#[test]
fn retryable_effect_failures_allow_multiple_attempts() {
    let mut operation = engine();
    let effect = EffectRequest::new(
        EffectId::new("effect-retry").unwrap(),
        StepId::new("step-retry").unwrap(),
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    operation.start_effect(effect.clone()).unwrap();
    operation.retry_effect(FailureCode::EffectFailed).unwrap();
    operation.start_effect(effect.clone()).unwrap();
    operation.retry_effect(FailureCode::EffectFailed).unwrap();
    assert!(operation.current_effect().is_none());

    let journal = Journal::from_bytes(
        operation.journal().binding().clone(),
        &operation.journal_bytes().unwrap(),
    )
    .unwrap();
    let request = operation.request().clone();
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
    assert!(reopened.current_effect().is_none());
    reopened.start_effect(effect).unwrap();
}

#[test]
fn reopen_keeps_started_destination_for_native_rollback() {
    let mut operation = engine();
    let drain = EffectRequest::new(
        EffectId::new("effect-drain-destination").unwrap(),
        StepId::new("step-drain-destination").unwrap(),
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    operation.start_effect(drain.clone()).unwrap();
    operation
        .complete_effect(
            drain.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded(),
                audit: AuditEvidence::durable("drain-audit").unwrap(),
            },
        )
        .unwrap();
    let store = EffectRequest::new(
        EffectId::new("effect-store-destination").unwrap(),
        StepId::new("step-store-destination").unwrap(),
        EffectKind::ResourceStoreCreate,
        ReplayClass::ReopenByJournaledIdentity,
        Some(CutoverPhase::ProviderInstall),
    )
    .with_identity(
        Some(ArtifactId::new("store-identity").unwrap()),
        Some(ArtifactId::new("store-destination").unwrap()),
    );
    operation.start_effect(store).unwrap();
    let journal = Journal::from_bytes(
        operation.journal().binding().clone(),
        &operation.journal_bytes().unwrap(),
    )
    .unwrap();
    let request = operation.request().clone();
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
    let reopened = CutoverEngine::reopen(request, &preview, journal).unwrap();
    assert_eq!(
        reopened
            .current_effect()
            .and_then(|effect| effect.destination()),
        Some(&ArtifactId::new("store-destination").unwrap())
    );
    assert_eq!(
        reopened.staged_destinations().collect::<Vec<_>>(),
        [&ArtifactId::new("store-destination").unwrap()]
    );
}

#[test]
fn pending_hold_reopens_as_pending_while_effect_is_in_flight() {
    let mut operation = engine();
    let effect = EffectRequest::new(
        EffectId::new("effect-hold-reopen").unwrap(),
        StepId::new("step-hold-reopen").unwrap(),
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    operation.start_effect(effect).unwrap();
    operation
        .request_hold(
            OperatorId::new("operator-1").unwrap(),
            HoldReason::new("pause-before-restart").unwrap(),
            AuditEvidence::durable("hold-audit").unwrap(),
        )
        .unwrap();
    let journal = Journal::from_bytes(
        operation.journal().binding().clone(),
        &operation.journal_bytes().unwrap(),
    )
    .unwrap();
    let request = operation.request().clone();
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
    let reopened = CutoverEngine::reopen(request, &preview, journal).unwrap();
    assert!(matches!(reopened.hold(), HoldState::Pending { .. }));
    assert_eq!(
        reopened.state(),
        OperationState::Applying(CutoverPhase::Drain)
    );
    assert!(reopened.current_effect().is_some());
}
