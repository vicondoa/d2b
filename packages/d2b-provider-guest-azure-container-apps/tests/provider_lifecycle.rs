use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use async_trait::async_trait;
use d2b_contracts_provider::v3::credential::CredentialLeaseHandle;
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_guest_azure_container_apps::{
    AcaClock, AcaConfiguredDiskId, AcaControl, AcaControlContext, AcaControlError,
    AcaControlErrorKind, AcaControlHealth, AcaController, AcaControllerError, AcaCpuMillis,
    AcaCredentialLease, AcaCredentialLeaseClient, AcaCredentialLeaseRequest, AcaDeleteOutcome,
    AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageCandidates, AcaDiskImageId,
    AcaDiskImageRecord, AcaDiskImageSource, AcaMemoryMib, AcaOperationId, AcaPhase, AcaProfileId,
    AcaReadinessPolicy, AcaReconcileOutcome, AcaResourceBinding,
    AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxId, AcaSandboxLifecycle, AcaSandboxProfile,
    AcaSandboxRecord,
};

#[derive(Default)]
struct FakeState {
    candidates: Vec<AcaSandboxRecord>,
    calls: Vec<&'static str>,
    revoked: usize,
    lease_expiries: Vec<u64>,
    resume_lifecycle: Option<AcaSandboxLifecycle>,
    delete_failures: usize,
    health: VecDeque<AcaControlHealth>,
    desired_sandbox: Option<AcaDesiredSandbox>,
}

struct FakeLeaseClient {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AcaCredentialLeaseClient for FakeLeaseClient {
    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError> {
        self.state
            .lock()
            .await
            .lease_expiries
            .push(request.requested_expiry_unix_ms());
        Ok(AcaCredentialLease::from_metadata(
            CredentialLeaseHandle::parse("aca-test-lease").unwrap(),
            request.requested_expiry_unix_ms(),
        ))
    }

    async fn revoke(&self, _: &AcaCredentialLease) -> Result<(), AcaControlError> {
        self.state.lock().await.revoked += 1;
        Ok(())
    }
}

struct FakeControl {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AcaControl for FakeControl {
    async fn health(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError> {
        Ok(self
            .state
            .lock()
            .await
            .health
            .pop_front()
            .unwrap_or(AcaControlHealth::Ready))
    }

    async fn find_sandboxes(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &d2b_provider_guest_azure_container_apps::AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError> {
        self.state.lock().await.calls.push("find-sandboxes");
        Ok(AcaSandboxCandidates::new(self.state.lock().await.candidates.clone()).unwrap())
    }

    async fn find_disk_images(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError> {
        self.state.lock().await.calls.push("find-images");
        Ok(AcaDiskImageCandidates::new(Vec::new()).unwrap())
    }

    async fn create_disk_image(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        self.state.lock().await.calls.push("create-image");
        Ok(AcaDiskImageRecord {
            id: AcaDiskImageId::parse("disk-1").unwrap(),
            generation: 1,
        })
    }

    async fn create_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.state.lock().await.calls.push("create-sandbox");
        self.state.lock().await.desired_sandbox = Some(desired.clone());
        Ok(record(AcaSandboxLifecycle::Creating))
    }

    async fn resume_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.state.lock().await.calls.push("resume");
        let lifecycle = self
            .state
            .lock()
            .await
            .resume_lifecycle
            .unwrap_or(AcaSandboxLifecycle::Running);
        Ok(record(lifecycle))
    }

    async fn stop_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.state.lock().await.calls.push("stop");
        Ok(record(AcaSandboxLifecycle::Stopped))
    }

    async fn delete_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        _: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError> {
        self.state.lock().await.calls.push("delete");
        let mut state = self.state.lock().await;
        if state.delete_failures > 0 {
            state.delete_failures -= 1;
            return Err(AcaControlError::new(AcaControlErrorKind::Unavailable));
        }
        Ok(AcaDeleteOutcome::Deleted)
    }
}

fn record(lifecycle: AcaSandboxLifecycle) -> AcaSandboxRecord {
    AcaSandboxRecord {
        id: AcaSandboxId::parse("sandbox-1").unwrap(),
        lifecycle,
        generation: 1,
    }
}

fn controller(state: Arc<Mutex<FakeState>>) -> AcaController<FakeControl, FakeLeaseClient> {
    let profile = AcaSandboxProfile::new(
        AcaProfileId::parse("default").unwrap(),
        AcaDiskImageSource::ConfiguredDisk {
            binding_id: AcaConfiguredDiskId::parse("image-1").unwrap(),
        },
        AcaCpuMillis::new(500).unwrap(),
        AcaMemoryMib::new(2_048).unwrap(),
        300,
        None,
    )
    .unwrap();
    let config =
        AcaRuntimeConfig::new(profile, AcaReadinessPolicy::new(3, 10).unwrap(), 1_000, 4).unwrap();
    let binding = AcaResourceBinding {
        guest_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        provider_generation: 1,
        config_fingerprint: [7; 32],
    };
    AcaController::new(
        binding,
        config,
        Arc::new(FakeControl {
            state: Arc::clone(&state),
        }),
        Arc::new(FakeLeaseClient { state }),
    )
}

struct FixedClock(u64);

impl AcaClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn running_sandbox_reaches_ready_without_exposing_identity() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    let operation = AcaOperationId::parse("operation-1").unwrap();
    assert_eq!(
        controller.reconcile(operation, 1_000).await.unwrap(),
        AcaReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AcaPhase::Ready);
    
