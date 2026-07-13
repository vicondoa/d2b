#![cfg(feature = "v2-state")]

use d2b_contracts::{
    v2_identity::{ProviderId, RealmId, RoleId, WorkloadId},
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

fn workload_id() -> WorkloadId {
    WorkloadId::parse("q5h7jtqteem7kua4tfva").unwrap()
}

fn provider_id() -> ProviderId {
    ProviderId::parse("f7z3k5e3awgn43aljt2a").unwrap()
}

fn other_realm_id() -> RealmId {
    RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap()
}

fn other_workload_id() -> WorkloadId {
    WorkloadId::parse("baaaaaaaaaaaaaaaaaaq").unwrap()
}

fn other_provider_id() -> ProviderId {
    ProviderId::parse("caaaaaaaaaaaaaaaaaaq").unwrap()
}

fn other_role_id() -> RoleId {
    RoleId::parse("daaaaaaaaaaaaaaaaaaq").unwrap()
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

fn audit_record(sequence: u64, previous_hash: Digest) -> AuditRecord {
    let mut record = AuditRecord {
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
        previous_hash,
        record_hash: digest(ZERO_DIGEST),
        encoded_bytes: 512,
    };
    record.record_hash = record.computed_hash();
    record
}

fn audit_segment() -> AuditSegment {
    let first = audit_record(1, digest(ZERO_DIGEST));
    let second = audit_record(2, first.record_hash.clone());
    let mut segment = AuditSegment {
        summary: AuditSegmentSummary {
            stream: realm_stream(),
            owner: realm_owner(),
            segment_id: ResourceId::parse("segment-1").unwrap(),
            first_sequence: 1,
            last_sequence: 2,
            previous_segment_digest: digest(ZERO_DIGEST),
            segment_digest: digest(ZERO_DIGEST),
            controller_generation: Generation::new(7).unwrap(),
            created_at_unix_ms: 10_000,
            sealed_at_unix_ms: 20_000,
            encoded_bytes: 1_024,
            prune_status: PruneStatus::EligibleAfterCheckpoint,
        },
        records: vec![first, second],
    };
    segment.summary.segment_digest = segment.computed_digest();
    segment
}

struct BoundSignatureVerifier;
static BOUND_SIGNATURE_VERIFIER: BoundSignatureVerifier = BoundSignatureVerifier;

impl AuditCheckpointSignatureVerifier for BoundSignatureVerifier {
    fn verify_realm_signature(
        &self,
        _realm_id: &RealmId,
        checkpoint_digest: &Digest,
        signature_digest: &Digest,
    ) -> bool {
        checkpoint_digest == signature_digest
    }
}

fn retention_evidence<'a>(
    age_days: u16,
    segment_bytes: u64,
    record_count: u32,
    sealed_segment: Option<&'a AuditSegment>,
    checkpoint: Option<&'a AuditCheckpoint>,
    previous_checkpoint: Option<&'a AuditCheckpoint>,
) -> AuditRetentionEvidence<'a, BoundSignatureVerifier> {
    AuditRetentionEvidence {
        age_days,
        segment_bytes,
        record_count,
        sealed_segment,
        checkpoint,
        previous_checkpoint,
        signature_verifier: &BOUND_SIGNATURE_VERIFIER,
    }
}

fn checkpoint_for(segment: &AuditSegment) -> AuditCheckpoint {
    let mut checkpoint = AuditCheckpoint {
        stream: realm_stream(),
        owner: realm_owner(),
        checkpoint_id: ResourceId::parse("checkpoint-1").unwrap(),
        through_sequence: segment.summary.last_sequence,
        segment_digest: segment.summary.segment_digest.clone(),
        previous_checkpoint_digest: digest(ZERO_DIGEST),
        checkpoint_digest: digest(ZERO_DIGEST),
        controller_generation: segment.summary.controller_generation,
        created_at_unix_ms: segment.summary.sealed_at_unix_ms + 1,
        realm_signature_digest: None,
    };
    checkpoint.checkpoint_digest = checkpoint.computed_digest();
    checkpoint.realm_signature_digest = Some(checkpoint.checkpoint_digest.clone());
    checkpoint
}

