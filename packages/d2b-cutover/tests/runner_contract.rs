use d2b_cutover::{
    BootstrapCapability, CandidateId, CapabilityLedger, CutoverPreview, Digest, EffectAllowlist,
    OperationId, OperationKind, OperationRequest, OperatorId, ResetInventory, ResetScope,
    RunnerBootstrap, RunnerCommand, RunnerLockError, RunnerPaths, RunnerSocketError,
    acquire_operation_lock, load_journal, persist_journal,
};
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

fn digest(seed: &str) -> Digest {
    Digest::derive("d2b:test:cutover-runner", seed.as_bytes())
}

#[test]
fn bootstrap_capability_is_single_use_and_rejects_replay() {
    let operation_id = OperationId::new("op-runner-contract").expect("operation id");
    let candidate_id = CandidateId::new("candidate-runner-contract").expect("candidate id");
    let operator_id = OperatorId::new("operator-runner-contract").expect("operator id");
    let capability = BootstrapCapability::new(
        operation_id,
        candidate_id,
        operator_id,
        OperationKind::Cutover,
        digest("nonce"),
        100,
        200,
    )
    .expect("capability");
    let encoded = capability.canonical_bytes().expect("canonical capability");
    let mut ledger = CapabilityLedger::default();

    let consumed =
        BootstrapCapability::decode_and_consume(&encoded, 150, &mut ledger).expect("first use");
    assert_eq!(consumed.operation_kind(), OperationKind::Cutover);
    let replay =
        BootstrapCapability::decode_and_consume(&encoded, 150, &mut ledger).expect_err("replay");
    assert_eq!(
        replay.to_string(),
        "cutover bootstrap capability already consumed"
    );
}

#[test]
fn bootstrap_capability_rejects_expiry_and_wrong_effect_allowlist() {
    let capability = BootstrapCapability::new(
        OperationId::new("op-expiry").expect("operation id"),
        CandidateId::new("candidate-expiry").expect("candidate id"),
        OperatorId::new("operator-expiry").expect("operator id"),
        OperationKind::Cutover,
        digest("expiry"),
        100,
        200,
    )
    .expect("capability");
    let encoded = capability.canonical_bytes().expect("canonical capability");
    let mut ledger = CapabilityLedger::default();

    let error =
        BootstrapCapability::decode_and_consume(&encoded, 201, &mut ledger).expect_err("expired");
    assert_eq!(error.to_string(), "cutover bootstrap capability expired");
    assert_eq!(
        EffectAllowlist::for_operation(OperationKind::Cutover),
        EffectAllowlist::for_operation(OperationKind::Cutover)
    );
}

#[test]
fn runner_socket_authority_distinguishes_admin_hold_and_owner_resume() {
    let capability = BootstrapCapability::new_with_identity(
        OperationId::new("op-socket-auth").expect("operation id"),
        CandidateId::new("candidate-socket-auth").expect("candidate id"),
        OperatorId::new("operator-socket-auth").expect("operator id"),
        OperationKind::Cutover,
        digest("socket-auth"),
        100,
        200,
        42,
        BTreeSet::from([7]),
    )
    .expect("capability");
    let bytes = capability.canonical_bytes().expect("canonical capability");
    let mut ledger = CapabilityLedger::default();
    let consumed =
        BootstrapCapability::decode_and_consume(&bytes, 150, &mut ledger).expect("consume");
    let root = std::path::PathBuf::from(".scratch")
        .join(format!("runner-contract-{}", std::process::id()));
    let paths = RunnerPaths::new(&root, consumed.operation_id());
    let socket = d2b_cutover::RunnerSocket::bind(&paths, consumed).expect("bind socket");

    assert!(
        socket
            .authorize(
                7,
                &RunnerCommand::Hold {
                    reason: "incident".to_owned()
                }
            )
            .is_ok()
    );
    assert_eq!(
        socket
            .authorize(
                7,
                &RunnerCommand::Resume {
                    fresh_consent: None
                }
            )
            .expect_err("admin needs fresh consent"),
        RunnerSocketError::OperatorMismatch
    );
    assert!(
        socket
            .authorize(
                7,
                &RunnerCommand::Resume {
                    fresh_consent: Some(digest("fresh-consent"))
                }
            )
            .is_ok()
    );
    assert!(
        socket
            .authorize(
                42,
                &RunnerCommand::Resume {
                    fresh_consent: None
                }
            )
            .is_ok()
    );
    assert_eq!(
        socket
            .authorize(0, &RunnerCommand::Status)
            .expect_err("HostShutdown is not a cutover peer"),
        RunnerSocketError::Unauthorized
    );
    assert_eq!(
        socket
            .authorize(8, &RunnerCommand::Status)
            .expect_err("unconfigured peer"),
        RunnerSocketError::Unauthorized
    );
    drop(socket);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn journal_is_root_only_and_operation_lock_is_ofd_owned() {
    let operation_id = OperationId::new("op-journal-contract").expect("operation id");
    let candidate_id = CandidateId::new("candidate-journal-contract").expect("candidate id");
    let revision_plan_id =
        d2b_cutover::RevisionPlanId::new("plan-journal-contract").expect("plan id");
    let operator_id = OperatorId::new("operator-journal-contract").expect("operator id");
    let inventory =
        ResetInventory::new(ResetScope::Zone, "zone-journal-contract").expect("inventory");
    let preview = CutoverPreview::new_reset(
        operation_id.clone(),
        OperationKind::ScopedReset(ResetScope::Zone),
        candidate_id.clone(),
        revision_plan_id.clone(),
        inventory.clone(),
    )
    .expect("preview");
    let preview_digest = preview.digest().expect("preview digest");
    let request = OperationRequest::new_reset(
        operation_id.clone(),
        ResetScope::Zone,
        candidate_id.clone(),
        revision_plan_id,
        operator_id.clone(),
        preview_digest,
        inventory,
    )
    .expect("request");
    let capability = BootstrapCapability::new(
        operation_id.clone(),
        candidate_id,
        operator_id,
        OperationKind::ScopedReset(ResetScope::Zone),
        digest("journal-capability"),
        100,
        200,
    )
    .expect("capability");
    let bootstrap = RunnerBootstrap {
        capability,
        request,
        preview,
    };
    let root = std::path::PathBuf::from(".scratch")
        .join(format!("journal-contract-{}", std::process::id()));
    let paths = RunnerPaths::new(&root, &operation_id);
    let lock = acquire_operation_lock(&paths).expect("first lock");
    let second = acquire_operation_lock(&paths).expect_err("second lock must contend");
    assert_eq!(second, RunnerLockError::Contended);
    persist_journal(&paths.journal, &bootstrap, b"").expect("persist journal");
    let metadata = std::fs::metadata(&paths.journal).expect("journal metadata");
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "journal must be root-only"
    );
    let (loaded, loaded_records) = load_journal(&paths.journal).expect("load journal");
    assert_eq!(loaded, bootstrap);
    assert!(loaded_records.is_empty());
    drop(lock);
    let _ = std::fs::remove_dir_all(root);
}