    assert_eq!(state.lock().await.revoked, 2);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn completed_operation_reconcile_replays_without_effects() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state)).with_clock(Arc::new(FixedClock(0)));
    let operation = AcaOperationId::parse("operation-replay").unwrap();
    assert_eq!(
        controller.reconcile(operation.clone(), 1_000).await.unwrap(),
        AcaReconcileOutcome::Converged
    );
    let calls = state.lock().await.calls.clone();
    let revoked = state.lock().await.revoked;
    assert_eq!(
        controller.reconcile(operation, 1_000).await.unwrap(),
        AcaReconcileOutcome::Converged
    );
    assert_eq!(state.lock().await.calls, calls);
    assert_eq!(state.lock().await.revoked, revoked);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn running_sandbox_requires_authenticated_healthy_control() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        health: VecDeque::from([AcaControlHealth::Degraded, AcaControlHealth::Ready]),
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    assert!(matches!(
        controller
            .reconcile(AcaOperationId::parse("operation-health-1").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), AcaPhase::Degraded);
    assert_eq!(
        controller
            .reconcile(AcaOperationId::parse("operation-health-2").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AcaPhase::Ready);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn ambiguous_adoption_fails_closed() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![
            record(AcaSandboxLifecycle::Running),
            AcaSandboxRecord {
                id: AcaSandboxId::parse("sandbox-2").unwrap(),
                lifecycle: AcaSandboxLifecycle::Running,
                generation: 1,
            },
        ],
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    let error = controller
        .reconcile(AcaOperationId::parse("operation-2").unwrap(), 1_000)
        .await
        .unwrap_err();
    assert_eq!(error, AcaControllerError::AmbiguousAdoption);
    assert_eq!(controller.phase(), AcaPhase::Degraded);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn missing_sandbox_uses_disk_and_sandbox_effects_then_finalizes() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller(Arc::clone(&state));
    assert!(matches!(
        controller
            .reconcile(AcaOperationId::parse("operation-3").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AcaPhase::Provisioning);
    assert_eq!(
        state.lock().await.calls,
        [
            "find-sandboxes",
            "find-images",
            "create-image",
            "create-sandbox"
        ]
    );
    state.lock().await.candidates = vec![record(AcaSandboxLifecycle::Stopped)];
    controller
        .finalize(AcaOperationId::parse("operation-4").unwrap(), 1_000)
        .await
        .unwrap();
    assert_eq!(controller.phase(), AcaPhase::Finalized);
    assert!(!controller.finalizer_installed());
    assert_eq!(state.lock().await.calls.last(), Some(&"delete"));
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn provider_settings_reach_the_sandbox_effect() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut controller = controller(Arc::clone(&state)).with_provider_settings(
        Some(ResourceRef::parse("Network/egress").unwrap()),
        AcaProfileId::parse("relay").unwrap(),
    );

    controller
        .reconcile(
            AcaOperationId::parse("operation-provider-settings").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    let desired = state
        .lock()
        .await
        .desired_sandbox
        .clone()
        .expect("sandbox effect should receive desired settings");
    assert_eq!(
        desired.network_ref,
        Some(ResourceRef::parse("Network/egress").unwrap())
    );
    assert_eq!(
        desired.sandbox_transport_alias,
        AcaProfileId::parse("relay").unwrap()
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn failed_sandbox_is_deleted_during_finalization() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Failed)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));

    controller
        .finalize(
            AcaOperationId::parse("operation-finalize-failed").unwrap(),
            1_000,
        )
        .await
        .unwrap();

    assert!(!controller.finalizer_installed());
    assert_eq!(state.lock().await.calls, ["find-sandboxes", "delete"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn unknown_sandbox_fails_closed_during_finalization() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Unknown)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));

    assert_eq!(
        controller
            .finalize(
                AcaOperationId::parse("operation-finalize-unknown").unwrap(),
                1_000,
            )
            .await,
        Err(AcaControllerError::Effect(AcaControlErrorKind::Ambiguous))
    );
    assert!(controller.finalizer_installed());
    assert_eq!(controller.phase(), AcaPhase::Degraded);
    assert_eq!(state.lock().await.calls, ["find-sandboxes"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn finalization_waits_for_a_creating_sandbox_before_stopping() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Creating)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));

    controller
        .finalize(
            AcaOperationId::parse("operation-finalize-creating").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert!(controller.finalizer_installed());
    assert_eq!(state.lock().await.calls, ["find-sandboxes"]);

    state.lock().await.candidates = vec![record(AcaSandboxLifecycle::Stopped)];
    controller
        .finalize(
            AcaOperationId::parse("operation-finalize-stopped").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert!(!controller.finalizer_installed());
    assert_eq!(
        state.lock().await.calls,
        ["find-sandboxes", "find-sandboxes", "delete"]
    );
}





#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn resume_waits_for_running_lifecycle() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Suspended)],
        resume_lifecycle: Some(AcaSandboxLifecycle::Creating),
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    assert!(matches!(
        controller
            .reconcile(AcaOperationId::parse("operation-resume").unwrap(), 1_000)
            .await
            .unwrap(),
        AcaReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AcaPhase::Provisioning);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn readiness_attempts_are_bounded() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Creating)],
        ..FakeState::default()
    }));
    let mut controller = controller(state);
    for index in 0..2 {
        assert!(matches!(
            controller
                .reconcile(
                    AcaOperationId::parse(format!("operation-ready-{index}")).unwrap(),
                    1_000
                )
                .await
                .unwrap(),
            AcaReconcileOutcome::Progressing { .. }
        ));
    }
    assert_eq!(
        controller
            .reconcile(
                AcaOperationId::parse("operation-ready-final").unwrap(),
                1_000
            )
            .await
            .unwrap_err(),
        AcaControllerError::ReadinessExhausted
    );
    assert_eq!(controller.phase(), AcaPhase::Failed);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn lease_expiry_uses_absolute_unix_time() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state)).with_clock(Arc::new(FixedClock(1_234_567)));
    controller
        .reconcile(AcaOperationId::parse("operation-clock").unwrap(), 1_000)
        .await
        .unwrap();
    assert_eq!(
        state.lock().await.lease_expiries,
        vec![1_235_567, 1_235_567]
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn finalization_retries_after_partial_delete_failure() {
    let state = Arc::new(Mutex::new(FakeState {
        candidates: vec![record(AcaSandboxLifecycle::Running)],
        delete_failures: 1,
        ..FakeState::default()
    }));
    let mut controller = controller(Arc::clone(&state));
    controller
        .reconcile(
            AcaOperationId::parse("operation-finalize-observe").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert_eq!(
        controller
            .finalize(
                AcaOperationId::parse("operation-finalize-first").unwrap(),
                1_000
            )
            .await
            .unwrap_err(),
        AcaControllerError::Effect(AcaControlErrorKind::Unavailable)
    );
    assert!(controller.finalizer_installed());
    controller
        .finalize(
            AcaOperationId::parse("operation-finalize-retry").unwrap(),
            1_000,
        )
        .await
        .unwrap();
    assert!(!controller.finalizer_installed());
}

#[test]
fn stable_error_codes_are_bounded() {
    assert_eq!(
        AcaControlError::new(AcaControlErrorKind::RateLimited).code(),
        "aca-control-rate-limited"
    );
}

// ---------------------------------------------------------------------------
// The admitted remote authority
//
// Everything below drives the controller through the converted entry point:
// the graph admitted one Guest, one cloud account, and one credential
// relationship, and every remote call has to survive all three before the
// control plane is contacted.
// ---------------------------------------------------------------------------

use d2b_contracts_provider::v3::credential::{
    DeliveryRouteDigest, MAX_DELIVERY_RECORD_BYTES,
};
use d2b_contracts_provider::v3::{
    AdmittedCredentialDelivery, AudienceToken, CredentialAuthorization,
    CredentialDeliveryEvidence, CredentialMethod, DeliveryIdentity, DeliverySessionParams,
    OperationClass, PresentationCapability,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration, ZoneId,
};
use d2b_provider_guest_azure_container_apps::{
    ACA_ARTIFACT_ID, ACA_CONTROL_AUDIENCE, AcaAdmittedGuest, AcaAdmittedRemote, AcaCloudIdentity,
    AcaConfiguredImageId, AcaDeliveryContext, AcaProviderConfig, AcaReconciliationKey,
    AcaRemoteAuthority, AcaRemoteDeliveryPort, AcaRemotePurpose, AcaRemoteRefusal,
    azure_container_apps_declaration, declared_presentation,
};

const ZONE: &str = "work";
const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const OTHER_GUEST_UID: &str = "223e4567-e89b-42d3-a456-426614174000";
const ROUTE_DIGEST: &str =
    "sha256:6f1c1b6f2a6f2cbb1f2e5b2f0c3a7a2b6c9d0e1f2a3b4c5d6e7f809a1b2c3d4e";

/// A candidate list over the bound is a malformed control-plane response.
fn control_error(error: d2b_provider_guest_azure_container_apps::AcaTypeError) -> AcaControlError {
    let _ = error;
    AcaControlError::new(AcaControlErrorKind::InvalidResponse)
}

fn operation_id(name: &str) -> AcaOperationId {
    AcaOperationId::parse(name).expect("valid operation identifier")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("canonical reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical uid")
}

fn generation(value: u64) -> ResourceGeneration {
    ResourceGeneration::new(value).expect("nonzero generation")
}

/// The `CredentialBinding` relationship the graph admitted, held exactly as
/// the source side committed it: an audience, a set of granted operation
/// classes, and the one current delivery session.
struct AdmittedRelationship {
    audience: AudienceToken,
    granted: Vec<OperationClass>,
    current: DeliveryIdentity,
    refusal: Option<BindingRefusal>,
}

impl AdmittedRelationship {
    fn live() -> Self {
        Self {
            audience: AudienceToken::parse(ACA_CONTROL_AUDIENCE).expect("ARM audience"),
            granted: vec![OperationClass::AcquireToken],
            current: identity(1, ACA_CONTROL_AUDIENCE),
            refusal: None,
        }
    }

    /// The same relationship after the source rotated its Credential: the
    /// consumer generation the session carries has moved on.
    fn revoked() -> Self {
        Self {
            refusal: Some(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            )),
            ..Self::live()
        }
    }
}

impl AdmittedCredentialDelivery for AdmittedRelationship {
    fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    fn grants(&self, class: OperationClass) -> bool {
        self.granted.contains(&class)
    }

    fn current_delivery(&self) -> DeliveryIdentity {
        self.current.clone()
    }

    fn fence_refusal(&self, _: &CredentialDeliveryEvidence) -> Option<BindingRefusal> {
        self.refusal
    }
}

fn identity(sequence: u64, audience: &str) -> DeliveryIdentity {
    DeliveryIdentity::new(
        reference("Credential/arm"),
        uid(GUEST_UID),
        generation(1),
        reference(d2b_provider_guest_azure_container_apps::PROVIDER_REF),
        generation(1),
        AudienceToken::parse(audience).expect("audience"),
        OperationClass::AcquireToken,
        sequence,
    )
}

fn delivery(sequence: u64, audience: &str) -> DeliverySessionParams {
    DeliverySessionParams::new(
        reference("Credential/arm"),
        uid(GUEST_UID),
        generation(1),
        reference(d2b_provider_guest_azure_container_apps::PROVIDER_REF),
        generation(1),
        AudienceToken::parse(audience).expect("audience"),
        OperationClass::AcquireToken,
        u64::MAX,
        u64::MAX,
        DeliveryRouteDigest::parse(ROUTE_DIGEST).expect("route digest"),
        MAX_DELIVERY_RECORD_BYTES as u32,
        sequence,
    )
    .expect("delivery session")
}

fn authorization(sequence: u64, audience: &str) -> CredentialAuthorization {
    CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(sequence, audience)),
    )
    .expect("authorization")
}

