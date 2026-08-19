use d2b_cutover::{
    ApplyContext, ArtifactId, AuditEvidence, CandidateId, CompletionEvidence, Consent,
    CutoverEngine, CutoverPhase, CutoverPreview, Digest, Disposition, EffectEvidence, EffectId,
    EffectKind, EffectRequest, FailureCode, FinalizationConsent, HoldReason, HostInventory,
    HostLockContract, InventoryClass, InventoryItem, OperationRequest, OperationState, OperatorId,
    RecoveryAttestation, RecoveryId, ReplayClass, RevisionPlanId, StepId, VerificationInput,
    ZoneId, ZoneInventory, ZoneVerification,
};

const NOW: u64 = 1_000;

fn digest(label: &str) -> Digest {
    Digest::derive("d2b:test", label.as_bytes())
}

fn id<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, d2b_cutover::IdError>) -> T {
    constructor(value.to_owned()).unwrap()
}

fn inventory() -> HostInventory {
    let zone_a = ZoneInventory::new(
        "zone-a",
        true,
        [InventoryItem::classified(
            "zone-a-tpm",
            InventoryClass::TpmIdentity,
            Disposition::Adopt,
        )
        .unwrap()
        .into()],
    )
    .unwrap();
    let zone_b = ZoneInventory::new(
        "zone-b",
        true,
        [InventoryItem::unclassified("zone-b-unknown")
            .unwrap()
            .into()],
    )
    .unwrap();
    HostInventory::build(
        [
            ZoneId::new("zone-b").unwrap(),
            ZoneId::new("zone-a").unwrap(),
        ],
        [zone_b, zone_a],
        [InventoryItem::classified(
            "host-volume",
            InventoryClass::DurableVolume,
            Disposition::Adopt,
        )
        .unwrap()
        .into()],
    )
    .unwrap()
}

struct Fixture {
    engine: CutoverEngine,
    context: ApplyContext,
}

fn fixture() -> Fixture {
    let inventory = inventory();
    let inventory_digest = inventory.digest().unwrap();
    let operation_id = id("operation-1", d2b_cutover::OperationId::new);
    let candidate_id = id("candidate-1", CandidateId::new);
    let revision = id("revision-1", RevisionPlanId::new);
    let operator = id("operator-1", OperatorId::new);
    let preview = CutoverPreview::new(
        operation_id.clone(),
        d2b_cutover::OperationKind::Cutover,
        candidate_id.clone(),
        revision.clone(),
        inventory.clone(),
        None,
    )
    .unwrap();
    let preview_digest = preview.digest().unwrap();
    let recovery = RecoveryAttestation::new(
        id("recovery-1", RecoveryId::new),
        candidate_id.clone(),
        digest("host"),
        preview_digest.clone(),
        operator.clone(),
        digest("restore"),
        900,
        1_900,
        true,
    )
    .unwrap();
    let recovery_digest = recovery.digest().unwrap();
    let request = OperationRequest::new_cutover(
        operation_id,
        candidate_id,
        revision,
        operator,
        preview_digest,
        recovery_digest,
        inventory,
    )
    .unwrap();
    let mut engine = CutoverEngine::new(request, &preview).unwrap();
    let mut lock = HostLockContract::new();
    engine.acquire_host_lock(&mut lock).unwrap();
    let context = ApplyContext::cutover(
        NOW,
        inventory_digest.clone(),
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
    Fixture { engine, context }
}

fn effect(
    number: &str,
    kind: EffectKind,
    replay: ReplayClass,
    advance_to: Option<CutoverPhase>,
) -> EffectRequest {
    EffectRequest::new(
        EffectId::new(format!("effect-{number}")).unwrap(),
        StepId::new(format!("step-{number}")).unwrap(),
        kind,
        replay,
        advance_to,
    )
}

fn complete(engine: &mut CutoverEngine, request: &EffectRequest) {
    engine
        .complete_effect(
            request.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded(),
                audit: AuditEvidence::durable(format!("audit-{}", request.effect_id())).unwrap(),
            },
        )
        .unwrap();
}