fn next_audit_segment(previous: &AuditSegment) -> AuditSegment {
    let first_sequence = previous.summary.last_sequence + 1;
    let first = audit_record(first_sequence, previous.summary.segment_digest.clone());
    let second = audit_record(first_sequence + 1, first.record_hash.clone());
    let mut segment = AuditSegment {
        summary: AuditSegmentSummary {
            stream: realm_stream(),
            owner: realm_owner(),
            segment_id: ResourceId::parse("segment-2").unwrap(),
            first_sequence,
            last_sequence: first_sequence + 1,
            previous_segment_digest: previous.summary.segment_digest.clone(),
            segment_digest: digest(ZERO_DIGEST),
            controller_generation: Generation::new(7).unwrap(),
            created_at_unix_ms: 21_000,
            sealed_at_unix_ms: 30_000,
            encoded_bytes: 1_024,
            prune_status: PruneStatus::EligibleAfterCheckpoint,
        },
        records: vec![first, second],
    };
    segment.summary.segment_digest = segment.computed_digest();
    segment
}

fn next_checkpoint(segment: &AuditSegment, previous: &AuditCheckpoint) -> AuditCheckpoint {
    let mut checkpoint = AuditCheckpoint {
        stream: realm_stream(),
        owner: realm_owner(),
        checkpoint_id: ResourceId::parse("checkpoint-2").unwrap(),
        through_sequence: segment.summary.last_sequence,
        segment_digest: segment.summary.segment_digest.clone(),
        previous_checkpoint_digest: previous.checkpoint_digest.clone(),
        checkpoint_digest: digest(ZERO_DIGEST),
        controller_generation: segment.summary.controller_generation,
        created_at_unix_ms: segment.summary.sealed_at_unix_ms + 1,
        realm_signature_digest: None,
    };
    checkpoint.checkpoint_digest = checkpoint.computed_digest();
    checkpoint.realm_signature_digest = Some(checkpoint.checkpoint_digest.clone());
    checkpoint
}

