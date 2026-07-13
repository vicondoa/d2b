#![cfg(feature = "v2-state")]

use d2b_contracts::{
    v2_identity::{RealmId, RoleId},
    v2_state::*,
};
use schemars::schema_for;
use serde_json::{Value, json};

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const ONE_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TWO_DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn fixture() -> StateStorageSyncAuditContract {
    serde_json::from_str(include_str!(
        "../../../docs/reference/state-storage-sync-audit-v2-fixture.json"
    ))
    .expect("v2 state fixture must deserialize")
}

fn realm_id() -> RealmId {
    RealmId::parse("yl2hpmks5td5dkeso6qq").unwrap()
}

fn role_id() -> RoleId {
    RoleId::parse("7xrbjonser3hpi7hqojq").unwrap()
}

fn digest(value: &str) -> Digest {
    Digest::parse(value).unwrap()
}

fn realm_stream() -> AuditStream {
    AuditStream::Realm {
        realm_id: realm_id(),
    }
}

fn realm_owner() -> AuditOwner {
    AuditOwner::RealmBroker {
        realm_id: realm_id(),
    }
}

fn audit_record(sequence: u64, previous_hash: &str, record_hash: &str) -> AuditRecord {
    AuditRecord {
        schema_version: STATE_SCHEMA_VERSION,
        stream: realm_stream(),
        sequence,
        occurred_at_unix_ms: 10_000 + sequence,
        correlation: AuditCorrelation {
            operation_id: CorrelationId::parse(format!("operation-{sequence}")).unwrap(),
            session_id: Some(CorrelationId::parse("session-1").unwrap()),
            provider_id: None,
        },
        actor: AuditActor::RealmBroker {
            realm_id: realm_id(),
        },
        event: AuditEvent::StorageReconcile,
        outcome: AuditOutcome::Succeeded,
        reason: AuditReason::IdentityVerified,
        previous_hash: digest(previous_hash),
        record_hash: digest(record_hash),
        encoded_bytes: 512,
    }
}

fn audit_segment() -> AuditSegment {
    AuditSegment {
        summary: AuditSegmentSummary {
            stream: realm_stream(),
            owner: realm_owner(),
            segment_id: ResourceId::parse("segment-1").unwrap(),
            first_sequence: 1,
            last_sequence: 2,
            previous_segment_digest: digest(ZERO_DIGEST),
            segment_digest: digest(TWO_DIGEST),
            controller_generation: Generation::new(7).unwrap(),
            created_at_unix_ms: 10_000,
            sealed_at_unix_ms: 20_000,
            encoded_bytes: 1_024,
            prune_status: PruneStatus::EligibleAfterCheckpoint,
        },
        records: vec![
            audit_record(1, ZERO_DIGEST, ONE_DIGEST),
            audit_record(2, ONE_DIGEST, TWO_DIGEST),
        ],
    }
}

#[test]
fn fixture_is_complete_and_fingerprint_bound() {
    let contract = fixture();
    contract.validate().expect("complete fixture validates");
    assert_eq!(contract.storage.resources.len(), StorageCategory::ALL.len());
    assert_eq!(contract.synchronization.locks.len(), 2);
    assert_eq!(contract.audit.streams.len(), 2);

    let categories = contract
        .storage
        .resources
        .iter()
        .map(|resource| resource.category)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        categories,
        StorageCategory::ALL.into_iter().collect(),
        "every required inventory category is represented"
    );

    let mut mismatch = contract.clone();
    mismatch.audit.contract_fingerprint = digest(TWO_DIGEST);
    assert_eq!(
        mismatch.validate(),
        Err(StateContractError::ContractFingerprintMismatch)
    );
}

#[test]
fn schema_and_serde_are_closed_and_bounded() {
    let schema = serde_json::to_value(schema_for!(StateStorageSyncAuditContract)).unwrap();
    assert_eq!(
        schema
            .pointer("/additionalProperties")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        schema
            .pointer("/definitions/StorageResource/additionalProperties")
            .and_then(Value::as_bool),
        Some(false)
    );

    let mut value: Value = serde_json::from_str(include_str!(
        "../../../docs/reference/state-storage-sync-audit-v2-fixture.json"
    ))
    .unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("futureField".into(), json!(true));
    assert!(serde_json::from_value::<StateStorageSyncAuditContract>(value).is_err());

    assert!(Generation::new(0).is_err());
    assert!(Generation::new(MAX_SAFE_JSON_INTEGER + 1).is_err());
    assert!(ResourceId::parse("a").is_ok());
    assert!(ResourceId::parse("a".repeat(MAX_OPAQUE_ID_BYTES)).is_ok());
    assert!(ResourceId::parse("a".repeat(MAX_OPAQUE_ID_BYTES + 1)).is_err());
    assert!(Digest::parse("A".repeat(64)).is_err());
}

