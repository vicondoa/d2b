use super::*;

fn infrastructure(identity: u64) -> InfrastructureHandle {
    InfrastructureHandle::new(
        ResourceId::new(identity).unwrap_or_else(|_| unreachable!()),
        ResourceGeneration::new(1).unwrap_or_else(|_| unreachable!()),
    )
}

fn deployment(infrastructure: InfrastructureHandle, identity: u64) -> DeploymentHandle {
    DeploymentHandle::new(
        infrastructure,
        ResourceId::new(identity).unwrap_or_else(|_| unreachable!()),
        ResourceGeneration::new(1).unwrap_or_else(|_| unreachable!()),
    )
}

fn key(value: u64) -> OperationKey {
    OperationKey::new(value).unwrap_or_else(|_| unreachable!())
}

#[test]
fn operation_inventory_is_closed_and_axis_separated() {
    assert_eq!(SdkOperation::ALL.len(), 12);
    assert_eq!(
        SdkOperation::ALL
            .iter()
            .filter(|operation| operation.axis() == SdkAxis::Infrastructure)
            .count(),
        6
    );
    assert_eq!(
        SdkOperation::ALL
            .iter()
            .filter(|operation| operation.axis() == SdkAxis::Runtime)
            .count(),
        6
    );
}

#[test]
fn infrastructure_binding_fingerprint_is_deterministic_bounded_and_resource_specific() {
    let configuration = "c".repeat(64);
    let material = InfrastructureBindingMaterial::new(
        2,
        "aaaaaaaaaaaaaaaaaaaa",
        "azure-vm-infrastructure-1",
        "bbbbbbbbbbbbbbbbbbba",
        4,
        1,
        &configuration,
    )
    .unwrap_or_else(|_| unreachable!());
    let resource = infrastructure(7);
    let fingerprint = InfrastructureBindingFingerprint::compute(&material, resource);
    assert_eq!(
        fingerprint,
        InfrastructureBindingFingerprint::compute(&material, resource)
    );
    assert!(fingerprint.verifies(&material, resource));
    assert!(!fingerprint.verifies(&material, infrastructure(8)));

    let other_identity = InfrastructureBindingMaterial::new(
        2,
        "aaaaaaaaaaaaaaaaaaaa",
        "azure-vm-infrastructure-2",
        "bbbbbbbbbbbbbbbbbbba",
        4,
        1,
        &configuration,
    )
    .unwrap_or_else(|_| unreachable!());
    assert_ne!(
        fingerprint,
        InfrastructureBindingFingerprint::compute(&other_identity, resource)
    );
    assert!(
        InfrastructureBindingMaterial::new(
            2,
            "aaaaaaaaaaaaaaaaaaaa",
            "/home/alice/private",
            "bbbbbbbbbbbbbbbbbbba",
            4,
            1,
            &configuration,
        )
        .is_err()
    );

    for rendered in [format!("{material:?}"), format!("{fingerprint:?}")] {
        assert!(!rendered.contains("azure-vm-infrastructure"));
        assert!(!rendered.contains("aaaaaaaa"));
        assert!(!rendered.contains("/home/"));
    }
}