#[test]
fn fixture_is_complete_and_fingerprint_bound() {
    let contract = fixture();
    contract.validate().expect("complete fixture validates");
    assert_eq!(
        contract.storage.resources.len(),
        MANDATORY_RESOURCE_CATALOG.len()
    );
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
    assert!(SafeJsonInteger::new(MAX_SAFE_JSON_INTEGER).is_ok());
    assert!(SafeJsonInteger::new(MAX_SAFE_JSON_INTEGER + 1).is_err());
    assert_eq!(
        detect_audit_gap(
            realm_stream(),
            MAX_SAFE_JSON_INTEGER + 1,
            MAX_SAFE_JSON_INTEGER + 1,
            1
        ),
        Err(StateContractError::BoundExceeded)
    );
    assert_eq!(
        detect_audit_gap(realm_stream(), 1, MAX_SAFE_JSON_INTEGER + 1, 1),
        Err(StateContractError::BoundExceeded)
    );
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

fn set_all_storage_authorities(resource: &mut StorageResource, authority: AuthorityRef) {
    resource.creation_authority = authority.clone();
    resource.reconcile_authority = authority.clone();
    resource.repair_authority = authority.clone();
    resource.delete_authority = authority.clone();
    resource.ownership.owner = authority;
}

#[test]
fn storage_requires_the_complete_mandatory_identity_multiset() {
    let mut contract = fixture();
    contract
        .storage
        .resources
        .push(contract.storage.resources[0].clone());
    assert_eq!(
        contract.storage.validate(),
        Err(StateContractError::DuplicateResourceId)
    );

    for omitted in 0..MANDATORY_RESOURCE_CATALOG.len() {
        let mut contract = fixture();
        contract.storage.resources.remove(omitted);
        assert_eq!(
            contract.storage.validate(),
            Err(StateContractError::MissingMandatoryResource),
            "catalog row {omitted} must be mandatory"
        );
    }

    let mut contract = fixture();
    let mut duplicate_key = contract.storage.resources[0].clone();
    duplicate_key.resource_id = ResourceId::parse("different-resource-id").unwrap();
    contract.storage.resources.push(duplicate_key);
    assert_eq!(
        contract.storage.validate(),
        Err(StateContractError::DuplicateMandatoryResource)
    );
}

#[test]
fn every_storage_and_lock_authority_is_exactly_scope_bound() {
    let valid_matrix = [
        (
            "local-root-broker-state",
            vec![
                AuthorityRef::Pid1,
                AuthorityRef::LocalRootAllocator,
                AuthorityRef::LocalRootBroker,
            ],
        ),
        (
            "realm-controller-state",
            vec![
                AuthorityRef::RealmController {
                    realm_id: realm_id(),
                },
                AuthorityRef::RealmBroker {
                    realm_id: realm_id(),
                },
            ],
        ),
        (
            "workload-state",
            vec![
                AuthorityRef::WorkloadController {
                    realm_id: realm_id(),
                    workload_id: workload_id(),
                },
                AuthorityRef::WorkloadBroker {
                    realm_id: realm_id(),
                    workload_id: workload_id(),
                },
            ],
        ),
        (
            "provider-state",
            vec![AuthorityRef::Provider {
                realm_id: realm_id(),
                provider_id: provider_id(),
            }],
        ),
        (
            "role-runtime",
            vec![AuthorityRef::WorkloadRole {
                realm_id: realm_id(),
                workload_id: workload_id(),
                role_id: role_id(),
            }],
        ),
    ];
    for (resource_id, authorities) in valid_matrix {
        for authority in authorities {
            let mut contract = fixture();
            let resource = contract
                .storage
                .resources
                .iter_mut()
                .find(|resource| resource.resource_id.as_str() == resource_id)
                .unwrap();
            set_all_storage_authorities(resource, authority);
            contract.storage.validate().unwrap();
        }
    }

    let invalid_authorities = [
        AuthorityRef::LocalRootBroker,
        AuthorityRef::RealmBroker {
            realm_id: other_realm_id(),
        },
        AuthorityRef::WorkloadBroker {
            realm_id: realm_id(),
            workload_id: other_workload_id(),
        },
        AuthorityRef::Provider {
            realm_id: realm_id(),
            provider_id: other_provider_id(),
        },
        AuthorityRef::WorkloadRole {
            realm_id: realm_id(),
            workload_id: workload_id(),
            role_id: other_role_id(),
        },
    ];
    for resource_index in 0..fixture().storage.resources.len() {
        for authority in invalid_authorities.iter().cloned() {
            let mut contract = fixture();
            let scope = contract.storage.resources[resource_index].scope.clone();
            if scope == IdentityScope::LocalRoot
                && matches!(
                    authority,
                    AuthorityRef::Pid1
                        | AuthorityRef::LocalRootAllocator
                        | AuthorityRef::LocalRootBroker
                )
            {
                continue;
            }
            set_all_storage_authorities(&mut contract.storage.resources[resource_index], authority);
            if contract.storage.validate().is_ok() {
                panic!("cross-scope authority unexpectedly accepted for {scope:?}");
            }
        }
    }

    let mut synchronization = fixture().synchronization;
    synchronization.locks[0].owner = AuthorityRef::RealmBroker {
        realm_id: other_realm_id(),
    };
    synchronization.locks[0].release_authority = synchronization.locks[0].owner.clone();
    assert_eq!(
        synchronization.validate(),
        Err(StateContractError::AuthorityScopeMismatch)
    );

    let mut synchronization = fixture().synchronization;
    synchronization.locks[1].owner = AuthorityRef::WorkloadBroker {
        realm_id: realm_id(),
        workload_id: other_workload_id(),
    };
    synchronization.locks[1].release_authority = synchronization.locks[1].owner.clone();
    assert_eq!(
        synchronization.validate(),
        Err(StateContractError::AuthorityScopeMismatch)
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
fn state_envelopes_verify_canonical_raw_bytes_length_checksum_and_payload() {
    struct CanonicalJson;
    impl CanonicalPayloadVerifier<Value> for CanonicalJson {
        fn decode_canonical(&self, raw_payload: &[u8]) -> Result<Value, StateContractError> {
            let decoded: Value = serde_json::from_slice(raw_payload)
                .map_err(|_| StateContractError::EnvelopePayloadMismatch)?;
            if serde_json::to_vec(&decoded).unwrap() != raw_payload {
                return Err(StateContractError::EnvelopePayloadMismatch);
            }
            Ok(decoded)
        }
    }

    let raw = br#"{"status":"ready"}"#;
    let envelope = StateEnvelope {
        schema_version: STATE_SCHEMA_VERSION,
        schema_generation: STATE_SCHEMA_GENERATION,
        config_generation: Generation::new(4).unwrap(),
        state_generation: Generation::new(9).unwrap(),
        writer: AuthorityRef::RealmController {
            realm_id: realm_id(),
        },
        encoded_bytes: raw.len() as u64,
        checksum: state_payload_digest(raw).unwrap(),
        payload: json!({"status": "ready"}),
    };
    assert_eq!(envelope.validate_payload_bytes(raw, &CanonicalJson), Ok(()));
    assert_eq!(envelope.next_generation().unwrap().get(), 10);

    let mut length_lie = envelope.clone();
    length_lie.encoded_bytes += 1;
    assert_eq!(
        length_lie.validate_payload_bytes(raw, &CanonicalJson),
        Err(StateContractError::BoundExceeded)
    );

    let mut bit_flip = raw.to_vec();
    *bit_flip.last_mut().unwrap() = b']';
    assert_eq!(
        envelope.validate_payload_bytes(&bit_flip, &CanonicalJson),
        Err(StateContractError::EnvelopeChecksumMismatch)
    );

    let noncanonical = br#"{ "status": "ready" }"#;
    let noncanonical_envelope = StateEnvelope {
        encoded_bytes: noncanonical.len() as u64,
        checksum: state_payload_digest(noncanonical).unwrap(),
        ..envelope
    };
    assert_eq!(
        noncanonical_envelope.validate_payload_bytes(noncanonical, &CanonicalJson),
        Err(StateContractError::EnvelopePayloadMismatch)
    );
}

#[test]
fn restart_adoption_requires_exact_role_scoped_target_evidence() {
    let target = RunnerAdoptionTarget {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: role_id(),
        config_generation: Generation::new(3).unwrap(),
        cgroup_identity: digest(TWO_DIGEST),
        executable_fingerprint: digest(ZERO_DIGEST),
        configuration_fingerprint: digest(ONE_DIGEST),
    };
    let exact = RunnerObservation {
        observation_id: ResourceId::parse("runner-observation").unwrap(),
        scope: target.scope_for_test(),
        observed_pid: 42,
        evidence: RunnerEvidence {
            realm_id: realm_id(),
            workload_id: workload_id(),
            role_id: role_id(),
            candidate_count: 1,
            pidfd_persistence: PidfdPersistence::ProcessLocalNonPersistent,
            identity: EvidenceVerdict::Match,
            cgroup_identity: digest(TWO_DIGEST),
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
        observation_id: exact.observation_id.clone(),
        ordering: RecoveryOrdering::RecoverBeforeCleanup,
        decision: AdoptionDecision::Adopt {
            fresh_pidfd_opened: true,
        },
    };
    assert_eq!(adopt.validate_for_runner(&exact, &target), Ok(()));

    let mut mismatches = Vec::new();
    let mut mismatch = exact.clone();
    mismatch.evidence.realm_id = other_realm_id();
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.workload_id = other_workload_id();
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.role_id = other_role_id();
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.cgroup_identity = digest(ZERO_DIGEST);
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.executable_fingerprint = digest(ONE_DIGEST);
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.configuration_fingerprint = digest(TWO_DIGEST);
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.config_generation = Generation::new(2).unwrap();
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.evidence.identity = EvidenceVerdict::Ambiguous;
    mismatch.evidence.candidate_count = 2;
    mismatches.push(mismatch);
    let mut mismatch = exact.clone();
    mismatch.scope = IdentityScope::Role {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: other_role_id(),
    };
    mismatches.push(mismatch);
    for field in [
        "identity",
        "cgroup-membership",
        "executable",
        "configuration",
        "generation",
    ] {
        let mut mismatch = exact.clone();
        match field {
            "identity" => mismatch.evidence.identity = EvidenceVerdict::Mismatch,
            "cgroup-membership" => {
                mismatch.evidence.cgroup_membership = EvidenceVerdict::Mismatch;
            }
            "executable" => mismatch.evidence.executable = EvidenceVerdict::Mismatch,
            "configuration" => mismatch.evidence.configuration = EvidenceVerdict::Mismatch,
            "generation" => mismatch.evidence.generation = EvidenceVerdict::Mismatch,
            _ => unreachable!(),
        }
        mismatches.push(mismatch);
    }
    for mismatch in mismatches {
        assert_eq!(
            adopt.validate_for_runner(&mismatch, &target),
            Err(StateContractError::RestartAmbiguous)
        );
        let quarantine = RestartDecision {
            observation_id: mismatch.observation_id.clone(),
            ordering: RecoveryOrdering::RecoverBeforeCleanup,
            decision: AdoptionDecision::Quarantine {
                reason: QuarantineReason::OwnerAmbiguous,
                remediation: Remediation::InspectQuarantine,
            },
        };
        assert_eq!(quarantine.validate_for_runner(&mismatch, &target), Ok(()));
    }

    let serialized = serde_json::to_string(&exact).unwrap();
    assert!(serialized.contains("process-local-non-persistent"));
    assert!(!serialized.contains("pidfd\":"));
    assert!(!serialized.contains("executablePath"));
    assert!(!serialized.contains("cgroupPath"));
}

trait TargetScopeForTest {
    fn scope_for_test(&self) -> IdentityScope;
}

impl TargetScopeForTest for RunnerAdoptionTarget {
    fn scope_for_test(&self) -> IdentityScope {
        IdentityScope::Role {
            realm_id: self.realm_id.clone(),
            workload_id: self.workload_id.clone(),
            role_id: self.role_id.clone(),
        }
    }
}

#[test]
fn cleanup_requires_fresh_target_bound_completed_discovery_absence() {
    let role_target = CleanupTarget::Role {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: role_id(),
    };
    let absent = RunnerObservation {
        observation_id: ResourceId::parse("runner-observation").unwrap(),
        scope: IdentityScope::Role {
            realm_id: realm_id(),
            workload_id: workload_id(),
            role_id: role_id(),
        },
        observed_pid: 0,
        evidence: RunnerEvidence {
            realm_id: realm_id(),
            workload_id: workload_id(),
            role_id: role_id(),
            candidate_count: 0,
            pidfd_persistence: PidfdPersistence::ProcessLocalNonPersistent,
            identity: EvidenceVerdict::Missing,
            cgroup_identity: digest(TWO_DIGEST),
            cgroup_membership: EvidenceVerdict::Missing,
            executable_fingerprint: digest(ZERO_DIGEST),
            executable: EvidenceVerdict::Missing,
            configuration_fingerprint: digest(ONE_DIGEST),
            configuration: EvidenceVerdict::Missing,
            config_generation: Generation::new(3).unwrap(),
            generation: EvidenceVerdict::Missing,
        },
    };
    let discovery = RestartDiscovery {
        discovery_id: ResourceId::parse("discovery-1").unwrap(),
        config_generation: Generation::new(3).unwrap(),
        completed_at_unix_ms: SafeJsonInteger::new(20_000).unwrap(),
        runners: vec![absent.clone()],
        resources: vec![],
    };
    let proof = discovery.prove_owner_absence(role_target.clone()).unwrap();
    let cleanup = RestartDecision {
        observation_id: absent.observation_id.clone(),
        ordering: RecoveryOrdering::RecoverBeforeCleanup,
        decision: AdoptionDecision::Cleanup {
            target: role_target.clone(),
            owner_absence_proof: proof.clone(),
        },
    };
    assert_eq!(
        cleanup.validate_cleanup(&discovery, Generation::new(3).unwrap()),
        Ok(())
    );
    assert_eq!(
        cleanup.validate_cleanup(&discovery, Generation::new(2).unwrap()),
        Err(StateContractError::CleanupWithoutOwnerAbsenceProof)
    );
    let mut newer_discovery = discovery.clone();
    newer_discovery.discovery_id = ResourceId::parse("discovery-2").unwrap();
    newer_discovery.completed_at_unix_ms = SafeJsonInteger::new(21_000).unwrap();
    assert_eq!(
        cleanup.validate_cleanup(&newer_discovery, Generation::new(3).unwrap()),
        Err(StateContractError::CleanupWithoutOwnerAbsenceProof)
    );

    let wrong_target = RestartDecision {
        observation_id: absent.observation_id.clone(),
        ordering: RecoveryOrdering::RecoverBeforeCleanup,
        decision: AdoptionDecision::Cleanup {
            target: CleanupTarget::Workload {
                realm_id: realm_id(),
                workload_id: workload_id(),
            },
            owner_absence_proof: proof,
        },
    };
    assert_eq!(
        wrong_target.validate_cleanup(&discovery, Generation::new(3).unwrap()),
        Err(StateContractError::CleanupWithoutOwnerAbsenceProof)
    );

    for verdict in [EvidenceVerdict::Match, EvidenceVerdict::Ambiguous] {
        let mut live_or_ambiguous = discovery.clone();
        live_or_ambiguous.runners[0].evidence.candidate_count = 1;
        live_or_ambiguous.runners[0].evidence.identity = verdict;
        assert_eq!(
            live_or_ambiguous.prove_owner_absence(role_target.clone()),
            Err(StateContractError::CleanupWithoutOwnerAbsenceProof)
        );
    }
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

    let checkpoint = checkpoint_for(&segment);
    checkpoint
        .verify_for_segment(&segment, None, &BoundSignatureVerifier)
        .expect("realm checkpoint binds segment");

    let gap = detect_audit_gap(realm_stream(), 3, 5, 22_000)
        .unwrap()
        .expect("gap is explicit");
    assert_eq!(gap.expected_sequence.get(), 3);
    assert_eq!(gap.observed_sequence.get(), 5);
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
        retention.decide(retention_evidence(1, 1_024, 1, None, None, None)),
        Ok(AuditRetentionDecision::Retain)
    );
    assert_eq!(
        retention.decide(retention_evidence(14, 1_024, 1, None, None, None)),
        Ok(AuditRetentionDecision::SealCurrentSegment)
    );
    assert_eq!(
        retention.decide(retention_evidence(
            14,
            segment.summary.encoded_bytes,
            segment.records.len() as u32,
            Some(&segment),
            None,
            None,
        )),
        Ok(AuditRetentionDecision::CreateCheckpoint)
    );
    assert_eq!(
        retention.decide(retention_evidence(
            14,
            segment.summary.encoded_bytes,
            segment.records.len() as u32,
            Some(&segment),
            Some(&checkpoint),
            None,
        )),
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
fn audit_hashes_bind_contents_ranges_checkpoint_chain_and_signatures() {
    let first_segment = audit_segment();
    let first_checkpoint = checkpoint_for(&first_segment);
    let second_segment = next_audit_segment(&first_segment);
    let second_checkpoint = next_checkpoint(&second_segment, &first_checkpoint);
    second_checkpoint
        .verify_for_segment(
            &second_segment,
            Some(&first_checkpoint),
            &BoundSignatureVerifier,
        )
        .unwrap();
    second_segment.validate_after(&first_segment).unwrap();

    let mut changed_record = first_segment.clone();
    changed_record.records[0].event = AuditEvent::StorageDelete;
    assert_eq!(
        changed_record.validate(),
        Err(StateContractError::AuditChainMismatch)
    );

    let mut arbitrary_record_digest = first_segment.clone();
    arbitrary_record_digest.records[0].record_hash = digest(ONE_DIGEST);
    assert_eq!(
        arbitrary_record_digest.validate(),
        Err(StateContractError::AuditChainMismatch)
    );

    let mut arbitrary_segment_root = first_segment.clone();
    arbitrary_segment_root.summary.segment_digest = digest(TWO_DIGEST);
    assert_eq!(
        arbitrary_segment_root.validate(),
        Err(StateContractError::AuditChainMismatch)
    );

    let mut wrong_range = second_segment.clone();
    wrong_range.summary.first_sequence -= 1;
    assert_eq!(
        wrong_range.validate(),
        Err(StateContractError::AuditSequenceMismatch)
    );

    let mut arbitrary_checkpoint_digest = first_checkpoint.clone();
    arbitrary_checkpoint_digest.checkpoint_digest = digest(TWO_DIGEST);
    assert_eq!(
        arbitrary_checkpoint_digest.verify_for_segment(
            &first_segment,
            None,
            &BoundSignatureVerifier
        ),
        Err(StateContractError::AuditCheckpointMismatch)
    );

    let mut unbound_signature = first_checkpoint.clone();
    unbound_signature.realm_signature_digest = Some(digest(TWO_DIGEST));
    assert_eq!(
        unbound_signature.verify_for_segment(&first_segment, None, &BoundSignatureVerifier),
        Err(StateContractError::AuditCheckpointMismatch)
    );

    assert_eq!(
        second_checkpoint.verify_for_segment(&second_segment, None, &BoundSignatureVerifier),
        Err(StateContractError::AuditCheckpointMismatch)
    );

    let mut unrelated_checkpoint = second_checkpoint.clone();
    unrelated_checkpoint.segment_digest = first_segment.summary.segment_digest.clone();
    unrelated_checkpoint.checkpoint_digest = unrelated_checkpoint.computed_digest();
    unrelated_checkpoint.realm_signature_digest =
        Some(unrelated_checkpoint.checkpoint_digest.clone());
    let retention = fixture().audit.streams[1].retention.clone();
    assert_eq!(
        retention.decide(retention_evidence(
            retention.max_age_days,
            second_segment.summary.encoded_bytes,
            second_segment.records.len() as u32,
            Some(&second_segment),
            Some(&unrelated_checkpoint),
            Some(&first_checkpoint),
        )),
        Err(StateContractError::AuditCheckpointMismatch)
    );

    let stale_checkpoint = first_checkpoint.clone();
    assert_eq!(
        retention.decide(retention_evidence(
            retention.max_age_days,
            second_segment.summary.encoded_bytes,
            second_segment.records.len() as u32,
            Some(&second_segment),
            Some(&stale_checkpoint),
            Some(&first_checkpoint),
        )),
        Err(StateContractError::AuditCheckpointMismatch)
    );
}

#[test]
fn audit_is_redacted_bounded_and_has_closed_labels() {
    let record = audit_record(1, digest(ZERO_DIGEST));
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
        oversized.validate_integrity(),
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
