use d2b_contracts::v3::credential::CredentialLeaseState;
use d2b_contracts::v3::{StateDigest, StateEnvelope, VolumeStateError};
use d2b_provider_volume_local::audit::VolumeAuditKind;
use d2b_provider_volume_local::sealing::{
    BoundRotationResult, RotationPhase, SealingAction, SealingError, SealingState,
    validate_envelope_for_sealing,
};
use serde_json::json;

#[test]
fn rotation_is_status_first_and_uses_only_the_admitted_bound_effect() {
    let mut state = SealingState::sealed(4).unwrap();
    let pending = state
        .observe_credential(CredentialLeaseState::Active, 5)
        .unwrap();
    assert_eq!(pending.phase, RotationPhase::StatusCommitRequired);
    assert_eq!(pending.action, SealingAction::CommitPendingStatus);
    assert_eq!(
        pending.audit,
        Some(VolumeAuditKind::VolumeSealingRotationStart)
    );
    assert_eq!(
        state.apply_bound_result(BoundRotationResult::Rotated),
        Err(SealingError::InvalidTransition)
    );

    let ready = state.pending_status_committed().unwrap();
    assert_eq!(ready.action, SealingAction::InvokeBoundRotation);
    let committed = state
        .apply_bound_result(BoundRotationResult::Rotated)
        .unwrap();
    assert_eq!(committed.phase, RotationPhase::Sealed);
    assert_eq!(committed.action, SealingAction::CommitSealedStatus);
    assert_eq!(state.active_generation(), 5);
    assert_eq!(state.target_generation(), None);
}

#[test]
fn interrupted_commit_retries_identical_request_until_durable_audit() {
    let mut state = SealingState::sealed(7).unwrap();
    state
        .observe_credential(CredentialLeaseState::Active, 8)
        .unwrap();
    state.pending_status_committed().unwrap();
    let pending = state
        .apply_bound_result(BoundRotationResult::CommitPendingAudit)
        .unwrap();
    assert_eq!(pending.phase, RotationPhase::CommitPendingAudit);
    assert_eq!(pending.action, SealingAction::RetryIdenticalRotation);

    let recovered = state
        .apply_bound_result(BoundRotationResult::RecoveredCommitted)
        .unwrap();
    assert_eq!(recovered.phase, RotationPhase::Sealed);
    assert_eq!(state.active_generation(), 8);
}

#[test]
fn revoked_credential_and_terminal_rotation_preserve_old_generation() {
    let mut revoked = SealingState::sealed(3).unwrap();
    let revoked_transition = revoked
        .observe_credential(CredentialLeaseState::Revoked, 4)
        .unwrap();
    assert_eq!(revoked_transition.action, SealingAction::CommitFailedStatus);
    assert_eq!(revoked.phase(), RotationPhase::Failed);
    assert_eq!(revoked.active_generation(), 3);

    let mut failed = SealingState::sealed(3).unwrap();
    failed
        .observe_credential(CredentialLeaseState::Active, 4)
        .unwrap();
    failed.pending_status_committed().unwrap();
    let transition = failed
        .apply_bound_result(BoundRotationResult::TerminalFailure)
        .unwrap();
    assert_eq!(transition.action, SealingAction::CommitFailedStatus);
    assert_eq!(failed.active_generation(), 3);
}

#[test]
fn envelope_sealing_fails_closed_without_a_provider_state_digest_domain() {
    let envelope = StateEnvelope::new(
        1,
        StateDigest::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
        json!({"private": true}),
    )
    .unwrap();
    assert_eq!(
        envelope.validate_digest(),
        Err(VolumeStateError::DigestDomainUnavailable)
    );
    assert_eq!(
        validate_envelope_for_sealing(&envelope),
        Err(SealingError::DigestDomainUnavailable)
    );
}