fn evidence() -> CredentialDeliveryEvidence {
    CredentialDeliveryEvidence::new(generation(1), generation(1), generation(1), 1)
}

/// The credential adapter the daemon composition root fills with the real
/// authenticated client. It records what it was asked so a test can prove the
/// credential is not even requested for a refused operation.
struct FakeDelivery {
    sequence: u64,
    audience: String,
    asked: Arc<Mutex<usize>>,
}

#[async_trait]
impl AcaRemoteDeliveryPort for FakeDelivery {
    async fn authorize(
        &self,
        purpose: AcaRemotePurpose,
        _: AcaOperationId,
    ) -> Result<AcaDeliveryContext, AcaRemoteRefusal> {
        *self.asked.lock().await += 1;
        let method = purpose.credential_method();
        let authorization = if method.requires_delivery() {
            authorization(self.sequence, &self.audience)
        } else {
            // A read carries no material: the adapter authorizes the metadata
            // method with no delivery session at all.
            CredentialAuthorization::new(method, None).expect("metadata authorization")
        };
        Ok(AcaDeliveryContext::new(authorization, evidence()))
    }
}

fn admitted_guest(uid_value: &str, generation_value: u64) -> AcaAdmittedGuest {
    AcaAdmittedGuest::new(
        ZoneId::parse(ZONE).expect("zone"),
        reference("Guest/workload"),
        uid(uid_value),
        reference(d2b_provider_guest_azure_container_apps::PROVIDER_REF),
        generation_value,
    )
    .expect("the evidence names this provider")
}