#[tokio::test]
async fn configured_outcomes_are_idempotent_and_count_every_call() {
    let sdk = FakeAzureVmSdk::new();
    let handle = infrastructure(11);
    sdk.configure_outcomes(
        SdkOperation::InfrastructureCreate,
        [
            ConfiguredOutcome::AlreadyApplied,
            ConfiguredOutcome::Applied,
        ],
    )
    .await
    .unwrap_or_else(|_| unreachable!());

    let first_context = SdkCallContext::infrastructure(key(1), handle, 1_000);
    let first = sdk
        .create_infrastructure(&first_context, handle, PowerState::Stopped)
        .await
        .unwrap_or_else(|_| unreachable!());
    let replay = sdk
        .create_infrastructure(&first_context, handle, PowerState::Stopped)
        .await
        .unwrap_or_else(|_| unreachable!());
    let second = sdk
        .create_infrastructure(
            &SdkCallContext::infrastructure(key(2), handle, 1_000),
            handle,
            PowerState::Stopped,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    assert_eq!(first, replay);
    assert_eq!(first.disposition(), ApplyDisposition::AlreadyApplied);
    assert_eq!(second.disposition(), ApplyDisposition::Applied);
    let snapshot = sdk.snapshot().await;
    assert_eq!(snapshot.total_calls(), 3);
    assert_eq!(snapshot.calls(SdkOperation::InfrastructureCreate), 3);
    assert_eq!(snapshot.log().len(), 3);
    assert!(!snapshot.log()[0].replayed());
    assert!(snapshot.log()[1].replayed());
    assert!(!snapshot.log()[2].replayed());
}

#[tokio::test]
async fn cancellation_deadline_and_wrong_axis_do_no_sdk_work() {
    let sdk = FakeAzureVmSdk::new();
    let infrastructure = infrastructure(21);
    let deployment = deployment(infrastructure, 22);

    let cancelled = SdkCallContext::infrastructure(key(3), infrastructure, 1_000).cancelled();
    let error = sdk
        .inspect_infrastructure(&cancelled, infrastructure)
        .await
        .expect_err("cancelled call must fail");
    assert_eq!(error.kind(), FakeSdkErrorKind::Cancelled);

    let expired = SdkCallContext::runtime(key(4), infrastructure, deployment, 0);
    let error = sdk
        .inspect_runtime(&expired, deployment)
        .await
        .expect_err("expired call must fail");
    assert_eq!(error.kind(), FakeSdkErrorKind::DeadlineExpired);

    let wrong_axis = SdkCallContext::runtime(key(5), infrastructure, deployment, 1_000);
    let error = sdk
        .inspect_infrastructure(&wrong_axis, infrastructure)
        .await
        .expect_err("runtime scope must not inspect infrastructure");
    assert_eq!(error.kind(), FakeSdkErrorKind::AuthorityDenied);
    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn queues_replay_state_and_call_log_are_strictly_bounded() {
    let sdk = FakeAzureVmSdk::new();
    let too_many = vec![ConfiguredOutcome::Applied; MAX_CONFIGURED_OUTCOMES + 1];
    let error = sdk
        .configure_outcomes(SdkOperation::RuntimeStart, too_many)
        .await
        .expect_err("oversized outcome queue must fail");
    assert_eq!(error.kind(), FakeSdkErrorKind::BoundExceeded);

    let infrastructure = infrastructure(31);
    for ordinal in 1..=(MAX_REPLAY_ENTRIES + MAX_CALL_LOG_ENTRIES + 8) {
        let handle = deployment(infrastructure, 10_000 + ordinal as u64);
        let context =
            SdkCallContext::runtime(key(100 + ordinal as u64), infrastructure, handle, 1_000);
        let _ = sdk.inspect_runtime(&context, handle).await;
    }
    let snapshot = sdk.snapshot().await;
    assert_eq!(
        snapshot.total_calls(),
        (MAX_REPLAY_ENTRIES + MAX_CALL_LOG_ENTRIES + 8) as u64
    );
    assert_eq!(snapshot.log().len(), MAX_CALL_LOG_ENTRIES);
}

#[tokio::test]
async fn debug_and_errors_reveal_no_opaque_identity_or_context_key() {
    let sdk = FakeAzureVmSdk::new();
    let infrastructure = infrastructure(424_242);
    let context = SdkCallContext::infrastructure(key(313_131), infrastructure, 0);
    let error = sdk
        .inspect_infrastructure(&context, infrastructure)
        .await
        .expect_err("zero deadline must fail");

    for rendered in [
        format!("{infrastructure:?}"),
        format!("{context:?}"),
        format!("{error:?}"),
        error.to_string(),
        format!("{sdk:?}"),
    ] {
        assert!(!rendered.contains("424242"));
        assert!(!rendered.contains("313131"));
        assert!(!rendered.contains("credential"));
        assert!(!rendered.contains("/home/"));
    }
}
