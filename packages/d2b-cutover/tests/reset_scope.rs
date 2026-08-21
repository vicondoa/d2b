use d2b_cutover::{
    ApplyContext, ArtifactId, AuditEvidence, CandidateId, Consent, ConsentBinding, CutoverPhase,
    CutoverPreview, Digest, Disposition, EffectAllowlist, EffectCapability, EffectId, EffectKind,
    EffectRequest, HostInventory, HostLockContract, InventoryClass, InventoryError,
    InventoryInputItem, InventoryItem, OperationInventory, OperationKind, OperationRequest,
    OperationState, OperatorId, ResetEngine, ResetError, ResetInventory, ResetScope,
    RevisionPlanId, StepId, ZoneId, ZoneInventory,
};

const NOW: u64 = 1_000;

fn digest(label: &str) -> Digest {
    Digest::derive("d2b:test", label.as_bytes())
}

fn inventory_in_order(reverse: bool) -> HostInventory {
    let mut zones = vec![
        ZoneInventory::new(
            "zone-a",
            true,
            [InventoryItem::classified(
                "zone-a-volume",
                InventoryClass::DurableVolume,
                Disposition::Adopt,
            )
            .unwrap()
            .into()],
        )
        .unwrap(),
        ZoneInventory::new(
            "zone-b",
            true,
            [InventoryItem::unclassified("unknown-b").unwrap().into()],
        )
        .unwrap(),
    ];
    if reverse {
        zones.reverse();
    }
    let mut configured = vec![
        ZoneId::new("zone-a").unwrap(),
        ZoneId::new("zone-b").unwrap(),
    ];
    if reverse {
        configured.reverse();
    }
    HostInventory::build(
        configured,
        zones,
        [
            InventoryItem::classified("host-key", InventoryClass::SshKey, Disposition::Preserve)
                .unwrap()
                .into(),
        ],
    )
    .unwrap()
}

#[test]
fn equivalent_all_zone_inventories_have_stable_preview_and_consent_digests() {
    let first_inventory = inventory_in_order(false);
    let second_inventory = inventory_in_order(true);
    assert_eq!(
        first_inventory.digest().unwrap(),
        second_inventory.digest().unwrap()
    );
    let first = CutoverPreview::new(
        d2b_cutover::OperationId::new("operation-1").unwrap(),
        OperationKind::Cutover,
        CandidateId::new("candidate-1").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        first_inventory,
        None,
    )
    .unwrap();
    let second = CutoverPreview::new(
        d2b_cutover::OperationId::new("operation-1").unwrap(),
        OperationKind::Cutover,
        CandidateId::new("candidate-1").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        second_inventory,
        None,
    )
    .unwrap();
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
    assert_eq!(
        first.canonical_bytes().unwrap(),
        second.canonical_bytes().unwrap()
    );
    assert_eq!(
        CutoverPreview::decode_json(&first.canonical_bytes().unwrap())
            .unwrap()
            .digest()
            .unwrap(),
        first.digest().unwrap()
    );
    let host = match first.inventory() {
        d2b_cutover::PreviewInventory::Host(host) => host,
        d2b_cutover::PreviewInventory::Reset(_) => panic!("cutover preview must be host-wide"),
    };
    assert!(HostInventory::decode_json(&host.canonical_bytes().unwrap()).is_ok());
}

#[test]
fn partial_zone_inventory_and_gateway_custody_refuse_before_preview() {
    let missing = HostInventory::build(
        [
            ZoneId::new("zone-a").unwrap(),
            ZoneId::new("zone-b").unwrap(),
        ],
        [ZoneInventory::empty("zone-a").unwrap()],
        [],
    );
    assert_eq!(missing, Err(InventoryError::PartialZoneInventory));

    let gateway = ZoneInventory::new(
        "zone-a",
        true,
        [InventoryInputItem::RealmGatewayCredentialAudit(
            ArtifactId::new("gateway-private").unwrap(),
        )],
    );
    assert_eq!(
        gateway,
        Err(InventoryError::RealmGatewayCredentialAuditForbidden)
    );
}

#[test]
fn reset_inventories_preserve_durable_volumes_and_have_distinct_effects() {
    for scope in [ResetScope::Zone, ResetScope::Provider, ResetScope::Guest] {
        let inventory = ResetInventory::new(scope, "target-1").unwrap();
        assert!(inventory.preserves_durable_volumes());
        assert!(!inventory.allows_destroy_durable_volumes());
        assert!(
            ResetInventory::new(scope, "target-1")
                .unwrap()
                .with_preserve_durable_volumes(false)
                .with_destroy_durable_consent(true)
                .allows_destroy_durable_volumes()
        );
        let kind = OperationKind::ScopedReset(scope);
        let allowlist = EffectAllowlist::for_operation(kind);
        assert!(!allowlist.permits(EffectKind::HostDrain));
        assert!(!allowlist.permits(EffectKind::ClosureActivation));
        assert!(!allowlist.permits(EffectKind::CutoverFinalization));
        assert!(!allowlist.permits(EffectKind::CutoverBroker));
        assert!(allowlist.permits(match scope {
            ResetScope::Zone => EffectKind::ScopedZoneReset,
            ResetScope::Provider => EffectKind::ScopedProviderReset,
            ResetScope::Guest => EffectKind::ScopedGuestReset,
        }));
    }

    let capability = EffectCapability::new(
        d2b_cutover::OperationId::new("reset-operation").unwrap(),
        OperatorId::new("operator-1").unwrap(),
        OperationKind::ScopedReset(ResetScope::Guest),
    );
    assert_eq!(
        capability.authorize(EffectKind::HostDrain),
        Err(ResetError::EffectNotAllowed(EffectKind::HostDrain))
    );
}