fn cloud(environment: &str, resource_group: &str) -> AcaCloudIdentity {
    AcaCloudIdentity::new(
        "tenant".to_owned(),
        "client".to_owned(),
        "subscription".to_owned(),
        AcaConfiguredImageId::parse(environment).expect("environment"),
        AcaConfiguredImageId::parse(resource_group).expect("resource group"),
    )
}

fn admitted_remote(
    guest: AcaAdmittedGuest,
    cloud_identity: AcaCloudIdentity,
    relationship: AdmittedRelationship,
    sequence: u64,
    audience: &str,
    asked: Arc<Mutex<usize>>,
) -> (Arc<AcaAdmittedRemote>, AcaReconciliationKey) {
    let authority = Arc::new(
        AcaRemoteAuthority::new(
            guest,
            cloud_identity,
            Arc::new(relationship),
        )
        .expect("the ARM audience is a bounded token"),
    );
    let key = authority.reconciliation_key();
    (
        Arc::new(AcaAdmittedRemote::new(
            authority,
            Arc::new(FakeDelivery {
                sequence,
                audience: audience.to_owned(),
                asked,
            }),
        )),
        key,
    )
}

/// A control plane that stores sandboxes by name, so a second create under a
/// different name is visible as a second resource rather than hidden.
#[derive(Default)]
struct CloudPlane {
    stored: Vec<AcaSandboxRecord>,
    created: Vec<AcaSandboxId>,
    looked_up: Vec<Option<AcaSandboxId>>,
    ambiguous_create: bool,
    disk_images: Vec<AcaConfiguredImageId>,
    stops: Vec<AcaSandboxId>,
}

