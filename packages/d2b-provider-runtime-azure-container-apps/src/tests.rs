use std::{
    collections::BTreeSet,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, ConfiguredItemId, CredentialLease,
        CredentialLeaseState, CredentialLeaseTransferPolicy, Fingerprint, Generation,
        IdempotencyKey, ImplementationId, LeaseId, MutationState, ObservationReason,
        ObservedLifecycleState, OperationId, Provider, ProviderCallContext, ProviderCapability,
        ProviderCapabilitySet, ProviderFailureKind, ProviderHealthState, ProviderMethod,
        ProviderOperationContext, ProviderOperationInput, ProviderOperationRequest,
        ProviderPlacement, ProviderTarget, RetryClass, RuntimeProvider, SdkOperationClass,
        SourceVersion,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, ProviderRegistryBuilder,
};
use d2b_provider_toolkit::{DeterministicClock, Fixture, Secret};

use super::*;

const NOW_UNIX_MS: u64 = 1_700_000_000_000;
const SECRET_CANARY: &str = "aca-private-token-canary-do-not-emit";

struct JumpOnCallClock {
    before_unix_ms: u64,
    after_unix_ms: u64,
    jump_on_call: usize,
    calls: AtomicUsize,
}

impl JumpOnCallClock {
    fn new(before_unix_ms: u64, after_unix_ms: u64, jump_on_call: usize) -> Self {
        Self {
            before_unix_ms,
            after_unix_ms,
            jump_on_call,
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderClock for JumpOnCallClock {
    fn now_unix_ms(&self) -> u64 {
        let call = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        if call >= self.jump_on_call {
            self.after_unix_ms
        } else {
            self.before_unix_ms
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct PrivateCredentialVault {
    secret: Secret<String>,
    active_leases: BTreeSet<LeaseId>,
    redemptions: usize,
}

impl PrivateCredentialVault {
    fn new() -> Self {
        Self {
            secret: Secret::new(SECRET_CANARY.to_owned()),
            active_leases: BTreeSet::new(),
            redemptions: 0,
        }
    }

    fn register(&mut self, lease_id: LeaseId) {
        self.active_leases.insert(lease_id);
    }

    fn redeem(&mut self, lease: &AcaCredentialLease) -> bool {
        if !self.active_leases.contains(&lease.metadata().lease_id) {
            return false;
        }
        self.redemptions += 1;
        self.secret
            .with_exposed(|secret| secret.as_str() == SECRET_CANARY)
    }

    fn revoke(&mut self, lease: &AcaCredentialLease) -> bool {
        self.active_leases.remove(&lease.metadata().lease_id)
    }

    fn active_lease_count(&self) -> usize {
        self.active_leases.len()
    }
}

impl fmt::Debug for PrivateCredentialVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateCredentialVault")
            .field("active_lease_count", &self.active_leases.len())
            .field("redemptions", &self.redemptions)
            .field("secret", &self.secret)
            .finish()
    }
}

struct FakeCredentialClient {
    descriptor: d2b_contracts::v2_provider::ProviderDescriptor,
    vault: Arc<Mutex<PrivateCredentialVault>>,
    acquisitions: AtomicUsize,
    revocations: AtomicUsize,
    completed_revocations: AtomicUsize,
    cancelled_revocations: AtomicUsize,
    stall_revoke_once: AtomicBool,
    timeout_revoke_once: AtomicBool,
    purposes: Mutex<Vec<AcaCredentialPurpose>>,
    fail_once: Mutex<Option<AcaControlError>>,
    revoke_fail_once: Mutex<Option<AcaControlError>>,
    last_metadata: Mutex<Option<CredentialLease>>,
}

impl FakeCredentialClient {
    fn new(
        descriptor: d2b_contracts::v2_provider::ProviderDescriptor,
        vault: Arc<Mutex<PrivateCredentialVault>>,
    ) -> Self {
        Self {
            descriptor,
            vault,
            acquisitions: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            completed_revocations: AtomicUsize::new(0),
            cancelled_revocations: AtomicUsize::new(0),
            stall_revoke_once: AtomicBool::new(false),
            timeout_revoke_once: AtomicBool::new(false),
            purposes: Mutex::new(Vec::new()),
            fail_once: Mutex::new(None),
            revoke_fail_once: Mutex::new(None),
            last_metadata: Mutex::new(None),
        }
    }

    fn acquisition_count(&self) -> usize {
        self.acquisitions.load(Ordering::Acquire)
    }

    fn revocation_count(&self) -> usize {
        self.revocations.load(Ordering::Acquire)
    }

    fn completed_revocation_count(&self) -> usize {
        self.completed_revocations.load(Ordering::Acquire)
    }

    fn cancelled_revocation_count(&self) -> usize {
        self.cancelled_revocations.load(Ordering::Acquire)
    }

    fn stall_next_revoke(&self) {
        self.stall_revoke_once.store(true, Ordering::Release);
    }

    fn timeout_next_revoke(&self) {
        self.timeout_revoke_once.store(true, Ordering::Release);
    }

    fn fail_next(&self, error: AcaControlError) {
        *lock(&self.fail_once) = Some(error);
    }

    fn fail_next_revoke(&self, error: AcaControlError) {
        *lock(&self.revoke_fail_once) = Some(error);
    }

    fn last_metadata(&self) -> Option<CredentialLease> {
        lock(&self.last_metadata).clone()
    }
}

struct InFlightRevoke<'a> {
    cancelled_revocations: &'a AtomicUsize,
    completed: bool,
}

impl Drop for InFlightRevoke<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled_revocations.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[async_trait]
impl AcaCredentialLeaseClient for FakeCredentialClient {
    fn descriptor(&self) -> d2b_contracts::v2_provider::ProviderDescriptor {
        self.descriptor.clone()
    }

    async fn acquire(
        &self,
        request: &AcaCredentialLeaseRequest,
    ) -> Result<AcaCredentialLease, AcaControlError> {
        let ordinal = self.acquisitions.fetch_add(1, Ordering::AcqRel) + 1;
        lock(&self.purposes).push(request.purpose());
        if let Some(error) = lock(&self.fail_once).take() {
            return Err(error);
        }
        let Some(placement_binding) = self.descriptor.placement.credential_binding() else {
            return Err(AcaControlError::closed(
                AcaControlErrorKind::InvalidResponse,
                AcaDiagnosticCode::InvalidResponse,
            ));
        };
        let allowed_operations =
            BoundedVec::new(request.required_operations().to_vec()).map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?;
        let metadata = CredentialLease {
            lease_id: LeaseId::parse(format!("aca-lease-{ordinal}")).map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            credential_provider_id: self.descriptor.provider_id.clone(),
            consumer_provider_id: request.operation().provider_id.clone(),
            placement_binding,
            allowed_operations,
            issued_at_unix_ms: NOW_UNIX_MS,
            expires_at_unix_ms: request.requested_expiry_unix_ms(),
            credential_provider_generation: self.descriptor.registry_generation,
            consumer_provider_generation: request.operation().provider_generation,
            source_version: SourceVersion::parse("aca-source-v1").map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            rotation_generation: Generation::new(1).map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            state: CredentialLeaseState::Active,
            transfer_policy: CredentialLeaseTransferPolicy::Forbidden,
            revoked_at_unix_ms: None,
        };
        lock(&self.vault).register(metadata.lease_id.clone());
        *lock(&self.last_metadata) = Some(metadata.clone());
        Ok(AcaCredentialLease::from_canonical(metadata))
    }

    async fn revoke(&self, lease: &AcaCredentialLease) -> Result<(), AcaControlError> {
        self.revocations.fetch_add(1, Ordering::AcqRel);
        if let Some(error) = lock(&self.revoke_fail_once).take() {
            return Err(error);
        }
        let revoke_delay_ms = if self.timeout_revoke_once.swap(false, Ordering::AcqRel) {
            Some(u64::from(MAX_ACA_LEASE_CLEANUP_MS) + 100)
        } else if self.stall_revoke_once.swap(false, Ordering::AcqRel) {
            Some(50)
        } else {
            None
        };
        if let Some(revoke_delay_ms) = revoke_delay_ms {
            let mut in_flight = InFlightRevoke {
                cancelled_revocations: &self.cancelled_revocations,
                completed: false,
            };
            tokio::time::sleep(Duration::from_millis(revoke_delay_ms)).await;
            in_flight.completed = true;
        }
        if !lock(&self.vault).revoke(lease) {
            return Err(AcaControlError::closed(
                AcaControlErrorKind::InvalidResponse,
                AcaDiagnosticCode::InvalidResponse,
            ));
        }
        self.completed_revocations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlCall {
    Health,
    FindSandboxes,
    FindDiskImages,
    ResolveConfiguredDisk,
    CreateDiskImage,
    CreateSandbox,
    ResumeSandbox,
    StopSandbox,
    DeleteSandbox,
}

impl ControlCall {
    const fn operation_class(self) -> SdkOperationClass {
        match self {
            Self::Health | Self::ResolveConfiguredDisk => SdkOperationClass::Read,
            Self::FindSandboxes | Self::FindDiskImages => SdkOperationClass::Discover,
            Self::CreateDiskImage | Self::CreateSandbox => SdkOperationClass::Create,
            Self::ResumeSandbox | Self::StopSandbox => SdkOperationClass::Power,
            Self::DeleteSandbox => SdkOperationClass::Delete,
        }
    }
}

#[derive(Default)]
struct FakeControlState {
    calls: Vec<ControlCall>,
    deadlines_ms: Vec<u32>,
    sandboxes: Vec<AcaSandboxRecord>,
    disk_images: Vec<AcaDiskImageRecord>,
    fail_once: Option<(ControlCall, AcaControlError)>,
    stall_once: Option<ControlCall>,
}

struct FakeAcaControl {
    vault: Arc<Mutex<PrivateCredentialVault>>,
    state: Mutex<FakeControlState>,
    cancelled_calls: AtomicUsize,
}

struct InFlightControlCall<'a> {
    cancelled_calls: &'a AtomicUsize,
    completed: bool,
}

impl Drop for InFlightControlCall<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled_calls.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl FakeAcaControl {
    fn new(vault: Arc<Mutex<PrivateCredentialVault>>) -> Self {
        Self {
            vault,
            state: Mutex::new(FakeControlState::default()),
            cancelled_calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> Vec<ControlCall> {
        lock(&self.state).calls.clone()
    }

    fn deadlines_ms(&self) -> Vec<u32> {
        lock(&self.state).deadlines_ms.clone()
    }

    fn cancelled_call_count(&self) -> usize {
        self.cancelled_calls.load(Ordering::Acquire)
    }

    fn sandboxes(&self) -> Vec<AcaSandboxRecord> {
        lock(&self.state).sandboxes.clone()
    }

    fn set_sandboxes(&self, sandboxes: Vec<AcaSandboxRecord>) {
        lock(&self.state).sandboxes = sandboxes;
    }

    fn fail_next(&self, call: ControlCall, error: AcaControlError) {
        lock(&self.state).fail_once = Some((call, error));
    }

    fn stall_next(&self, call: ControlCall) {
        lock(&self.state).stall_once = Some(call);
    }

    async fn enter(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        call: ControlCall,
    ) -> Result<(), AcaControlError> {
        if context.operation_class() != call.operation_class()
            || !lease
                .metadata()
                .allowed_operations
                .contains(&context.operation_class())
            || !lock(&self.vault).redeem(lease)
        {
            return Err(AcaControlError::closed(
                AcaControlErrorKind::Authentication,
                AcaDiagnosticCode::AuthenticationFailed,
            ));
        }
        let (failure, stall) = {
            let mut state = lock(&self.state);
            state.calls.push(call);
            state.deadlines_ms.push(context.deadline_remaining_ms());
            let failure = match state.fail_once {
                Some((expected, error)) if expected == call => {
                    state.fail_once = None;
                    Some(error)
                }
                _ => None,
            };
            let stall = state.stall_once == Some(call);
            if stall {
                state.stall_once = None;
            }
            (failure, stall)
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if stall {
            let mut in_flight = InFlightControlCall {
                cancelled_calls: &self.cancelled_calls,
                completed: false,
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            in_flight.completed = true;
        }
        Ok(())
    }

    fn sandbox_with_lifecycle(
        &self,
        sandbox_id: &AcaSandboxId,
        lifecycle: AcaSandboxLifecycle,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        let mut state = lock(&self.state);
        let Some(index) = state
            .sandboxes
            .iter()
            .position(|record| record.id() == sandbox_id)
        else {
            return Err(AcaControlError::closed(
                AcaControlErrorKind::NotFound,
                AcaDiagnosticCode::ResourceMissing,
            ));
        };
        let current = &state.sandboxes[index];
        let updated =
            AcaSandboxRecord::new(current.id().clone(), current.binding().clone(), lifecycle);
        state.sandboxes[index] = updated.clone();
        Ok(updated)
    }
}

#[async_trait]
impl AcaControl for FakeAcaControl {
    async fn health(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError> {
        self.enter(lease, context, ControlCall::Health).await?;
        Ok(AcaControlHealth::Ready)
    }

    async fn find_sandboxes(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        _query: &AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError> {
        self.enter(lease, context, ControlCall::FindSandboxes)
            .await?;
        AcaSandboxCandidates::new(lock(&self.state).sandboxes.clone()).map_err(|_| {
            AcaControlError::closed(
                AcaControlErrorKind::InvalidResponse,
                AcaDiagnosticCode::InvalidResponse,
            )
        })
    }

    async fn find_disk_images(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        _desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError> {
        self.enter(lease, context, ControlCall::FindDiskImages)
            .await?;
        AcaDiskImageCandidates::new(lock(&self.state).disk_images.clone()).map_err(|_| {
            AcaControlError::closed(
                AcaControlErrorKind::InvalidResponse,
                AcaDiagnosticCode::InvalidResponse,
            )
        })
    }

    async fn resolve_configured_disk(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        self.enter(lease, context, ControlCall::ResolveConfiguredDisk)
            .await?;
        let record = AcaDiskImageRecord::new(
            AcaDiskImageId::parse("configured-disk").map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            desired.binding().clone(),
            desired.profile_id().clone(),
            desired.source().clone(),
        );
        lock(&self.state).disk_images = vec![record.clone()];
        Ok(record)
    }

    async fn create_disk_image(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError> {
        self.enter(lease, context, ControlCall::CreateDiskImage)
            .await?;
        let record = AcaDiskImageRecord::new(
            AcaDiskImageId::parse("created-disk").map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            desired.binding().clone(),
            desired.profile_id().clone(),
            desired.source().clone(),
        );
        lock(&self.state).disk_images = vec![record.clone()];
        Ok(record)
    }

    async fn create_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.enter(lease, context, ControlCall::CreateSandbox)
            .await?;
        let record = AcaSandboxRecord::new(
            AcaSandboxId::parse("sandbox-one").map_err(|_| {
                AcaControlError::closed(
                    AcaControlErrorKind::InvalidResponse,
                    AcaDiagnosticCode::InvalidResponse,
                )
            })?,
            desired.binding().clone(),
            AcaSandboxLifecycle::Idle,
        );
        lock(&self.state).sandboxes = vec![record.clone()];
        Ok(record)
    }

    async fn resume_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.enter(lease, context, ControlCall::ResumeSandbox)
            .await?;
        self.sandbox_with_lifecycle(sandbox_id, AcaSandboxLifecycle::Running)
    }

    async fn stop_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError> {
        self.enter(lease, context, ControlCall::StopSandbox).await?;
        self.sandbox_with_lifecycle(sandbox_id, AcaSandboxLifecycle::Stopped)
    }

    async fn delete_sandbox(
        &self,
        lease: &AcaCredentialLease,
        context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError> {
        self.enter(lease, context, ControlCall::DeleteSandbox)
            .await?;
        let mut state = lock(&self.state);
        let before = state.sandboxes.len();
        state.sandboxes.retain(|record| record.id() != sandbox_id);
        Ok(if state.sandboxes.len() == before {
            AcaDeleteOutcome::AlreadyAbsent
        } else {
            AcaDeleteOutcome::Deleted
        })
    }
}

struct Harness {
    fixture: Fixture,
    configuration: AcaRuntimeConfig,
    provider: AzureContainerAppsRuntimeProvider,
    credential: Arc<FakeCredentialClient>,
    control: Arc<FakeAcaControl>,
    vault: Arc<Mutex<PrivateCredentialVault>>,
}

impl Harness {
    fn new(source: AcaDiskImageSource) -> Self {
        Self::new_at(source, 0, 1)
    }

    fn new_at(
        source: AcaDiskImageSource,
        runtime_ordinal: usize,
        credential_ordinal: usize,
    ) -> Self {
        let mut fixture = Fixture::new(ProviderType::Runtime, runtime_ordinal).unwrap();
        fixture.descriptor.implementation_id =
            ImplementationId::parse(ACA_IMPLEMENTATION_ID).unwrap();
        fixture.descriptor.capabilities =
            AzureContainerAppsRuntimeProvider::advertised_capabilities().unwrap();

        let mut credential_fixture =
            Fixture::new(ProviderType::Credential, credential_ordinal).unwrap();
        credential_fixture.descriptor.placement = fixture.descriptor.placement.clone();
        let vault = Arc::new(Mutex::new(PrivateCredentialVault::new()));
        let credential = Arc::new(FakeCredentialClient::new(
            credential_fixture.descriptor,
            Arc::clone(&vault),
        ));
        let control = Arc::new(FakeAcaControl::new(Arc::clone(&vault)));
        let profile = AcaSandboxProfile::new(
            AcaProfileId::parse("profile-one").unwrap(),
            source,
            AcaCpuMillis::new(1_000).unwrap(),
            AcaMemoryMib::new(2_048).unwrap(),
            600,
            Some(AcaManagedIdentityBindingId::parse("sandbox-mi").unwrap()),
        )
        .unwrap();
        let configuration =
            AcaRuntimeConfig::new(profile, AcaReadinessPolicy::new(3, 1).unwrap(), 60_000, 128)
                .unwrap();
        let provider = AzureContainerAppsRuntimeProvider::with_clock(
            fixture.descriptor.clone(),
            configuration.clone(),
            credential.clone(),
            control.clone(),
            Arc::new(DeterministicClock::new(NOW_UNIX_MS)),
        )
        .unwrap();
        Self {
            fixture,
            configuration,
            provider,
            credential,
            control,
            vault,
        }
    }

    fn container_image() -> Self {
        Self::new(AcaDiskImageSource::ConfiguredContainerImage {
            image_binding_id: AcaConfiguredImageId::parse("image-binding").unwrap(),
            disk_name: AcaDiskImageName::parse("disk-name").unwrap(),
            pull_identity_binding_id: Some(AcaManagedIdentityBindingId::parse("pull-mi").unwrap()),
        })
    }

    fn configured_disk() -> Self {
        Self::new(AcaDiskImageSource::ConfiguredDisk {
            binding_id: AcaConfiguredDiskId::parse("disk-binding").unwrap(),
        })
    }

    fn configured_disk_at(runtime_ordinal: usize, credential_ordinal: usize) -> Self {
        Self::new_at(
            AcaDiskImageSource::ConfiguredDisk {
                binding_id: AcaConfiguredDiskId::parse("disk-binding").unwrap(),
            },
            runtime_ordinal,
            credential_ordinal,
        )
    }

    fn factory_binding(&self) -> AcaRuntimeProviderBinding {
        AcaRuntimeProviderBinding::new(
            self.fixture.descriptor.clone(),
            self.configuration.clone(),
            self.credential.clone(),
            self.control.clone(),
        )
        .unwrap()
    }

    fn factory(&self) -> AzureContainerAppsRuntimeProviderFactory {
        AzureContainerAppsRuntimeProviderFactory::with_clock(
            vec![self.factory_binding()],
            Arc::new(DeterministicClock::new(NOW_UNIX_MS)),
        )
        .unwrap()
    }

    fn provider_with_clock(
        &self,
        clock: Arc<dyn ProviderClock>,
    ) -> AzureContainerAppsRuntimeProvider {
        AzureContainerAppsRuntimeProvider::with_clock(
            self.fixture.descriptor.clone(),
            self.configuration.clone(),
            self.credential.clone(),
            self.control.clone(),
            clock,
        )
        .unwrap()
    }

    fn operation(&self, method: ProviderMethod, label: &str) -> ProviderOperationContext {
        let mut operation = self.fixture.operation(method).unwrap();
        operation.operation_id = OperationId::parse(format!("aca-{label}")).unwrap();
        operation.idempotency_key = IdempotencyKey::parse(format!("idem-{label}")).unwrap();
        let seed = label.bytes().fold(17_u64, |value, byte| {
            value.wrapping_mul(37).wrapping_add(u64::from(byte))
        });
        operation.request_digest = fingerprint(seed);
        operation
    }

    fn request(&self, method: ProviderMethod, label: &str) -> ProviderOperationRequest {
        let mut request = self.fixture.request(method).unwrap();
        request.context = self.operation(method, label);
        request
    }

    fn handle_request(
        &self,
        method: ProviderMethod,
        label: &str,
        handle: &d2b_contracts::v2_provider::ProviderHandle,
    ) -> ProviderOperationRequest {
        let mut request = self.request(method, label);
        request.target = ProviderTarget::Handle {
            realm_id: handle.realm_id.clone(),
            workload_id: handle.workload_id.clone(),
            handle_id: handle.handle_id.clone(),
            handle_generation: handle.resource_generation,
        };
        request
    }

    fn call_context<'a>(
        &self,
        operation: &'a ProviderOperationContext,
        remaining_ms: u32,
        cancelled: bool,
    ) -> ProviderCallContext<'a> {
        ProviderCallContext {
            operation,
            peer_role: EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: remaining_ms,
            cancelled,
        }
    }
}

fn fingerprint(seed: u64) -> Fingerprint {
    Fingerprint::parse(format!("{seed:064x}")).unwrap()
}

async fn plan_and_ensure(
    harness: &Harness,
    label: &str,
) -> (
    d2b_contracts::v2_provider::ProviderPlan,
    d2b_contracts::v2_provider::ProviderHandle,
) {
    let plan_request = harness.request(ProviderMethod::RuntimePlan, &format!("{label}-plan"));
    let plan_context = harness.call_context(&plan_request.context, 1_000, false);
    let plan = harness
        .provider
        .plan(&plan_context, &plan_request)
        .await
        .unwrap();
    let ensure_operation =
        harness.operation(ProviderMethod::RuntimeEnsure, &format!("{label}-ensure"));
    let ensure_context = harness.call_context(&ensure_operation, 1_000, false);
    let handle = harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .unwrap();
    (plan, handle)
}

#[test]
fn capabilities_match_the_seven_implemented_methods_exactly() {
    let capabilities = AzureContainerAppsRuntimeProvider::advertised_capabilities().unwrap();
    let methods = capabilities
        .as_slice()
        .iter()
        .map(|capability| capability.0)
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        vec![
            ProviderMethod::RuntimePlan,
            ProviderMethod::RuntimeEnsure,
            ProviderMethod::RuntimeStart,
            ProviderMethod::RuntimeStop,
            ProviderMethod::RuntimeInspect,
            ProviderMethod::RuntimeAdopt,
            ProviderMethod::RuntimeDestroy,
        ]
    );
    assert!(!capabilities.contains_method(ProviderMethod::RuntimeExecute));

    let harness = Harness::container_image();
    assert!(harness.provider.capabilities() == capabilities);
    assert!(harness.provider.descriptor().capabilities == capabilities);
}

#[test]
fn factory_key_constructs_the_runtime_through_the_registry() {
    let harness = Harness::container_image();
    let key = aca_provider_factory_key().unwrap();
    assert_eq!(key.provider_type, ProviderType::Runtime);
    assert_eq!(key.implementation_id.as_str(), ACA_IMPLEMENTATION_ID);
    assert_eq!(
        AzureContainerAppsRuntimeProviderFactory::key().unwrap(),
        key
    );

    let factory = Arc::new(harness.factory());
    let instance = factory.construct(&harness.fixture.descriptor).unwrap();
    assert!(matches!(&instance, ProviderInstance::Runtime(_)));
    assert_eq!(instance.descriptor(), harness.fixture.descriptor);
    assert_eq!(
        instance.capabilities(),
        AzureContainerAppsRuntimeProvider::advertised_capabilities().unwrap()
    );

    let mut builder = ProviderRegistryBuilder::new(
        harness.fixture.descriptor.registry_generation,
        fingerprint(900),
        NOW_UNIX_MS,
    );
    builder.register_factory(key.clone(), factory).unwrap();
    builder
        .register_instance(harness.fixture.descriptor.clone())
        .unwrap();
    let registry = builder.finish().unwrap();
    assert_eq!(registry.snapshot().factories.as_slice(), [key]);
}

#[test]
fn factory_rejects_wrong_descriptor_type_before_external_calls() {
    let harness = Harness::container_image();
    let factory = harness.factory();
    let mut wrong_type = Fixture::new(ProviderType::Credential, 2)
        .unwrap()
        .descriptor;
    wrong_type.implementation_id = ImplementationId::parse(ACA_IMPLEMENTATION_ID).unwrap();

    assert!(matches!(
        factory.construct(&wrong_type),
        Err(FactoryError::Rejected)
    ));
    assert!(harness.control.calls().is_empty());
    assert_eq!(harness.credential.acquisition_count(), 0);
}

#[test]
fn factory_rejects_wrong_implementation_before_external_calls() {
    let harness = Harness::container_image();
    let factory = harness.factory();
    let mut wrong_implementation = harness.fixture.descriptor.clone();
    wrong_implementation.implementation_id = ImplementationId::parse("other-runtime").unwrap();

    assert!(matches!(
        factory.construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));
    assert!(harness.control.calls().is_empty());
    assert_eq!(harness.credential.acquisition_count(), 0);
}

#[test]
fn factory_rejects_unbound_or_reconfigured_descriptor_before_external_calls() {
    let harness = Harness::container_image();
    let factory = harness.factory();
    let mut mismatches = Vec::new();

    let mut configuration_schema = harness.fixture.descriptor.clone();
    configuration_schema.configuration_schema_fingerprint = fingerprint(901);
    mismatches.push(configuration_schema);

    let mut configured_scope = harness.fixture.descriptor.clone();
    configured_scope.configured_scope_digest = fingerprint(902);
    mismatches.push(configured_scope);

    let mut placement = harness.fixture.descriptor.clone();
    match &mut placement.placement {
        ProviderPlacement::ProviderAgent {
            agent_generation, ..
        } => *agent_generation = Generation::new(2).unwrap(),
        ProviderPlacement::TrustedFirstPartyInProcess { .. }
        | ProviderPlacement::UserAgent { .. } => unreachable!(),
    }
    mismatches.push(placement);

    let mut generation = harness.fixture.descriptor.clone();
    generation.registry_generation = Generation::new(2).unwrap();
    mismatches.push(generation);

    let mut unbound = Fixture::new(ProviderType::Runtime, 2).unwrap().descriptor;
    unbound.implementation_id = ImplementationId::parse(ACA_IMPLEMENTATION_ID).unwrap();
    unbound.capabilities = AzureContainerAppsRuntimeProvider::advertised_capabilities().unwrap();
    mismatches.push(unbound);

    for descriptor in mismatches {
        assert!(matches!(
            factory.construct(&descriptor),
            Err(FactoryError::Rejected)
        ));
    }
    assert!(harness.control.calls().is_empty());
    assert_eq!(harness.credential.acquisition_count(), 0);
}

#[test]
fn factory_rejects_empty_and_duplicate_binding_sets() {
    assert!(matches!(
        AzureContainerAppsRuntimeProviderFactory::new(Vec::new()),
        Err(AcaFactoryBuildError::EmptyBindings)
    ));

    let harness = Harness::container_image();
    let binding = harness.factory_binding();
    assert!(matches!(
        AzureContainerAppsRuntimeProviderFactory::new(vec![binding.clone(), binding]),
        Err(AcaFactoryBuildError::DuplicateProvider)
    ));
}

#[tokio::test]
async fn factory_selects_configuration_and_ports_by_exact_provider_id() {
    let first = Harness::container_image();
    let second = Harness::configured_disk_at(2, 3);
    let factory = AzureContainerAppsRuntimeProviderFactory::with_clock(
        vec![first.factory_binding(), second.factory_binding()],
        Arc::new(DeterministicClock::new(NOW_UNIX_MS)),
    )
    .unwrap();

    let first_instance = factory.construct(&first.fixture.descriptor).unwrap();
    let ProviderInstance::Runtime(first_provider) = first_instance else {
        unreachable!()
    };
    let first_operation = first.operation(ProviderMethod::RuntimeInspect, "factory-first");
    let first_context = first.call_context(&first_operation, 1_000, false);
    first_provider.health(&first_context).await.unwrap();
    assert_eq!(first.control.calls(), [ControlCall::Health]);
    assert!(second.control.calls().is_empty());
    assert_eq!(first.credential.acquisition_count(), 1);
    assert_eq!(second.credential.acquisition_count(), 0);

    let second_instance = factory.construct(&second.fixture.descriptor).unwrap();
    let ProviderInstance::Runtime(second_provider) = second_instance else {
        unreachable!()
    };
    let second_operation = second.operation(ProviderMethod::RuntimeInspect, "factory-second");
    let second_context = second.call_context(&second_operation, 1_000, false);
    second_provider.health(&second_context).await.unwrap();
    assert_eq!(second.control.calls(), [ControlCall::Health]);
    assert_eq!(second.credential.acquisition_count(), 1);
}

#[tokio::test]
async fn live_lifecycle_uses_opaque_leases_and_replays_completed_operations() {
    let harness = Harness::container_image();

    let health_operation = harness.operation(ProviderMethod::RuntimeInspect, "health");
    let health_context = harness.call_context(&health_operation, 1_000, false);
    let health = harness.provider.health(&health_context).await.unwrap();
    assert_eq!(health.state, ProviderHealthState::Healthy);

    let plan_request = harness.request(ProviderMethod::RuntimePlan, "lifecycle-plan");
    let plan_context = harness.call_context(&plan_request.context, 1_000, false);
    let plan = harness
        .provider
        .plan(&plan_context, &plan_request)
        .await
        .unwrap();
    assert_eq!(
        plan.resources.as_slice(),
        [d2b_contracts::v2_provider::PlannedResourceClass::WorkloadExecution]
    );

    let ensure_operation = harness.operation(ProviderMethod::RuntimeEnsure, "lifecycle-ensure");
    let ensure_context = harness.call_context(&ensure_operation, 1_000, false);
    let handle = harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .unwrap();
    assert_eq!(handle.handle_id.as_str(), "aca-sandbox-one");
    let calls_after_ensure = harness.control.calls();
    let acquisitions_after_ensure = harness.credential.acquisition_count();
    let replayed = harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .unwrap();
    assert!(replayed == handle);
    assert_eq!(harness.control.calls(), calls_after_ensure);
    assert_eq!(
        harness.credential.acquisition_count(),
        acquisitions_after_ensure
    );

    let start_request =
        harness.handle_request(ProviderMethod::RuntimeStart, "lifecycle-start", &handle);
    let start_context = harness.call_context(&start_request.context, 1_000, false);
    let started = harness
        .provider
        .start(&start_context, &start_request)
        .await
        .unwrap();
    assert_eq!(started.lifecycle, ObservedLifecycleState::Running);

    let inspect_request =
        harness.handle_request(ProviderMethod::RuntimeInspect, "lifecycle-inspect", &handle);
    let inspect_context = harness.call_context(&inspect_request.context, 1_000, false);
    let inspected = harness
        .provider
        .inspect(&inspect_context, &inspect_request)
        .await
        .unwrap();
    assert_eq!(inspected.lifecycle, ObservedLifecycleState::Running);

    let adopt_operation = harness.operation(ProviderMethod::RuntimeAdopt, "lifecycle-adopt");
    let adoption = AdoptionRequest {
        context: adopt_operation,
        handle: handle.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: handle.resource_generation,
    };
    let adopt_context = harness.call_context(&adoption.context, 1_000, false);
    let adopted = harness
        .provider
        .adopt(&adopt_context, &adoption)
        .await
        .unwrap();
    assert_eq!(adopted.adoption, AdoptionState::Adopted);
    assert_eq!(adopted.lifecycle, ObservedLifecycleState::Running);

    let stop_request =
        harness.handle_request(ProviderMethod::RuntimeStop, "lifecycle-stop", &handle);
    let stop_context = harness.call_context(&stop_request.context, 1_000, false);
    let stopped = harness
        .provider
        .stop(&stop_context, &stop_request)
        .await
        .unwrap();
    assert_eq!(stopped.lifecycle, ObservedLifecycleState::Stopped);

    let destroy_request =
        harness.handle_request(ProviderMethod::RuntimeDestroy, "lifecycle-destroy", &handle);
    let destroy_context = harness.call_context(&destroy_request.context, 1_000, false);
    let destroyed = harness
        .provider
        .destroy(&destroy_context, &destroy_request)
        .await
        .unwrap();
    assert_eq!(destroyed.state, MutationState::Applied);

    let absent_request = harness.handle_request(
        ProviderMethod::RuntimeInspect,
        "lifecycle-inspect-absent",
        &handle,
    );
    let absent_context = harness.call_context(&absent_request.context, 1_000, false);
    let absent = harness
        .provider
        .inspect(&absent_context, &absent_request)
        .await
        .unwrap();
    assert_eq!(absent.lifecycle, ObservedLifecycleState::Destroyed);

    assert_eq!(
        harness.control.calls(),
        vec![
            ControlCall::Health,
            ControlCall::FindSandboxes,
            ControlCall::FindDiskImages,
            ControlCall::CreateDiskImage,
            ControlCall::CreateSandbox,
            ControlCall::FindSandboxes,
            ControlCall::ResumeSandbox,
            ControlCall::FindSandboxes,
            ControlCall::FindSandboxes,
            ControlCall::FindSandboxes,
            ControlCall::StopSandbox,
            ControlCall::FindSandboxes,
            ControlCall::DeleteSandbox,
            ControlCall::FindSandboxes,
        ]
    );
    assert_eq!(
        lock(&harness.vault).redemptions,
        harness.control.calls().len()
    );
    assert_eq!(
        harness.credential.revocation_count(),
        harness.credential.acquisition_count()
    );
    assert_eq!(
        harness.credential.completed_revocation_count(),
        harness.credential.acquisition_count()
    );
    assert_eq!(harness.credential.cancelled_revocation_count(), 0);
    assert_eq!(lock(&harness.vault).active_lease_count(), 0);
}

#[tokio::test]
async fn configured_disk_is_resolved_without_image_creation() {
    let harness = Harness::configured_disk();
    let (_, handle) = plan_and_ensure(&harness, "configured-disk").await;
    assert_eq!(handle.handle_id.as_str(), "aca-sandbox-one");
    assert_eq!(
        harness.control.calls(),
        vec![
            ControlCall::FindSandboxes,
            ControlCall::ResolveConfiguredDisk,
            ControlCall::CreateSandbox,
        ]
    );
}

#[tokio::test]
async fn validation_denies_method_input_scope_and_peer_before_external_calls() {
    let harness = Harness::container_image();

    let wrong_method = harness.request(ProviderMethod::RuntimeStart, "deny-method");
    let wrong_method_context = harness.call_context(&wrong_method.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&wrong_method_context, &wrong_method)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    let mut wrong_input = harness.request(ProviderMethod::RuntimeInspect, "deny-input");
    wrong_input.input = ProviderOperationInput::ConfiguredRuntimeExecution {
        configured_item_id: ConfiguredItemId::parse("configured-command").unwrap(),
    };
    let wrong_input_context = harness.call_context(&wrong_input.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&wrong_input_context, &wrong_input)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);

    let mut wrong_scope = harness.request(ProviderMethod::RuntimeInspect, "deny-scope");
    wrong_scope.target = ProviderTarget::Workload {
        realm_id: wrong_scope.target.realm_id().clone(),
        workload_id: WorkloadId::parse("eeeeeeeeeeeeeeeeeeea").unwrap(),
    };
    let wrong_scope_context = harness.call_context(&wrong_scope.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&wrong_scope_context, &wrong_scope)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::UnauthorizedScope);

    let wrong_peer = harness.request(ProviderMethod::RuntimeInspect, "deny-peer");
    let mut wrong_peer_context = harness.call_context(&wrong_peer.context, 1_000, false);
    wrong_peer_context.peer_role = EndpointRole::ProviderAgent;
    let failure = harness
        .provider
        .inspect(&wrong_peer_context, &wrong_peer)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::UnauthorizedScope);

    let plan_request = harness.request(ProviderMethod::RuntimePlan, "deny-plan");
    let plan_context = harness.call_context(&plan_request.context, 1_000, false);
    let mut plan = harness
        .provider
        .plan(&plan_context, &plan_request)
        .await
        .unwrap();
    plan.schema_version += 1;
    let ensure_operation = harness.operation(ProviderMethod::RuntimeEnsure, "deny-plan-ensure");
    let ensure_context = harness.call_context(&ensure_operation, 1_000, false);
    let failure = harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);

    assert_eq!(harness.credential.acquisition_count(), 0);
    assert!(harness.control.calls().is_empty());
}

#[tokio::test]
async fn operation_id_reuse_cannot_widen_authorized_scope() {
    let harness = Harness::container_image();
    let request = harness.request(ProviderMethod::RuntimeInspect, "scope-replay");
    let context = harness.call_context(&request.context, 1_000, false);
    let observation = harness.provider.inspect(&context, &request).await.unwrap();
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Destroyed);
    let calls = harness.control.calls();
    let acquisitions = harness.credential.acquisition_count();

    let other_workload = WorkloadId::parse("eeeeeeeeeeeeeeeeeeea").unwrap();
    let mut widened = request;
    widened.context.scope = AuthorizedProviderScope::Workload {
        realm_id: widened.context.scope.realm_id().clone(),
        workload_id: other_workload.clone(),
    };
    widened.target = ProviderTarget::Workload {
        realm_id: widened.target.realm_id().clone(),
        workload_id: other_workload,
    };
    let widened_context = harness.call_context(&widened.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&widened_context, &widened)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(harness.control.calls(), calls);
    assert_eq!(harness.credential.acquisition_count(), acquisitions);
}

#[tokio::test]
async fn adoption_rejects_identity_configuration_generation_and_operation_mismatch() {
    let harness = Harness::container_image();
    let (_, handle) = plan_and_ensure(&harness, "adoption").await;
    let original = harness.control.sandboxes().remove(0);
    let base = original.binding();
    let other_provider = ProviderId::parse("eeeeeeeeeeeeeeeeeeea").unwrap();
    let mut wrong_created_by = base.created_by().clone();
    wrong_created_by.operation_id = OperationId::parse("aca-wrong-provenance").unwrap();
    let mismatches = [
        (
            AcaResourceBinding::new(
                base.realm_id().clone(),
                base.workload_id().clone(),
                other_provider,
                base.provider_generation(),
                base.configuration_fingerprint().clone(),
                base.resource_generation(),
                base.created_by().clone(),
            ),
            ObservationReason::IdentityMismatch,
        ),
        (
            AcaResourceBinding::new(
                base.realm_id().clone(),
                base.workload_id().clone(),
                base.provider_id().clone(),
                base.provider_generation(),
                fingerprint(9_999),
                base.resource_generation(),
                base.created_by().clone(),
            ),
            ObservationReason::ConfigurationMismatch,
        ),
        (
            AcaResourceBinding::new(
                base.realm_id().clone(),
                base.workload_id().clone(),
                base.provider_id().clone(),
                Generation::new(2).unwrap(),
                base.configuration_fingerprint().clone(),
                Generation::new(2).unwrap(),
                base.created_by().clone(),
            ),
            ObservationReason::GenerationMismatch,
        ),
        (
            AcaResourceBinding::new(
                base.realm_id().clone(),
                base.workload_id().clone(),
                base.provider_id().clone(),
                base.provider_generation(),
                base.configuration_fingerprint().clone(),
                base.resource_generation(),
                wrong_created_by,
            ),
            ObservationReason::IdentityMismatch,
        ),
    ];

    for (index, (binding, expected_reason)) in mismatches.into_iter().enumerate() {
        harness.control.set_sandboxes(vec![AcaSandboxRecord::new(
            original.id().clone(),
            binding,
            AcaSandboxLifecycle::Running,
        )]);
        let operation = harness.operation(
            ProviderMethod::RuntimeAdopt,
            &format!("adopt-mismatch-{index}"),
        );
        let request = AdoptionRequest {
            context: operation,
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: handle.resource_generation,
        };
        let context = harness.call_context(&request.context, 1_000, false);
        let observation = harness.provider.adopt(&context, &request).await.unwrap();
        assert_eq!(observation.adoption, AdoptionState::Rejected);
        assert_eq!(observation.reason, expected_reason);
        assert_eq!(observation.lifecycle, ObservedLifecycleState::Quarantined);
    }
    assert!(!harness.control.calls().iter().any(|call| matches!(
        call,
        ControlCall::ResumeSandbox | ControlCall::StopSandbox | ControlCall::DeleteSandbox
    )));
}

#[tokio::test]
async fn cancellation_and_deadline_fail_closed_and_same_operation_can_retry() {
    let harness = Harness::container_image();
    harness.control.stall_next(ControlCall::FindSandboxes);
    let request = harness.request(ProviderMethod::RuntimeInspect, "deadline-retry");
    let short_context = harness.call_context(&request.context, 20, false);
    let failure = harness
        .provider
        .inspect(&short_context, &request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(failure.retry, RetryClass::SameOperation);
    assert_eq!(harness.control.cancelled_call_count(), 1);
    assert_eq!(harness.credential.revocation_count(), 1);

    let retry_context = harness.call_context(&request.context, 1_000, false);
    let observation = harness
        .provider
        .inspect(&retry_context, &request)
        .await
        .unwrap();
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Destroyed);
    assert_eq!(harness.credential.revocation_count(), 2);
    assert_eq!(
        harness.control.calls(),
        vec![ControlCall::FindSandboxes, ControlCall::FindSandboxes]
    );

    let before_credentials = harness.credential.acquisition_count();
    let before_calls = harness.control.calls();
    let cancelled_request = harness.request(ProviderMethod::RuntimeInspect, "cancelled");
    let cancelled_context = harness.call_context(&cancelled_request.context, 1_000, true);
    let failure = harness
        .provider
        .inspect(&cancelled_context, &cancelled_request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);

    let expired_request = harness.request(ProviderMethod::RuntimeInspect, "zero-deadline");
    let expired_context = harness.call_context(&expired_request.context, 0, false);
    let failure = harness
        .provider
        .inspect(&expired_context, &expired_request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(harness.credential.acquisition_count(), before_credentials);
    assert_eq!(harness.control.calls(), before_calls);
    assert_eq!(harness.credential.revocation_count(), 2);
}

#[tokio::test]
async fn external_budget_is_the_minimum_of_wall_and_monotonic_deadlines() {
    let wall_limited = Harness::container_image();
    let mut wall_request = wall_limited.request(ProviderMethod::RuntimeInspect, "wall-deadline");
    wall_request.context.expires_at_unix_ms = NOW_UNIX_MS + 250;
    let wall_context = wall_limited.call_context(&wall_request.context, 5_000, false);
    wall_limited
        .provider
        .inspect(&wall_context, &wall_request)
        .await
        .unwrap();
    let wall_deadlines = wall_limited.control.deadlines_ms();
    assert_eq!(wall_deadlines.len(), 1);
    assert!((1..=250).contains(&wall_deadlines[0]));
    let wall_lease_expiry = wall_limited
        .credential
        .last_metadata()
        .unwrap()
        .expires_at_unix_ms;
    assert!(wall_lease_expiry > NOW_UNIX_MS);
    assert!(wall_lease_expiry <= NOW_UNIX_MS + 250);

    let monotonic_limited = Harness::configured_disk();
    let monotonic_request =
        monotonic_limited.request(ProviderMethod::RuntimeInspect, "monotonic-deadline");
    let monotonic_context = monotonic_limited.call_context(&monotonic_request.context, 200, false);
    monotonic_limited
        .provider
        .inspect(&monotonic_context, &monotonic_request)
        .await
        .unwrap();
    let monotonic_deadlines = monotonic_limited.control.deadlines_ms();
    assert_eq!(monotonic_deadlines.len(), 1);
    assert!((1..=200).contains(&monotonic_deadlines[0]));
    let monotonic_lease_expiry = monotonic_limited
        .credential
        .last_metadata()
        .unwrap()
        .expires_at_unix_ms;
    assert!(monotonic_lease_expiry > NOW_UNIX_MS);
    assert!(monotonic_lease_expiry <= NOW_UNIX_MS + 200);
    assert_eq!(lock(&wall_limited.vault).redemptions, 1);
    assert_eq!(lock(&monotonic_limited.vault).redemptions, 1);
}

#[tokio::test]
async fn forward_clock_jumps_fail_before_credential_and_control_effects() {
    let credential_jump = Harness::container_image();
    let credential_request =
        credential_jump.request(ProviderMethod::RuntimeInspect, "credential-clock-jump");
    let credential_clock = Arc::new(JumpOnCallClock::new(
        NOW_UNIX_MS,
        credential_request.context.expires_at_unix_ms,
        5,
    ));
    let credential_provider = credential_jump.provider_with_clock(credential_clock);
    let credential_context =
        credential_jump.call_context(&credential_request.context, 1_000, false);
    let failure = credential_provider
        .inspect(&credential_context, &credential_request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(credential_jump.credential.acquisition_count(), 0);
    assert!(credential_jump.control.calls().is_empty());

    let control_jump = Harness::configured_disk();
    let control_request =
        control_jump.request(ProviderMethod::RuntimeInspect, "control-clock-jump");
    let control_clock = Arc::new(JumpOnCallClock::new(
        NOW_UNIX_MS,
        control_request.context.expires_at_unix_ms,
        8,
    ));
    let control_provider = control_jump.provider_with_clock(control_clock);
    let control_context = control_jump.call_context(&control_request.context, 1_000, false);
    let failure = control_provider
        .inspect(&control_context, &control_request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(control_jump.credential.acquisition_count(), 1);
    assert_eq!(control_jump.credential.revocation_count(), 1);
    assert!(control_jump.control.calls().is_empty());
}

#[tokio::test]
async fn first_mutation_expiry_before_dispatch_is_not_ambiguous() {
    let harness = Harness::configured_disk();
    let (_, handle) = plan_and_ensure(&harness, "pre-dispatch-expiry").await;
    let calls_before = harness.control.calls().len();
    let revocations_before = harness.credential.revocation_count();
    let request = harness.handle_request(
        ProviderMethod::RuntimeDestroy,
        "pre-dispatch-expiry-delete",
        &handle,
    );
    let clock = Arc::new(JumpOnCallClock::new(
        NOW_UNIX_MS,
        request.context.expires_at_unix_ms,
        9,
    ));
    let provider = harness.provider_with_clock(clock);
    let context = harness.call_context(&request.context, 1_000, false);

    let failure = provider.destroy(&context, &request).await.unwrap_err();

    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(failure.retry, RetryClass::SameOperation);
    assert_eq!(
        &harness.control.calls()[calls_before..],
        &[ControlCall::FindSandboxes]
    );
    assert_eq!(harness.control.cancelled_call_count(), 0);
    assert_eq!(
        harness.credential.revocation_count(),
        revocations_before + 1
    );
    assert_eq!(harness.control.sandboxes().len(), 1);
}

#[tokio::test]
async fn first_mutation_timeout_after_dispatch_is_ambiguous() {
    let harness = Harness::configured_disk();
    let (_, handle) = plan_and_ensure(&harness, "post-dispatch-timeout").await;
    let calls_before = harness.control.calls().len();
    harness.control.stall_next(ControlCall::DeleteSandbox);
    let request = harness.handle_request(
        ProviderMethod::RuntimeDestroy,
        "post-dispatch-timeout-delete",
        &handle,
    );
    let context = harness.call_context(&request.context, 20, false);

    let failure = harness
        .provider
        .destroy(&context, &request)
        .await
        .unwrap_err();

    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    assert_eq!(
        &harness.control.calls()[calls_before..],
        &[ControlCall::FindSandboxes, ControlCall::DeleteSandbox]
    );
    assert_eq!(harness.control.cancelled_call_count(), 1);
}

#[tokio::test]
async fn successful_operation_revokes_its_opaque_lease_once() {
    let harness = Harness::container_image();
    let request = harness.request(ProviderMethod::RuntimeInspect, "success-revoke");
    let context = harness.call_context(&request.context, 1_000, false);

    let observation = harness.provider.inspect(&context, &request).await.unwrap();

    assert_eq!(observation.lifecycle, ObservedLifecycleState::Destroyed);
    assert_eq!(harness.credential.acquisition_count(), 1);
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 1);
    assert_eq!(harness.credential.cancelled_revocation_count(), 0);
    assert_eq!(lock(&harness.vault).active_lease_count(), 0);
}

#[tokio::test]
async fn synchronous_revoke_success_preserves_the_operation_failure() {
    let harness = Harness::container_image();
    harness.control.fail_next(
        ControlCall::FindSandboxes,
        AcaControlError::closed(
            AcaControlErrorKind::Unavailable,
            AcaDiagnosticCode::ServiceUnavailable,
        ),
    );
    let request = harness.request(ProviderMethod::RuntimeInspect, "failure-revoke");
    let context = harness.call_context(&request.context, 1_000, false);

    let failure = harness
        .provider
        .inspect(&context, &request)
        .await
        .unwrap_err();

    assert_eq!(failure.kind, ProviderFailureKind::Unavailable);
    assert_eq!(failure.retry, RetryClass::SameOperation);
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 1);
    assert_eq!(harness.credential.cancelled_revocation_count(), 0);
    assert_eq!(lock(&harness.vault).active_lease_count(), 0);
}

#[tokio::test]
async fn revoke_timeout_is_typed_ambiguity_without_a_second_attempt() {
    let harness = Harness::container_image();
    harness.credential.timeout_next_revoke();
    let request = harness.request(ProviderMethod::RuntimeInspect, "revoke-timeout");
    let context = harness.call_context(&request.context, 2_000, false);

    let failure = harness
        .provider
        .inspect(&context, &request)
        .await
        .unwrap_err();

    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 0);
    assert_eq!(harness.credential.cancelled_revocation_count(), 1);
    tokio::task::yield_now().await;
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(lock(&harness.vault).active_lease_count(), 1);
}

#[tokio::test]
async fn dropping_an_in_flight_call_revokes_the_opaque_lease_once() {
    let harness = Harness::container_image();
    harness.control.stall_next(ControlCall::FindSandboxes);
    let request = harness.request(ProviderMethod::RuntimeInspect, "drop-in-flight");
    let context = harness.call_context(&request.context, 1_000, false);
    let mut call = harness.provider.inspect(&context, &request);

    tokio::select! {
        result = &mut call => panic!("provider call completed before drop: {result:?}"),
        () = async {
            while harness.control.calls().is_empty() {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    assert_eq!(harness.credential.acquisition_count(), 1);
    assert_eq!(harness.credential.revocation_count(), 0);
    drop(call);

    tokio::time::timeout(Duration::from_millis(100), async {
        while harness.credential.revocation_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 1);
    assert_eq!(harness.credential.cancelled_revocation_count(), 0);
    assert_eq!(lock(&harness.vault).active_lease_count(), 0);
}

#[tokio::test]
async fn dropping_revoke_future_keeps_exactly_one_background_completion() {
    let harness = Harness::container_image();
    harness.control.fail_next(
        ControlCall::FindSandboxes,
        AcaControlError::closed(AcaControlErrorKind::Unavailable, AcaDiagnosticCode::Unknown),
    );
    harness.credential.stall_next_revoke();
    let request = harness.request(ProviderMethod::RuntimeInspect, "drop-mid-revoke");
    let context = harness.call_context(&request.context, 1_000, false);
    let mut call = harness.provider.inspect(&context, &request);

    tokio::select! {
        result = &mut call => panic!("provider call completed before revoke drop: {result:?}"),
        () = async {
            while harness.credential.revocation_count() == 0 {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 0);
    drop(call);

    tokio::time::timeout(Duration::from_millis(100), async {
        while harness.credential.completed_revocation_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(harness.credential.cancelled_revocation_count(), 0);
    assert_eq!(harness.credential.revocation_count(), 1);
    assert_eq!(harness.credential.completed_revocation_count(), 1);
    assert_eq!(lock(&harness.vault).active_lease_count(), 0);
}

#[tokio::test]
async fn uncertain_lease_revoke_returns_typed_ambiguity_without_retrying_cleanup() {
    let harness = Harness::container_image();
    harness.control.fail_next(
        ControlCall::FindSandboxes,
        AcaControlError::closed(AcaControlErrorKind::Unavailable, AcaDiagnosticCode::Unknown),
    );
    harness.credential.fail_next_revoke(AcaControlError::closed(
        AcaControlErrorKind::Ambiguous,
        AcaDiagnosticCode::Unknown,
    ));
    let request = harness.request(ProviderMethod::RuntimeInspect, "revoke-ambiguous");
    let context = harness.call_context(&request.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&context, &request)
        .await
        .unwrap_err();

    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    assert_eq!(harness.credential.revocation_count(), 1);
}

#[tokio::test]
async fn ambiguous_mutation_requires_observation_before_same_operation_retry() {
    let harness = Harness::container_image();
    let (_, handle) = plan_and_ensure(&harness, "ambiguous").await;
    harness.control.stall_next(ControlCall::DeleteSandbox);
    let request =
        harness.handle_request(ProviderMethod::RuntimeDestroy, "ambiguous-delete", &handle);
    let short_context = harness.call_context(&request.context, 20, false);
    let failure = harness
        .provider
        .destroy(&short_context, &request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    let calls = harness.control.calls();
    let acquisitions = harness.credential.acquisition_count();

    let replay_context = harness.call_context(&request.context, 1_000, false);
    let replayed = harness
        .provider
        .destroy(&replay_context, &request)
        .await
        .unwrap_err();
    assert!(replayed == failure);
    assert_eq!(harness.control.calls(), calls);
    assert_eq!(harness.credential.acquisition_count(), acquisitions);

    let inspect_request =
        harness.handle_request(ProviderMethod::RuntimeInspect, "ambiguous-inspect", &handle);
    let inspect_context = harness.call_context(&inspect_request.context, 1_000, false);
    let observation = harness
        .provider
        .inspect(&inspect_context, &inspect_request)
        .await
        .unwrap();
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Stopped);

    let completed = harness
        .provider
        .destroy(&replay_context, &request)
        .await
        .unwrap();
    assert_eq!(completed.state, MutationState::Applied);
    assert_eq!(
        &harness.control.calls()[calls.len()..],
        &[
            ControlCall::FindSandboxes,
            ControlCall::FindSandboxes,
            ControlCall::DeleteSandbox,
        ]
    );
}

#[tokio::test]
async fn sdk_cancellation_after_mutation_dispatch_is_ambiguous() {
    let harness = Harness::container_image();
    let (_, handle) = plan_and_ensure(&harness, "sdk-cancel").await;
    assert_eq!(harness.credential.revocation_count(), 1);
    harness.control.fail_next(
        ControlCall::DeleteSandbox,
        AcaControlError::closed(AcaControlErrorKind::Cancelled, AcaDiagnosticCode::Unknown),
    );
    let request =
        harness.handle_request(ProviderMethod::RuntimeDestroy, "sdk-cancel-delete", &handle);
    let context = harness.call_context(&request.context, 1_000, false);
    let failure = harness
        .provider
        .destroy(&context, &request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
    assert_eq!(harness.credential.revocation_count(), 2);
}

#[tokio::test]
async fn rate_limit_and_credential_failure_use_closed_retry_classes_without_fallback() {
    let harness = Harness::container_image();
    harness.control.fail_next(
        ControlCall::FindSandboxes,
        AcaControlError::new(
            AcaControlErrorKind::RateLimited,
            AcaDiagnosticCode::TooManyRequests,
            Some(25),
        )
        .unwrap(),
    );
    let request = harness.request(ProviderMethod::RuntimeInspect, "rate-limited");
    let context = harness.call_context(&request.context, 1_000, false);
    let failure = harness
        .provider
        .inspect(&context, &request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::Unavailable);
    assert_eq!(failure.retry, RetryClass::SameOperation);
    assert_eq!(harness.credential.revocation_count(), 1);
    let observation = harness.provider.inspect(&context, &request).await.unwrap();
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Destroyed);
    assert_eq!(harness.credential.revocation_count(), 2);

    let no_fallback = Harness::container_image();
    no_fallback.credential.fail_next(AcaControlError::closed(
        AcaControlErrorKind::Authentication,
        AcaDiagnosticCode::AuthenticationFailed,
    ));
    let request = no_fallback.request(ProviderMethod::RuntimeInspect, "credential-denied");
    let context = no_fallback.call_context(&request.context, 1_000, false);
    let failure = no_fallback
        .provider
        .inspect(&context, &request)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ProviderFailureKind::CredentialLeaseInvalid);
    assert_eq!(failure.retry, RetryClass::AfterInteraction);
    assert_eq!(no_fallback.credential.acquisition_count(), 1);
    assert_eq!(no_fallback.credential.revocation_count(), 0);
    assert!(no_fallback.control.calls().is_empty());
}

#[test]
fn bounds_and_diagnostics_are_closed_and_secret_free() {
    assert_eq!(
        AcaSandboxId::parse("https-endpoint/path").unwrap_err(),
        AcaTypeError::InvalidIdentifier
    );
    assert_eq!(
        AcaCpuMillis::new(251).unwrap_err(),
        AcaTypeError::InvalidResourceBounds
    );
    assert_eq!(
        AcaReadinessPolicy::new(0, 1).unwrap_err(),
        AcaTypeError::InvalidReadinessPolicy
    );
    assert_eq!(
        AcaControlError::new(
            AcaControlErrorKind::RateLimited,
            AcaDiagnosticCode::TooManyRequests,
            Some(MAX_ACA_RETRY_AFTER_MS + 1),
        )
        .unwrap_err(),
        AcaControlErrorBuildError::RetryBoundExceeded
    );
    assert_eq!(
        AcaControlError::new(
            AcaControlErrorKind::Unavailable,
            AcaDiagnosticCode::ServiceUnavailable,
            Some(1),
        )
        .unwrap_err(),
        AcaControlErrorBuildError::RetryNotApplicable
    );

    let harness = Harness::container_image();
    let binding = AcaResourceBinding::new(
        harness.fixture.descriptor.placement.realm_id().clone(),
        match &harness.fixture.descriptor.placement {
            ProviderPlacement::ProviderAgent { workload_id, .. } => workload_id.clone(),
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
            | ProviderPlacement::UserAgent { .. } => unreachable!(),
        },
        harness.fixture.descriptor.provider_id.clone(),
        harness.fixture.descriptor.registry_generation,
        harness
            .fixture
            .descriptor
            .configuration_schema_fingerprint
            .clone(),
        Generation::new(1).unwrap(),
        harness
            .operation(ProviderMethod::RuntimePlan, "bounded-record")
            .binding(),
    );
    let record = AcaSandboxRecord::new(
        AcaSandboxId::parse("bounded-record").unwrap(),
        binding,
        AcaSandboxLifecycle::Running,
    );
    assert_eq!(
        AcaSandboxCandidates::new(vec![record; MAX_ACA_CANDIDATES + 1]).unwrap_err(),
        AcaTypeError::CandidateBoundExceeded
    );

    let error = AcaControlError::closed(
        AcaControlErrorKind::Authentication,
        AcaDiagnosticCode::AuthenticationFailed,
    );
    let rendered = format!(
        "{:?} {:?} {:?} {:?}",
        harness.provider, harness.configuration, harness.vault, error
    );
    assert!(!rendered.contains(SECRET_CANARY));
    assert!(!rendered.contains("https://"));
    assert!(!rendered.contains("/"));
}

#[tokio::test]
async fn lease_and_request_debug_are_opaque_and_canary_free() {
    let harness = Harness::container_image();
    let request = harness.request(ProviderMethod::RuntimeInspect, "redaction");
    let context = harness.call_context(&request.context, 1_000, false);
    let _ = harness.provider.inspect(&context, &request).await.unwrap();
    let metadata = harness.credential.last_metadata().unwrap();
    let lease = AcaCredentialLease::from_canonical(metadata);
    let lease_request = AcaCredentialLeaseRequest::new(
        request.context,
        AcaCredentialPurpose::Inspect,
        NOW_UNIX_MS + 1_000,
    );
    let rendered = format!("{lease:?} {lease_request:?} {:?}", harness.provider);
    assert!(rendered.contains("AcaCredentialLease(<opaque>)"));
    assert!(!rendered.contains(SECRET_CANARY));
}

#[test]
fn construction_requires_exact_credential_colocation() {
    let harness = Harness::container_image();
    let mut overclaimed = harness.fixture.descriptor.clone();
    let mut capabilities = overclaimed.capabilities.as_slice().to_vec();
    capabilities.push(ProviderCapability(ProviderMethod::RuntimeExecute));
    overclaimed.capabilities = ProviderCapabilitySet::new(capabilities).unwrap();
    let result = AzureContainerAppsRuntimeProvider::new(
        overclaimed,
        harness.configuration.clone(),
        harness.credential.clone(),
        harness.control.clone(),
    );
    assert!(matches!(
        result,
        Err(AcaProviderBuildError::CapabilityMismatch)
    ));

    let mut remote_credential = Fixture::new(ProviderType::Credential, 3)
        .unwrap()
        .descriptor;
    remote_credential.provider_id = harness.credential.descriptor.provider_id.clone();
    if let ProviderPlacement::ProviderAgent { workload_id, .. } = &mut remote_credential.placement {
        *workload_id = WorkloadId::parse("eeeeeeeeeeeeeeeeeeea").unwrap();
    }
    let credential = Arc::new(FakeCredentialClient::new(
        remote_credential,
        Arc::clone(&harness.vault),
    ));
    let result = AzureContainerAppsRuntimeProvider::new(
        harness.fixture.descriptor,
        harness.configuration,
        credential,
        harness.control,
    );
    assert!(matches!(
        result,
        Err(AcaProviderBuildError::CredentialNotColocated)
    ));
}