#[test]
fn raw_names_and_paths_have_no_serialized_entry_point() {
    for rejected in [
        "/run/d2b/r/raw-name",
        "../realm",
        "configured_provider",
        "device:1-2",
        "UpperName",
        "é",
    ] {
        assert!(ResourceId::parse(rejected).is_err());
        assert!(CorrelationId::parse(rejected).is_err());
    }

    let mut value = serde_json::to_value(fixture()).unwrap();
    value["storage"]["resources"][0]["path"] = json!("/var/lib/d2b/raw");
    assert!(serde_json::from_value::<StateStorageSyncAuditContract>(value).is_err());

    let serialized = serde_json::to_string(&fixture()).unwrap();
    assert!(!serialized.contains("/run/"));
    assert!(!serialized.contains("/var/"));
    assert!(!serialized.contains("configuredProvider"));
    assert!(!serialized.contains("deviceId"));
}

#[test]
fn storage_requires_unique_ids_categories_and_single_repair_owner() {
    let mut contract = fixture();
    contract
        .storage
        .resources
        .push(contract.storage.resources[0].clone());
    assert_eq!(
        contract.storage.validate(),
        Err(StateContractError::DuplicateResourceId)
    );

    let mut contract = fixture();
    contract
        .storage
        .resources
        .retain(|resource| resource.category != StorageCategory::Projection);
    assert_eq!(
        contract.storage.validate(),
        Err(StateContractError::MissingInventoryCategory)
    );

    let mut contract = fixture();
    contract.storage.resources[0].repair_authority = AuthorityRef::Pid1;
    assert_eq!(
        contract.storage.validate(),
        Err(StateContractError::RepairAuthorityMismatch)
    );
}

#[test]
fn atomic_write_state_machine_has_only_prior_or_complete_outcomes() {
    for phases in AtomicWritePhase::ALL.windows(2) {
        assert_eq!(phases[0].transition(phases[1]), Ok(phases[1]));
    }
    assert_eq!(
        AtomicWritePhase::TemporaryCreated.transition(AtomicWritePhase::Renamed),
        Err(StateContractError::InvalidAtomicTransition)
    );
    assert_eq!(
        AtomicWritePhase::Renamed.crash_outcomes(),
        &[
            CrashRecoveryOutcome::PriorDocument,
            CrashRecoveryOutcome::CompleteNewDocument
        ]
    );
    assert_eq!(
        AtomicWritePhase::ParentDirectorySynced.crash_outcomes(),
        &[CrashRecoveryOutcome::CompleteNewDocument]
    );

    for phase in AtomicWritePhase::ALL {
        let receipt = AtomicWriteReceipt {
            resource_id: ResourceId::parse("controller-state").unwrap(),
            generation: Generation::new(2).unwrap(),
            phase,
            checksum: digest(ZERO_DIGEST),
            success: true,
        };
        assert_eq!(
            receipt.validate().is_ok(),
            phase == AtomicWritePhase::ParentDirectorySynced
        );
    }
}

#[test]
fn state_envelopes_enforce_header_bounds_and_monotonic_generation() {
    let envelope = StateEnvelope {
        schema_version: STATE_SCHEMA_VERSION,
        schema_generation: STATE_SCHEMA_GENERATION,
        config_generation: Generation::new(4).unwrap(),
        state_generation: Generation::new(9).unwrap(),
        writer: AuthorityRef::RealmController {
            realm_id: realm_id(),
        },
        encoded_bytes: MAX_JSON_DOCUMENT_BYTES,
        checksum: digest(ZERO_DIGEST),
        payload: json!({"status": "ready"}),
    };
    assert_eq!(envelope.validate_header(), Ok(()));
    assert_eq!(envelope.next_generation().unwrap().get(), 10);

    let mut oversized = envelope;
    oversized.encoded_bytes += 1;
    assert_eq!(
        oversized.validate_header(),
        Err(StateContractError::BoundExceeded)
    );
}