impl CloudPlane {
    fn put(&mut self, name: AcaSandboxId, lifecycle: AcaSandboxLifecycle) {
        self.stored.push(AcaSandboxRecord {
            id: name,
            lifecycle,
            generation: 1,
        });
    }

    fn by_name(&self, name: Option<AcaSandboxId>) -> Vec<AcaSandboxRecord> {
        match name {
            Some(name) => self.stored.iter().filter(|record| record.id == name).cloned().collect(),
            None => Vec::new(),
        }
    }
}

struct NamedControl {
    plane: Arc<Mutex<CloudPlane>>,
}

#[async_trait]
impl AcaControl for NamedControl {
    async fn health(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError> {
        Ok(AcaControlHealth::Ready)
    }

    async fn find_sandboxes(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        query: &d2b_provider_guest_azure_container_apps::AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError> {
        let mut plane = self.plane.lock().await;
        let name = query
            .reconciliation
            .as_ref()
            .map(|key| key.sandbox_name().clone());
        plane.looked_up.push(name.clone());
        AcaSandboxCandidates::new(plane.by_name(name)).map_err(control_error)
    }

    async fn find_disk_images(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError> {
        let Some(name) = desired.name.clone() else {
            return AcaDiskImageCandidates::new(Vec::new()).map_err(control_error);
        };
        let plane = self.plane.lock().await;
        let found = plane
            .disk_images
            .iter()
            .filter(|stored| **stored == name)
            .map(|_| AcaDiskImageRecord {
                id: AcaDiskImageId::parse("disk-1").unwrap(),
                generation: 1,
            })
            .collect();
        AcaDiskImageCandidates::new(found).map_err(control_error)
    }

    async fn create_disk_image(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        let name = desired
            .name
            .clone()
            .ok_or(AcaControlError::new(AcaControlErrorKind::InvalidResponse))?;
        self.plane.lock().await.disk_images.push(name);
        Ok(AcaDiskImageRecord {
            id: AcaDiskImageId::parse("disk-1").unwrap(),
            generation: 1,
        })
    }

    async fn create_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        let name = desired
            .reconciliation
            .as_ref()
            .map(|key| key.sandbox_name().clone())
            .ok_or(AcaControlError::new(AcaControlErrorKind::InvalidResponse))?;
        let mut plane = self.plane.lock().await;
        plane.created.push(name.clone());
        if plane.ambiguous_create {
            // The resource really was created; the response never arrived.
            // This is the case a naive retry turns into a duplicate.
            plane.put(name, AcaSandboxLifecycle::Running);
            plane.ambiguous_create = false;
            return Err(AcaControlError::new(AcaControlErrorKind::Ambiguous));
        }
        plane.put(name.clone(), AcaSandboxLifecycle::Running);
        Ok(AcaSandboxRecord {
            id: name,
            lifecycle: AcaSandboxLifecycle::Creating,
            generation: 1,
        })
    }

    async fn resume_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        Ok(AcaSandboxRecord {
            id: id.clone(),
            lifecycle: AcaSandboxLifecycle::Running,
            generation: 1,
        })
    }

    async fn stop_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        // The first stop is observed as still stopping, so a release test can
        // hold the controller between "stop requested" and "delete issued".
        let mut plane = self.plane.lock().await;
        let stopping = !plane.stops.iter().any(|stopped| stopped == id);
        plane.stops.push(id.clone());
        let lifecycle = if stopping {
            // The cloud keeps stopping; the stored record says so, and the
            // returned record still reports the in-flight stop.
            AcaSandboxLifecycle::Stopped
        } else {
            AcaSandboxLifecycle::Stopped
        };
        if let Some(record) = plane.stored.iter_mut().find(|record| record.id == *id) {
            record.lifecycle = lifecycle;
        }
        Ok(AcaSandboxRecord {
            id: id.clone(),
            lifecycle: if stopping {
                AcaSandboxLifecycle::Stopping
            } else {
                lifecycle
            },
            generation: 1,
        })
    }