#[test]
fn cutover_capability_does_not_authorize_scoped_reset_effects() {
    let capability = EffectCapability::new(
        d2b_cutover::OperationId::new("cutover-operation").unwrap(),
        OperatorId::new("operator-1").unwrap(),
        OperationKind::Cutover,
    );
    assert_eq!(
        capability.authorize(EffectKind::ScopedGuestReset),
        Err(ResetError::EffectNotAllowed(EffectKind::ScopedGuestReset))
    );
    assert!(capability.authorize(EffectKind::HostDrain).is_ok());
}

#[test]
fn consent_is_single_use_and_cannot_transfer_to_a_new_operation() {
    let binding = ConsentBinding::new(
        d2b_cutover::OperationId::new("operation-1").unwrap(),
        OperationKind::Cutover,
        CandidateId::new("candidate-1").unwrap(),
        digest("preview"),
        Some(digest("recovery")),
        OperatorId::new("operator-1").unwrap(),
    );
    let mut consent = Consent::issue(binding.clone(), 100, 200).unwrap();
    consent.consume(&binding, 150).unwrap();
    assert!(consent.consume(&binding, 150).is_err());
    let other_operation = ConsentBinding::new(
        d2b_cutover::OperationId::new("operation-2").unwrap(),
        OperationKind::Cutover,
        CandidateId::new("candidate-1").unwrap(),
        digest("preview"),
        Some(digest("recovery")),
        OperatorId::new("operator-1").unwrap(),
    );
    assert!(consent.consume(&other_operation, 150).is_err());
}

#[test]
fn reset_request_cannot_be_constructed_from_host_wide_inventory() {
    let preview_inventory = inventory_in_order(false);
    let preview = CutoverPreview::new_reset(
        d2b_cutover::OperationId::new("reset-operation").unwrap(),
        OperationKind::ScopedReset(ResetScope::Guest),
        CandidateId::new("candidate-1").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        ResetInventory::new(ResetScope::Guest, "guest-1").unwrap(),
    )
    .unwrap();
    let reset_inventory = ResetInventory::new(ResetScope::Guest, "guest-1").unwrap();
    let request = OperationRequest::new(
        d2b_cutover::OperationId::new("reset-operation").unwrap(),
        OperationKind::ScopedReset(ResetScope::Guest),
        CandidateId::new("candidate-1").unwrap(),
        RevisionPlanId::new("revision-1").unwrap(),
        OperatorId::new("operator-1").unwrap(),
        preview.digest().unwrap(),
        None,
        OperationInventory::Host(preview_inventory),
    );
    assert!(request.is_err());
    assert!(
        OperationRequest::new_reset(
            d2b_cutover::OperationId::new("reset-operation").unwrap(),
            ResetScope::Guest,
            CandidateId::new("candidate-1").unwrap(),
            RevisionPlanId::new("revision-1").unwrap(),
            OperatorId::new("operator-1").unwrap(),
            digest("reset-preview"),
            reset_inventory,
        )
        .is_ok()
    );
}

#[test]
fn default_reset_cannot_destroy_durable_volume_without_separate_consent() {
    let inventory = ResetInventory::new(ResetScope::Guest, "guest-1").unwrap();
    let operation_id = d2b_cutover::OperationId::new("reset-operation").unwrap();
    let candidate = CandidateId::new("candidate-1").unwrap();
    let revision = RevisionPlanId::new("revision-1").unwrap();
    let operator = OperatorId::new("operator-1").unwrap();
    let preview = CutoverPreview::new_reset(
        operation_id.clone(),
        OperationKind::ScopedReset(ResetScope::Guest),
        candidate.clone(),
        revision.clone(),
        inventory.clone(),
    )
    .unwrap();
    let request = OperationRequest::new_reset(
        operation_id,
        ResetScope::Guest,
        candidate,
        revision,
        operator,
        preview.digest().unwrap(),
        inventory,
    )
    .unwrap();
    let mut engine = ResetEngine::new(request, &preview).unwrap();
    let mut lock = HostLockContract::new();
    engine.acquire_host_lock(&mut lock).unwrap();
    let inventory_digest = match engine.request().inventory() {
        d2b_cutover::OperationInventory::Reset(inventory) => inventory.digest().unwrap(),
        d2b_cutover::OperationInventory::Host(_) => panic!("reset must use reset inventory"),
    };
    let context = ApplyContext::reset(NOW, inventory_digest, true, true, true);
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
    let destroy = EffectRequest::new(
        EffectId::new("destroy-volume").unwrap(),
        StepId::new("destroy-volume-step").unwrap(),
        EffectKind::DestroyDurableVolume,
        d2b_cutover::ReplayClass::QuarantineOnly,
        None,
    );
    assert_eq!(
        engine.start_effect(destroy),
        Err(d2b_cutover::OperationError::EffectNotAllowed(
            EffectKind::DestroyDurableVolume
        ))
    );
    assert_eq!(
        engine.state(),
        OperationState::Applying(CutoverPhase::Drain)
    );
}