#[test]
fn restart_adoption_rejects_ambiguity_and_requires_recovery_first() {
    let observation = RunnerObservation {
        observation_id: ResourceId::parse("runner-observation").unwrap(),
        scope: IdentityScope::Role {
            realm_id: realm_id(),
            workload_id: d2b_contracts::v2_identity::WorkloadId::parse("q5h7jtqteem7kua4tfva")
                .unwrap(),
            role_id: role_id(),
        },
        observed_pid: 42,
        evidence: RunnerEvidence {
            role_id: role_id(),
            candidate_count: 2,
            pidfd_persistence: PidfdPersistence::ProcessLocalNonPersistent,
            identity: EvidenceVerdict::Ambiguous,
            cgroup_membership: EvidenceVerdict::Match,
            executable_fingerprint: digest(ZERO_DIGEST),
            executable: EvidenceVerdict::Match,
            configuration_fingerprint: digest(ONE_DIGEST),
            configuration: EvidenceVerdict::Match,
            config_generation: Generation::new(3).unwrap(),
            generation: EvidenceVerdict::Match,
        },
    };
    let adopt = RestartDecision {
        observation_id: observation.observation_id.clone(),
        ordering: RecoveryOrdering::RecoverBeforeCleanup,
        recovery_completed: true,
        decision: AdoptionDecision::Adopt {
            fresh_pidfd_opened: true,
        },
    };
    assert_eq!(
        adopt.validate_for_runner(&observation),
        Err(StateContractError::RestartAmbiguous)
    );

    let cleanup = RestartDecision {
        observation_id: observation.observation_id.clone(),
        ordering: RecoveryOrdering::RecoverBeforeCleanup,
        recovery_completed: false,
        decision: AdoptionDecision::Cleanup {
            target: CleanupTarget::Resource {
                resource_id: ResourceId::parse("role-runtime").unwrap(),
            },
            owner_absence_proof: OwnerAbsenceProof::EmptyDeclaredCgroup,
        },
    };
    assert_eq!(
        cleanup.validate_for_runner(&observation),
        Err(StateContractError::CleanupBeforeRecovery)
    );

    let serialized = serde_json::to_string(&observation).unwrap();
    assert!(serialized.contains("process-local-non-persistent"));
    assert!(!serialized.contains("pidfd\":"));
    assert!(!serialized.contains("executablePath"));
    assert!(!serialized.contains("cgroupPath"));
}

#[test]
fn lock_order_is_total_and_cycles_fail_closed() {
    fixture()
        .synchronization
        .validate()
        .expect("fixture lock order validates");

    let mut cycle = fixture().synchronization;
    cycle.locks[0].acquire_after = vec![cycle.locks[1].lock_id.clone()];
    assert_eq!(cycle.validate(), Err(StateContractError::LockOrderCycle));

    let mut duplicate_order = fixture().synchronization;
    duplicate_order.locks[1].global_order = duplicate_order.locks[0].global_order;
    assert_eq!(
        duplicate_order.validate(),
        Err(StateContractError::DuplicateLockOrder)
    );

    let mut inheritable = fixture().synchronization;
    inheritable.locks[0].cloexec = false;
    assert_eq!(
        inheritable.validate(),
        Err(StateContractError::InvalidOfdPolicy)
    );

    let mut attached = fixture().synchronization;
    attached.locks[0].fd_transfer = FdTransferPolicy::ComponentSessionAttachment;
    assert_eq!(attached.validate(), Ok(()));

    let mut implicit = fixture().synchronization;
    implicit.locks[0].fd_transfer = FdTransferPolicy::ExplicitFdMapping;
    assert_eq!(
        implicit.validate(),
        Err(StateContractError::InvalidOfdPolicy)
    );
}

#[test]
fn leases_bind_generation_expiry_revocation_and_explicit_transfer() {
    let lease = LeaseRecord {
        lease_id: ResourceId::parse("lease-1").unwrap(),
        resource_id: ResourceId::parse("role-runtime").unwrap(),
        owner: AuthorityRef::RealmBroker {
            realm_id: realm_id(),
        },
        generation: Generation::new(8).unwrap(),
        expires_at_unix_ms: 10_000,
        revocation: LeaseRevocation::Active,
        fd_transfer: FdTransferPolicy::ScmRightsLeaseHandoff,
    };
    assert_eq!(
        lease.validate_use(Generation::new(8).unwrap(), 9_999),
        Ok(())
    );
    assert_eq!(
        lease.validate_use(Generation::new(7).unwrap(), 9_999),
        Err(StateContractError::LeaseGenerationMismatch)
    );
    assert_eq!(
        lease.validate_use(Generation::new(8).unwrap(), 10_000),
        Err(StateContractError::LeaseExpired)
    );
}