    async fn delete_sandbox(
        &self,
        _: &AcaCredentialLease,
        _: &AcaControlContext,
        id: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError> {
        let mut plane = self.plane.lock().await;
        plane.stored.retain(|record| record.id != *id);
        Ok(AcaDeleteOutcome::Deleted)
    }
}

/// A controller wired to the admitted authority and to a name-addressed cloud.
fn admitted_controller(
    plane: Arc<Mutex<CloudPlane>>,
    guest_uid: &str,
    binding_uid: &str,
    generation: u64,
    requested_presentation: PresentationCapability,
    environment: &str,
    resource_group: &str,
) -> (AcaController<NamedControl, FakeLeaseClient>, AcaReconciliationKey) {
    let (controller, key) = admitted_controller_on(
        plane,
        guest_uid,
        binding_uid,
        generation,
        requested_presentation,
        environment,
        resource_group,
        environment,
        resource_group,
    );
    (controller, key)
}

/// The same controller, but the cloud account its `Provider` configuration
/// addresses is stated separately from the one the authority admitted.
#[allow(clippy::too_many_arguments, reason = "one parameter per identity input")]
fn admitted_controller_on(
    plane: Arc<Mutex<CloudPlane>>,
    guest_uid: &str,
    binding_uid: &str,
    generation: u64,
    requested_presentation: PresentationCapability,
    admitted_environment: &str,
    admitted_resource_group: &str,
    controller_environment: &str,
    controller_resource_group: &str,
) -> (AcaController<NamedControl, FakeLeaseClient>, AcaReconciliationKey) {
    let asked = Arc::new(Mutex::new(0));
    let (remote, key) = admitted_remote(
        admitted_guest(guest_uid, generation),
        cloud(admitted_environment, admitted_resource_group),
        AdmittedRelationship::live(),
        1,
        ACA_CONTROL_AUDIENCE,
        Arc::clone(&asked),
    );
    let binding = AcaResourceBinding {
        guest_uid: uid(binding_uid),
        provider_generation: generation,
        config_fingerprint: [7; 32],
    };
    let controller = AcaController::new(
        binding,
        runtime_config(),
        Arc::new(NamedControl { plane }),
        Arc::new(FakeLeaseClient {
            state: Arc::new(Mutex::new(FakeState::default())),
        }),
    )
    .with_cloud_identity(cloud(controller_environment, controller_resource_group))
    .with_admitted_authority(remote)
    .with_requested_presentation(requested_presentation);
    (controller, key)
}

fn runtime_config() -> AcaRuntimeConfig {
    let profile = AcaSandboxProfile::new(
        AcaProfileId::parse("default").unwrap(),
        AcaDiskImageSource::ConfiguredDisk {
            binding_id: AcaConfiguredDiskId::parse("image-1").unwrap(),
        },
        AcaCpuMillis::new(500).unwrap(),
        AcaMemoryMib::new(2_048).unwrap(),
        300,
        None,
    )
    .unwrap();
    AcaRuntimeConfig::new(profile, AcaReadinessPolicy::new(3, 10).unwrap(), 1_000, 4).unwrap()
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_ambiguous_create_retries_onto_the_same_cloud_resource() {
    let plane = Arc::new(Mutex::new(CloudPlane {
        ambiguous_create: true,
        ..CloudPlane::default()
    }));
    let (mut controller, key) = admitted_controller(
        Arc::clone(&plane),
        GUEST_UID,
        GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
    );

    // First pass: nothing is stored, so the controller creates. The create
    // really happens and the response is lost.
    assert!(
        controller.reconcile(operation_id("provision"), 1_000).await.is_err(),
        "an ambiguous create must not read as success"
    );
    {
        let plane = plane.lock().await;
        assert_eq!(plane.created, [key.sandbox_name().clone()]);
        assert_eq!(
            plane.stored.len(),
            1,
            "the cloud really holds the resource the lost response described"
        );
    }

    // Second pass: the retry derives the same name and adopts it.
    assert_eq!(
        controller.reconcile(operation_id("provision"), 1_000).await.unwrap(),
        AcaReconcileOutcome::Converged
    );
    let plane = plane.lock().await;
    assert_eq!(
        plane.created,
        [key.sandbox_name().clone()],
        "the retry must reconcile the first attempt's resource, not create a second"
    );
    assert_eq!(plane.stored.len(), 1, "exactly one sandbox exists in the cloud");
    assert_eq!(
        plane.looked_up,
        vec![Some(key.sandbox_name().clone()); 2],
        "both attempts asked for the one admitted cloud name"
    );
    assert_eq!(
        plane.disk_images,
        [key.disk_image_name().clone()],
        "the disk image is named from the same admitted key"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_revoked_credential_prevents_new_remote_mutation() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let asked = Arc::new(Mutex::new(0));
    let authority = Arc::new(
        AcaRemoteAuthority::new(
            admitted_guest(GUEST_UID, 1),
            cloud("environment", "resource-group"),
            Arc::new(AdmittedRelationship::revoked()),
        )
        .unwrap(),
    );
    let remote = Arc::new(AcaAdmittedRemote::new(
        authority,
        Arc::new(FakeDelivery {
            sequence: 1,
            audience: ACA_CONTROL_AUDIENCE.to_owned(),
            asked: Arc::clone(&asked),
        }),
    ));
    let mut controller = AcaController::new(
        AcaResourceBinding {
            guest_uid: uid(GUEST_UID),
            provider_generation: 1,
            config_fingerprint: [7; 32],
        },
        runtime_config(),
        Arc::new(NamedControl {
            plane: Arc::clone(&plane),
        }),
        Arc::new(FakeLeaseClient {
            state: Arc::new(Mutex::new(FakeState::default())),
        }),
    )
    .with_cloud_identity(cloud("environment", "resource-group"))
    .with_admitted_authority(remote);

    assert_eq!(
        controller.reconcile(operation_id("provision"), 1_000).await.unwrap_err(),
        AcaControllerError::RemoteRefused
    );
    assert!(
        plane.lock().await.created.is_empty() && plane.lock().await.looked_up.is_empty(),
        "a revoked credential must stop the call before the control plane is read"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_guest_this_provider_did_not_admit_cannot_mutate_the_cloud() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&plane),
        GUEST_UID,
        OTHER_GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
    );
    assert_eq!(
        controller.reconcile(operation_id("provision"), 1_000).await.unwrap_err(),
        AcaControllerError::RemoteRefused
    );
    assert!(plane.lock().await.looked_up.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_controller_on_another_cloud_account_cannot_mutate_this_one() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let (mut controller, _) = admitted_controller_on(
        Arc::clone(&plane),
        GUEST_UID,
        GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
        "environment",
        "other-resource-group",
    );
    assert_eq!(
        controller.reconcile(operation_id("provision"), 1_000).await.unwrap_err(),
        AcaControllerError::RemoteRefused
    );
    assert!(plane.lock().await.looked_up.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unsupported_local_presentation_refuses_before_any_remote_call() {
    for requested in [
        PresentationCapability::FilesystemPresentation,
        PresentationCapability::NamespaceFirstServiceSource,
    ] {
        let plane = Arc::new(Mutex::new(CloudPlane::default()));
        let (mut controller, _) = admitted_controller(
            Arc::clone(&plane),
            GUEST_UID,
            GUEST_UID,
            1,
            requested,
            "environment",
            "resource-group",
        );
        assert_eq!(
            controller.reconcile(operation_id("provision"), 1_000).await.unwrap_err(),
            AcaControllerError::RemoteRefused,
            "{requested:?} is not something a Container Apps sandbox can present"
        );
        assert!(plane.lock().await.looked_up.is_empty());
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_supported_presentation_hands_the_effect_the_admitted_identity() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let (mut controller, key) = admitted_controller(
        Arc::clone(&plane),
        GUEST_UID,
        GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
    );
    controller.reconcile(operation_id("provision"), 1_000).await.unwrap();
    controller.reconcile(operation_id("provision"), 1_000).await.unwrap();

    // The effective-access evidence a remote backend can produce hermetically:
    // the control plane was addressed by exactly the name the accepted Guest
    // identity derives, in exactly the admitted cloud account. Whether Azure
    // then honours the membership is a live acceptance condition, not
    // something a fake can answer.
    let plane = plane.lock().await;
    assert_eq!(
        plane.looked_up,
        vec![Some(key.sandbox_name().clone()); 2],
        "the find path and the create path address one admitted name"
    );
    assert_eq!(plane.created, [key.sandbox_name().clone()]);
    assert_eq!(key, AcaReconciliationKey::derive(&admitted_guest(GUEST_UID, 1)));
    assert_ne!(
        key,
        AcaReconciliationKey::derive(&admitted_guest(OTHER_GUEST_UID, 1)),
        "two admitted Guests never share a cloud name"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn release_needs_a_confirmed_delete_and_a_dropped_finalizer() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let (mut controller, key) = admitted_controller(
        Arc::clone(&plane),
        GUEST_UID,
        GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
    );

    controller.reconcile(operation_id("provision"), 1_000).await.unwrap();
    controller.reconcile(operation_id("provision"), 1_000).await.unwrap();

    // The Guest is running, so nothing is released yet.
    assert!(
        controller.release_evidence().is_none(),
        "a running Guest has no release evidence"
    );

    controller.finalize(operation_id("delete"), 1_000).await.unwrap();
    assert!(
        controller.release_evidence().is_none(),
        "a stop that has not become a delete is not a release"
    );
    assert_eq!(plane.lock().await.stored.len(), 1, "the sandbox is still there");

    controller.finalize(operation_id("delete"), 1_000).await.unwrap();
    let evidence = controller
        .release_evidence()
        .expect("a confirmed delete releases the binding");
    assert!(evidence.is_terminal());
    assert!(evidence.finalizer_released());
    assert_eq!(evidence.deletion(), AcaDeleteOutcome::Deleted);
    assert_eq!(evidence.guest_uid(), &uid(GUEST_UID));
    assert_eq!(evidence.generation(), 1);
    assert_eq!(evidence.reconciliation(), &key);
    assert!(
        plane.lock().await.stored.is_empty(),
        "the cloud no longer holds the sandbox"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_guest_the_cloud_never_had_still_releases_with_honest_evidence() {
    let plane = Arc::new(Mutex::new(CloudPlane::default()));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&plane),
        GUEST_UID,
        GUEST_UID,
        1,
        declared_presentation(),
        "environment",
        "resource-group",
    );

    controller.finalize(operation_id("delete"), 1_000).await.unwrap();
    let evidence = controller
        .release_evidence()
        .expect("an absent Guest still releases");
    assert!(evidence.is_terminal());
    assert_eq!(
        evidence.deletion(),
        AcaDeleteOutcome::AlreadyAbsent,
        "release evidence names what the cloud actually reported"
    );
}

#[test]
fn removed_configuration_and_operation_paths_are_refused_deterministically() {
    // A Provider configuration carrying a field the removed model accepted is
    // refused by decode, not silently dropped.
    let legacy = r#"{
        "gatewayExecutionRef": "Guest/gateway",
        "tenantId": "tenant",
        "clientId": "client",
        "subscriptionId": "subscription",
        "controlCredentialRef": "Credential/arm",
        "environmentId": "environment",
        "resourceGroupId": "resource-group",
        "sandboxTransportAlias": "default",
        "managedIdentityClientSecret": "not-a-field-anymore",
        "defaults": {
            "profile": {
                "profileId": "default",
                "diskImage": {"configuredDisk": {"binding_id": "image-1"}},
                "cpu": 500,
                "memory": 2048,
                "autoSuspendSecs": 300,
                "sandboxIdentityBindingId": null
            },
            "readiness": {"attempts": 3, "intervalMs": 10},
            "planTtlMs": 1000,
            "completedOperationCapacity": 4
        }
    }"#;
    assert!(serde_json::from_str::<AcaProviderConfig>(legacy).is_err());

    // The credential relationship is reached only through an admitted
    // CredentialBinding now, so a control-plane read is the one purpose that
    // must not present a delivery session at all.
    assert_eq!(
        AcaRemotePurpose::Inspect.credential_method(),
        CredentialMethod::InspectMetadata
    );
    assert!(!AcaRemotePurpose::Inspect.mutates_remote_state());
    for purpose in [
        AcaRemotePurpose::Ensure,
        AcaRemotePurpose::Start,
        AcaRemotePurpose::Stop,
        AcaRemotePurpose::Destroy,
    ] {
        assert!(purpose.mutates_remote_state(), "{purpose:?} mutates the cloud");
        assert_eq!(
            purpose.credential_method(),
            CredentialMethod::AcquireToken
        );
    }
    assert!(AcaRemotePurpose::Ensure.code().starts_with("aca-remote-"));

    // The declaration is the one place that says what this provider is.
    let spec = azure_container_apps_declaration();
    assert_eq!(spec.artifact_id().as_str(), ACA_ARTIFACT_ID);
    assert_eq!(
        spec.provider().to_canonical_string(),
        d2b_provider_guest_azure_container_apps::PROVIDER_REF
    );
    assert_eq!(declared_presentation(), PresentationCapability::None);
}

#[test]
fn the_admitted_guest_refuses_a_graph_identity_that_is_not_this_provider() {
    assert_eq!(
        AcaAdmittedGuest::new(
            ZoneId::parse(ZONE).unwrap(),
            reference("Guest/workload"),
            uid(GUEST_UID),
            reference("Provider/runtime-qemu-media"),
            1,
        )
        .unwrap_err(),
        AcaRemoteRefusal::ProviderIdentityMismatch
    );
    assert_eq!(
        AcaAdmittedGuest::new(
            ZoneId::parse(ZONE).unwrap(),
            reference("Guest/workload"),
            uid(GUEST_UID),
            reference(d2b_provider_guest_azure_container_apps::PROVIDER_REF),
            0,
        )
        .unwrap_err(),
        AcaRemoteRefusal::GuestIdentityMismatch
    );
}