#[test]
fn u3_exposes_closed_phase_and_operation_contracts() {
    assert_eq!(CutoverPhase::Preflight.number(), 0);
    assert_eq!(CutoverPhase::Finalization.number(), 10);
}

#[test]
fn all_zone_inventory_is_sorted_and_unknown_items_preserve() {
    let inventory = inventory();
    assert_eq!(
        inventory.zone_ids().map(ZoneId::as_str).collect::<Vec<_>>(),
        ["zone-a", "zone-b"]
    );
    assert_eq!(
        inventory
            .zones()
            .iter()
            .find(|zone| zone.zone_id().as_str() == "zone-b")
            .unwrap()
            .items()[0]
            .disposition(),
        Disposition::Preserve
    );
    assert!(inventory.sources_retained());
}

#[test]
fn lock_contention_refuses_the_second_operation_without_mutation() {
    let fixture = fixture();
    let mut first_lock = HostLockContract::new();
    let mut first = fixture.engine;
    first.acquire_host_lock(&mut first_lock).unwrap();

    let inventory = inventory();
    let preview = CutoverPreview::new(
        d2b_cutover::OperationId::new("operation-2").unwrap(),
        d2b_cutover::OperationKind::Cutover,
        CandidateId::new("candidate-2").unwrap(),
        RevisionPlanId::new("revision-2").unwrap(),
        inventory.clone(),
        None,
    )
    .unwrap();
    let request = OperationRequest::new(
        d2b_cutover::OperationId::new("operation-2").unwrap(),
        d2b_cutover::OperationKind::Cutover,
        CandidateId::new("candidate-2").unwrap(),
        RevisionPlanId::new("revision-2").unwrap(),
        OperatorId::new("operator-2").unwrap(),
        preview.digest().unwrap(),
        Some(digest("recovery-2")),
        d2b_cutover::OperationInventory::Host(inventory),
    )
    .unwrap();
    let mut second = CutoverEngine::new(request, &preview).unwrap();
    assert!(matches!(
        second.acquire_host_lock(&mut first_lock),
        Err(d2b_cutover::OperationError::LockContended(_))
    ));
    assert!(second.journal().records().is_empty());
    assert!(first.current_effect().is_none());
}

#[test]
fn hold_waits_for_current_atomic_step_and_resume_reuses_operation() {
    let mut fixture = fixture();
    let request = effect(
        "drain",
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    fixture.engine.start_effect(request.clone()).unwrap();
    fixture
        .engine
        .request_hold(
            OperatorId::new("other-admin").unwrap(),
            HoldReason::new("pause for inspection").unwrap(),
            AuditEvidence::durable("hold-request").unwrap(),
        )
        .unwrap();
    assert!(!fixture.engine.hold().is_active());
    complete(&mut fixture.engine, &request);
    assert_eq!(fixture.engine.state(), OperationState::Held);
    let bound_operator = fixture.engine.request().operator_id().clone();
    fixture
        .engine
        .resume(
            &bound_operator,
            &fixture.context,
            AuditEvidence::durable("hold-clear").unwrap(),
        )
        .unwrap();
    assert_eq!(
        fixture.engine.state(),
        OperationState::Applying(CutoverPhase::Disposition)
    );
}

#[test]
fn audit_failure_does_not_advance_or_clear_started_effect() {
    let mut fixture = fixture();
    let request = effect(
        "drain",
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    fixture.engine.start_effect(request.clone()).unwrap();
    assert_eq!(
        fixture.engine.complete_effect(
            request.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded(),
                audit: AuditEvidence::unavailable(),
            },
        ),
        Err(d2b_cutover::OperationError::AuditNotDurable)
    );
    assert_eq!(fixture.engine.phase(), CutoverPhase::Drain);
    assert!(fixture.engine.current_effect().is_some());
}

#[test]
fn phase_four_rollback_preserves_sources_and_quarantines_staged_destination() {
    let mut fixture = fixture();
    let drain = effect(
        "drain",
        EffectKind::HostDrain,
        ReplayClass::Repeatable,
        Some(CutoverPhase::Disposition),
    );
    fixture.engine.start_effect(drain.clone()).unwrap();
    complete(&mut fixture.engine, &drain);
    let disposition = effect(
        "disposition",
        EffectKind::CutoverDisposition,
        ReplayClass::ReopenByJournaledIdentity,
        None,
    )
    .with_identity(
        Some(ArtifactId::new("journaled-store").unwrap()),
        Some(ArtifactId::new("staged-store").unwrap()),
    );
    fixture.engine.start_effect(disposition.clone()).unwrap();
    fixture
        .engine
        .complete_effect(
            disposition.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded_with_identity("journaled-store").unwrap(),
                audit: AuditEvidence::durable("disposition-audit").unwrap(),
            },
        )
        .unwrap();
    let rollback = fixture
        .engine
        .rollback(AuditEvidence::durable("rollback-audit").unwrap())
        .unwrap();
    assert!(rollback.sources_preserved());
    assert_eq!(
        rollback.quarantined_destinations(),
        &[ArtifactId::new("staged-store").unwrap()]
    );
    assert_eq!(fixture.engine.state(), OperationState::RolledBack);
}