#[test]
fn audit_chain_checkpoint_gap_and_retention_are_explicit() {
    let segment = audit_segment();
    segment.validate().expect("valid segment chain");

    let checkpoint = AuditCheckpoint {
        stream: realm_stream(),
        owner: realm_owner(),
        checkpoint_id: ResourceId::parse("checkpoint-1").unwrap(),
        through_sequence: 2,
        segment_digest: digest(TWO_DIGEST),
        previous_checkpoint_digest: digest(ZERO_DIGEST),
        checkpoint_digest: digest(ONE_DIGEST),
        controller_generation: Generation::new(7).unwrap(),
        created_at_unix_ms: 21_000,
        realm_signature_digest: Some(digest(TWO_DIGEST)),
    };
    checkpoint
        .validate_for_segment(&segment.summary)
        .expect("realm checkpoint binds segment");

    let gap = detect_audit_gap(realm_stream(), 3, 5, 22_000)
        .unwrap()
        .expect("gap is explicit");
    assert_eq!(gap.expected_sequence, 3);
    assert_eq!(gap.observed_sequence, 5);
    assert_eq!(gap.reason, AuditReason::SequenceGap);
    assert!(
        detect_audit_gap(realm_stream(), 3, 3, 22_000)
            .unwrap()
            .is_none()
    );

    let mut broken = segment.clone();
    broken.records[1].previous_hash = digest(ZERO_DIGEST);
    assert_eq!(
        broken.validate(),
        Err(StateContractError::AuditChainMismatch)
    );

    let mut retention = fixture().audit.streams[0].retention.clone();
    assert_eq!(
        retention.decide(1, 1_024, 1, true, false),
        Ok(AuditRetentionDecision::Retain)
    );
    assert_eq!(
        retention.decide(14, 1_024, 1, false, false),
        Ok(AuditRetentionDecision::SealCurrentSegment)
    );
    assert_eq!(
        retention.decide(14, 1_024, 1, true, false),
        Ok(AuditRetentionDecision::CreateCheckpoint)
    );
    assert_eq!(
        retention.decide(14, 1_024, 1, true, true),
        Ok(AuditRetentionDecision::PruneCheckpointedSegment)
    );
    retention.max_age_days = MAX_AUDIT_RETENTION_DAYS + 1;
    assert_eq!(
        retention.validate(),
        Err(StateContractError::RetentionOutOfBounds)
    );

    let export = AuditExportRequest {
        stream: realm_stream(),
        operation_id: CorrelationId::parse("export-1").unwrap(),
        first_sequence: 1,
        last_sequence: 2,
        format: AuditExportFormat::CheckpointBundle,
        include_checkpoints: true,
    };
    assert_eq!(export.validate(), Ok(()));
    let invalid_export = AuditExportRequest {
        first_sequence: 3,
        ..export
    };
    assert_eq!(
        invalid_export.validate(),
        Err(StateContractError::AuditExportRangeInvalid)
    );
}

#[test]
fn audit_is_redacted_bounded_and_has_closed_labels() {
    let record = audit_record(1, ZERO_DIGEST, ONE_DIGEST);
    let encoded = serde_json::to_string(&record).unwrap();
    for forbidden in [
        "path",
        "argv",
        "payload",
        "command",
        "credential",
        "endpoint",
        "proof",
        "secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked field: {forbidden}");
    }

    let mut value = serde_json::to_value(&record).unwrap();
    value["payloadBytes"] = json!("sensitive");
    assert!(serde_json::from_value::<AuditRecord>(value).is_err());
    assert!(serde_json::from_str::<AuditEvent>("\"future-event\"").is_err());
    assert!(serde_json::from_str::<AuditOutcome>("\"maybe\"").is_err());
    assert!(serde_json::from_str::<AuditReason>("\"free-form\"").is_err());

    let mut oversized = record;
    oversized.encoded_bytes = MAX_AUDIT_RECORD_BYTES + 1;
    assert_eq!(
        oversized.validate_bounds(),
        Err(StateContractError::BoundExceeded)
    );
}

#[test]
fn projections_are_diagnostic_read_models_only() {
    let projection = StateProjection {
        schema_version: STATE_SCHEMA_VERSION,
        generated_at_unix_ms: 20_000,
        authority: ProjectionAuthority::DiagnosticsOnly,
        entries: vec![ProjectionEntry {
            scope: IdentityScope::Realm {
                realm_id: realm_id(),
            },
            status: ProjectionStatus::Degraded,
            reason: Some(DegradedReason::AdoptionQuarantined),
            remediation: Some(Remediation::InspectQuarantine),
            observed_generation: Some(Generation::new(4).unwrap()),
        }],
    };
    assert_eq!(projection.validate(), Ok(()));
    assert!(!projection.can_authorize());
    assert!(!projection.can_repair());
    assert_eq!(
        serde_json::to_value(&projection).unwrap()["authority"],
        "diagnostics-only"
    );
}