#[test]
fn phase_five_rollback_is_refused() {
    let mut fixture = fixture();
    let request = effect(
        "store",
        EffectKind::ResourceStoreCreate,
        ReplayClass::ReopenByJournaledIdentity,
        Some(CutoverPhase::ProviderInstall),
    )
    .with_identity(
        Some(ArtifactId::new("store-uuid").unwrap()),
        Some(ArtifactId::new("store-destination").unwrap()),
    );
    fixture.engine.start_effect(request.clone()).unwrap();
    fixture
        .engine
        .complete_effect(
            request.effect_id(),
            CompletionEvidence {
                effect: EffectEvidence::succeeded_with_identity("store-uuid").unwrap(),
                audit: AuditEvidence::durable("store-audit").unwrap(),
            },
        )
        .unwrap();
    assert!(matches!(
        fixture
            .engine
            .rollback(AuditEvidence::durable("rollback-audit").unwrap()),
        Err(d2b_cutover::OperationError::Rollback(
            d2b_cutover::RollbackError::BoundaryClosed(CutoverPhase::ProviderInstall)
        ))
    ));
}

#[test]
fn terminal_outcome_is_write_once() {
    let mut fixture = fixture();
    fixture
        .engine
        .fail_terminal(
            FailureCode::EffectFailed,
            AuditEvidence::durable("failure-audit").unwrap(),
        )
        .unwrap();
    assert_eq!(fixture.engine.state(), OperationState::Failed);
    assert_eq!(
        fixture.engine.fail_terminal(
            FailureCode::CandidateDrift,
            AuditEvidence::durable("second").unwrap(),
        ),
        Err(d2b_cutover::OperationError::TerminalAlreadyWritten)
    );
}

#[test]
fn phase_ten_requires_distinct_consent_and_closes_once() {
    let mut fixture = fixture();
    for ((number, kind), next) in [
        (("drain", EffectKind::HostDrain), CutoverPhase::Disposition),
        (
            ("disposition", EffectKind::CutoverDisposition),
            CutoverPhase::ResourceStore,
        ),
        (
            ("store", EffectKind::ResourceStoreCreate),
            CutoverPhase::ProviderInstall,
        ),
        (
            ("provider", EffectKind::ProviderInstall),
            CutoverPhase::ZoneCutover,
        ),
        (
            ("zone", EffectKind::ZoneActivation),
            CutoverPhase::Activation,
        ),
        (
            ("guest", EffectKind::GuestActivation),
            CutoverPhase::Verification,
        ),
    ] {
        let request = effect(
            number,
            kind,
            if kind == EffectKind::ResourceStoreCreate {
                ReplayClass::ReopenByJournaledIdentity
            } else {
                ReplayClass::Repeatable
            },
            Some(next),
        );
        let request = if kind == EffectKind::ResourceStoreCreate {
            request.with_identity(
                Some(ArtifactId::new("store-uuid").unwrap()),
                Some(ArtifactId::new("store-destination").unwrap()),
            )
        } else {
            request
        };
        fixture.engine.start_effect(request.clone()).unwrap();
        let evidence = if request.identity_bearing() {
            EffectEvidence::succeeded_with_identity("store-uuid").unwrap()
        } else {
            EffectEvidence::succeeded()
        };
        fixture
            .engine
            .complete_effect(
                request.effect_id(),
                CompletionEvidence {
                    effect: evidence,
                    audit: AuditEvidence::durable(format!("audit-{number}")).unwrap(),
                },
            )
            .unwrap();
    }
    fixture
        .engine
        .verify(&VerificationInput::new(
            [
                ZoneVerification::new(ZoneId::new("zone-a").unwrap(), true),
                ZoneVerification::new(ZoneId::new("zone-b").unwrap(), true),
            ],
            true,
            true,
            true,
            true,
        ))
        .unwrap();
    let mut finalization = FinalizationConsent::issue(
        fixture.engine.request().finalization_binding(),
        NOW,
        NOW + 500,
    )
    .unwrap();
    fixture
        .engine
        .begin_finalization(&mut finalization, NOW + 1)
        .unwrap();
    let request = effect(
        "finalize",
        EffectKind::CutoverFinalization,
        ReplayClass::QuarantineOnly,
        None,
    );
    fixture.engine.start_effect(request.clone()).unwrap();
    complete(&mut fixture.engine, &request);
    assert_eq!(fixture.engine.state(), OperationState::Closed);
    assert_eq!(
        fixture.engine.terminal_outcome(),
        Some(d2b_cutover::TerminalOutcomeKind::Closed)
    );
}

#[test]
fn verification_requires_every_zone_and_preserved_identity_before_success() {
    let mut fixture = fixture();
    for ((number, kind), next) in [
        (("drain", EffectKind::HostDrain), CutoverPhase::Disposition),
        (
            ("disposition", EffectKind::CutoverDisposition),
            CutoverPhase::ResourceStore,
        ),
        (
            ("store", EffectKind::ResourceStoreCreate),
            CutoverPhase::ProviderInstall,
        ),
        (
            ("provider", EffectKind::ProviderInstall),
            CutoverPhase::ZoneCutover,
        ),
        (
            ("zone", EffectKind::ZoneActivation),
            CutoverPhase::Activation,
        ),
        (
            ("guest", EffectKind::GuestActivation),
            CutoverPhase::Verification,
        ),
    ] {
        let request = effect(
            number,
            kind,
            if kind == EffectKind::ResourceStoreCreate {
                ReplayClass::ReopenByJournaledIdentity
            } else {
                ReplayClass::Repeatable
            },
            Some(next),
        );
        let request = if kind == EffectKind::ResourceStoreCreate {
            request.with_identity(
                Some(ArtifactId::new("store-uuid").unwrap()),
                Some(ArtifactId::new("store-destination").unwrap()),
            )
        } else {
            request
        };
        fixture.engine.start_effect(request.clone()).unwrap();
        if request.identity_bearing() {
            fixture
                .engine
                .complete_effect(
                    request.effect_id(),
                    CompletionEvidence {
                        effect: EffectEvidence::succeeded_with_identity("store-uuid").unwrap(),
                        audit: AuditEvidence::durable(format!("audit-{number}")).unwrap(),
                    },
                )
                .unwrap();
        } else {
            complete(&mut fixture.engine, &request);
        }
    }
    let report = fixture
        .engine
        .verify(&VerificationInput::new(
            [
                ZoneVerification::new(ZoneId::new("zone-a").unwrap(), true),
                ZoneVerification::new(ZoneId::new("zone-b").unwrap(), true),
            ],
            true,
            true,
            true,
            true,
        ))
        .unwrap();
    assert_eq!(report.zone_count(), 2);
    assert_eq!(fixture.engine.state(), OperationState::CutoverSucceeded);
}
