//! Shared-Runner composition for the Guest runtime Provider family.
//!
//! The runtime Providers (cloud-hypervisor, qemu-media, azure container apps,
//! azure virtual machine) run through one closed, typed effect boundary; every
//! registration row names the exact controller process, Provider identity,
//! ResourceType, and finalizer it owns. The registration shape and the effect
//! trait live here rather than in the orchestrator.
//!
//! The U8 host Provider family, the interaction/shell family (U9), and the
//! storage family (U7) were converted to v3 drivers; their arms left this
//! module with their reconcilers. What remains is the Guest leg consumed by
//! `guest_provider_runtime`.

use std::{
    collections::BTreeMap,
    sync::Arc,
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceGeneration, ResourceRef, ResourceTypeName,
    ResourceUid, ZoneId,
    identity::{AuthenticatedSubjectContext, ReconnectGeneration},
};
use d2b_core_controller::{
    ControllerDescriptor, ControllerExecutionPolicy, ControllerIdentity, ControllerSelector,
    ControllerVerb, DependencySnapshot, DisruptionClass, DrainResult, FinalizeResult,
    ObservationResult, OwnedChildIntent, ReconcileContext, ReconcileDisposition, ReconcilePlan,
    ReconcileReason, ReconcileResult, ResourceKey, ResourceMutationBatch, ResourceReconciler,
    ResourceRegistration, ResourceSnapshot, ResyncPolicy, SelectorField, StatusPersistence,
    UpdateAssessment, UpdateAssessmentState, UpgradePlan, UpgradeStage, ValidationResult,
};
use d2b_provider_runtime_azure_container_apps as aca_runtime;
use d2b_provider_runtime_azure_virtual_machine as azure_vm_runtime;
use d2b_provider_runtime_qemu_media as qemu_media_runtime;
use d2b_resource_store::{
    StoreGetRequest, StoreOperationContext, StoreProjection, StoredResource,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ServerState;
use crate::binding_child_resource_runtime::{
    OneOwnedChildProgress, OwnedChildOwner, reconcile_one_guest_child,
};

use super::{
    CORE_CONTROLLER_HOST_REF, CloudHypervisorReconcileOutcome, ResourceRuntimeError,
    SharedProviderRunnerRegistration, ZoneResourceRuntime,
};

/// Closed Provider handler set used by the shared Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderResourceKind {
    CloudHypervisorGuest,
    QemuMediaGuest,
    AzureContainerAppsGuest,
    AzureVirtualMachineGuest,
}

impl SharedProviderResourceKind {
    pub(super) fn from_registration(
        registration: SharedProviderRunnerRegistration,
    ) -> Result<Self, ResourceRuntimeError> {
        match (
            registration.provider_ref,
            registration.resource_type,
            registration.controller_ref,
        ) {
            (
                "Provider/runtime-cloud-hypervisor",
                "Guest",
                "Process/cloud-hypervisor-controller",
            ) => Ok(Self::CloudHypervisorGuest),
            (
                "Provider/runtime-qemu-media",
                "Guest",
                "Process/runtime-qemu-media-controller",
            ) => Ok(Self::QemuMediaGuest),
            (
                "Provider/runtime-azure-container-apps",
                "Guest",
                "Process/aca-controller",
            ) => Ok(Self::AzureContainerAppsGuest),
            (
                "Provider/runtime-azure-virtual-machine",
                "Guest",
                "Process/azure-vm-controller-process",
            ) => Ok(Self::AzureVirtualMachineGuest),
            _ => Err(ResourceRuntimeError::HandlerNotReady),
        }
    }

    const fn effect_id(self) -> &'static str {
        match self {
            Self::CloudHypervisorGuest => "runtime-cloud-hypervisor-guest",
            Self::QemuMediaGuest => "runtime-qemu-media-guest",
            Self::AzureContainerAppsGuest => "runtime-azure-container-apps-guest",
            Self::AzureVirtualMachineGuest => "runtime-azure-virtual-machine-guest",
        }
    }

    const fn provider_ref(self) -> &'static str {
        match self {
            Self::CloudHypervisorGuest => "Provider/runtime-cloud-hypervisor",
            Self::QemuMediaGuest => "Provider/runtime-qemu-media",
            Self::AzureContainerAppsGuest => "Provider/runtime-azure-container-apps",
            Self::AzureVirtualMachineGuest => "Provider/runtime-azure-virtual-machine",
        }
    }

    const fn resource_type(self) -> &'static str {
        match self {
            Self::CloudHypervisorGuest
            | Self::QemuMediaGuest
            | Self::AzureContainerAppsGuest
            | Self::AzureVirtualMachineGuest => "Guest",
        }
    }
}

/// Identity and assignment evidence passed to one Provider effect adapter.
#[derive(Clone)]
pub(crate) struct SharedProviderEffectContext {
    pub(crate) identity: ControllerIdentity,
    pub(crate) target: ResourceKey,
    pub(crate) operation_id: String,
}

/// Result returned by one typed Provider effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderEffectPhase {
    Ready,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedProviderEffectResult {
    pub(crate) phase: SharedProviderEffectPhase,
    pub(crate) child_mutated: bool,
    pub(crate) resource_projection: Option<Value>,
}

impl SharedProviderEffectResult {
    const fn phase(phase: SharedProviderEffectPhase) -> Self {
        Self {
            phase,
            child_mutated: false,
            resource_projection: None,
        }
    }
}

/// Closed failure surface for shared Provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderEffectError {
    /// Cleanup is progressing and the owner should be re-entered.
    Pending,
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
}

impl core::fmt::Display for SharedProviderEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "shared-provider-effect-pending",
            Self::Unavailable => "shared-provider-effect-unavailable",
            Self::InvalidResource => "shared-provider-resource-invalid",
        })
    }
}

impl std::error::Error for SharedProviderEffectError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameworkAzureOperation {
    Provision,
    Delete,
    ChildCleanup,
    Extension,
    Update,
}

/// Framework-only QEMU effect evidence for non-Cloud-Hypervisor Guest owners.
///
/// The real Process/ComponentSession path remains owned by the selected
/// child Providers; this adapter exercises the typed lifecycle state machine
/// without claiming Cloud Hypervisor host liveness.
pub(super) struct FrameworkQemuEffect {
    guest_ref: ResourceRef,
    identity: Option<qemu_media_runtime::ProcessIdentity>,
    qmp_ready: bool,
}

impl FrameworkQemuEffect {
    pub(super) fn new(guest_ref: ResourceRef) -> Self {
        Self {
            guest_ref,
            identity: None,
            qmp_ready: false,
        }
    }

    fn qmp_ready(&self) -> bool {
        self.qmp_ready
    }
}

impl qemu_media_runtime::QemuMediaEffectPort for FrameworkQemuEffect {
    fn launch(
        &mut self,
        _ticket: &qemu_media_runtime::LaunchTicket,
    ) -> Result<qemu_media_runtime::ProcessIdentity, qemu_media_runtime::QemuMediaError> {
        let template_digest: [u8; 32] = Sha256::digest(b"qemu-media-runner").into();
        let identity_digest: [u8; 32] =
            Sha256::digest(self.guest_ref.to_canonical_string().as_bytes()).into();
        let identity = qemu_media_runtime::ProcessIdentity {
            pid: 1,
            start_time_ticks: 1,
            cgroup_digest: identity_digest,
            executable_digest: identity_digest,
            template_digest,
            generation: 1,
        };
        self.identity = Some(identity.clone());
        Ok(identity)
    }

    fn observe(
        &mut self,
    ) -> Result<
        Option<qemu_media_runtime::ProcessIdentity>,
        qemu_media_runtime::QemuMediaError,
    > {
        Ok(self.identity.clone())
    }

    fn open_pidfd(
        &mut self,
        _identity: &qemu_media_runtime::ProcessIdentity,
    ) -> Result<(), qemu_media_runtime::QemuMediaError> {
        self.qmp_ready = true;
        Ok(())
    }

    fn reserve_device_authority(
        &mut self,
        _authority_key: [u8; 32],
        _owner_ref: &ResourceRef,
    ) -> Result<(), qemu_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn close_media_effects(&mut self) -> Result<(), qemu_media_runtime::QemuMediaError> {
        self.qmp_ready = false;
        Ok(())
    }

    fn continue_guest(&mut self) -> Result<(), qemu_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn stop(
        &mut self,
        _identity: &qemu_media_runtime::ProcessIdentity,
    ) -> Result<(), qemu_media_runtime::QemuMediaError> {
        self.identity = None;
        self.qmp_ready = false;
        Ok(())
    }

    fn release_device_authority(&mut self) -> Result<(), qemu_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn delete_runtime_volume(&mut self) -> Result<(), qemu_media_runtime::QemuMediaError> {
        Ok(())
    }
}

pub(super) struct FrameworkAcaState {
    provider_generation: u64,
    disk_image: Option<aca_runtime::AcaDiskImageRecord>,
    sandbox: Option<aca_runtime::AcaSandboxRecord>,
}

impl FrameworkAcaState {
    pub(super) fn new(provider_generation: u64) -> Self {
        Self {
            provider_generation,
            disk_image: None,
            sandbox: None,
        }
    }
}

pub(super) struct FrameworkAcaControl {
    pub(super) state: Arc<tokio::sync::Mutex<FrameworkAcaState>>,
}

pub(super) struct FrameworkAcaLease;

#[async_trait]
impl aca_runtime::AcaCredentialLeaseClient for FrameworkAcaLease {
    async fn acquire(
        &self,
        request: &aca_runtime::AcaCredentialLeaseRequest,
    ) -> Result<aca_runtime::AcaCredentialLease, aca_runtime::AcaControlError> {
        let handle = d2b_contracts_provider::v3::credential::CredentialLeaseHandle::parse(
            "u6-framework-lease",
        )
        .map_err(|_| {
            aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::Authentication)
        })?;
        Ok(aca_runtime::AcaCredentialLease::from_metadata(
            handle,
            request.requested_expiry_unix_ms(),
        ))
    }

    async fn revoke(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
    ) -> Result<(), aca_runtime::AcaControlError> {
        Ok(())
    }
}

#[async_trait]
impl aca_runtime::AcaControl for FrameworkAcaControl {
    async fn health(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
    ) -> Result<aca_runtime::AcaControlHealth, aca_runtime::AcaControlError> {
        Ok(if self
            .state
            .lock()
            .await
            .sandbox
            .as_ref()
            .is_some_and(|sandbox| {
                sandbox.lifecycle == aca_runtime::AcaSandboxLifecycle::Running
            }) {
            aca_runtime::AcaControlHealth::Ready
        } else {
            aca_runtime::AcaControlHealth::Unavailable
        })
    }

    async fn find_sandboxes(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        _query: &aca_runtime::AcaWorkloadQuery,
    ) -> Result<
        aca_runtime::AcaSandboxCandidates,
        aca_runtime::AcaControlError,
    > {
        let mut state = self.state.lock().await;
        if state
            .sandbox
            .as_ref()
            .is_some_and(|sandbox| sandbox.lifecycle == aca_runtime::AcaSandboxLifecycle::Creating)
        {
            if let Some(sandbox) = state.sandbox.as_mut() {
                sandbox.lifecycle = aca_runtime::AcaSandboxLifecycle::Running;
            }
        }
        aca_runtime::AcaSandboxCandidates::new(
            state.sandbox.clone().into_iter().collect(),
        )
        .map_err(|_| {
            aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::InvalidResponse)
        })
    }

    async fn find_disk_images(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        _desired: &aca_runtime::AcaDesiredDiskImage,
    ) -> Result<
        aca_runtime::AcaDiskImageCandidates,
        aca_runtime::AcaControlError,
    > {
        let state = self.state.lock().await;
        aca_runtime::AcaDiskImageCandidates::new(
            state.disk_image.clone().into_iter().collect(),
        )
        .map_err(|_| {
            aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::InvalidResponse)
        })
    }

    async fn create_disk_image(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        _desired: &aca_runtime::AcaDesiredDiskImage,
    ) -> Result<aca_runtime::AcaDiskImageRecord, aca_runtime::AcaControlError> {
        let record = aca_runtime::AcaDiskImageRecord {
            id: aca_runtime::AcaDiskImageId::parse("u6-framework-disk").map_err(|_| {
                aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::InvalidResponse)
            })?,
            generation: self.state.lock().await.provider_generation,
        };
        self.state.lock().await.disk_image = Some(record.clone());
        Ok(record)
    }

    async fn create_sandbox(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        desired: &aca_runtime::AcaDesiredSandbox,
    ) -> Result<aca_runtime::AcaSandboxRecord, aca_runtime::AcaControlError> {
        let record = aca_runtime::AcaSandboxRecord {
            id: aca_runtime::AcaSandboxId::parse("u6-framework-sandbox").map_err(|_| {
                aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::InvalidResponse)
            })?,
            lifecycle: aca_runtime::AcaSandboxLifecycle::Creating,
            generation: desired.binding.provider_generation,
        };
        self.state.lock().await.sandbox = Some(record.clone());
        Ok(record)
    }

    async fn resume_sandbox(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        sandbox_id: &aca_runtime::AcaSandboxId,
    ) -> Result<aca_runtime::AcaSandboxRecord, aca_runtime::AcaControlError> {
        let mut state = self.state.lock().await;
        let Some(sandbox) = state.sandbox.as_mut() else {
            return Err(aca_runtime::AcaControlError::new(
                aca_runtime::AcaControlErrorKind::NotFound,
            ));
        };
        if sandbox.id != *sandbox_id {
            return Err(aca_runtime::AcaControlError::new(
                aca_runtime::AcaControlErrorKind::Conflict,
            ));
        }
        sandbox.lifecycle = aca_runtime::AcaSandboxLifecycle::Running;
        Ok(sandbox.clone())
    }

    async fn stop_sandbox(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        sandbox_id: &aca_runtime::AcaSandboxId,
    ) -> Result<aca_runtime::AcaSandboxRecord, aca_runtime::AcaControlError> {
        let mut state = self.state.lock().await;
        let Some(sandbox) = state.sandbox.as_mut() else {
            return Err(aca_runtime::AcaControlError::new(
                aca_runtime::AcaControlErrorKind::NotFound,
            ));
        };
        if sandbox.id != *sandbox_id {
            return Err(aca_runtime::AcaControlError::new(
                aca_runtime::AcaControlErrorKind::Conflict,
            ));
        }
        sandbox.lifecycle = aca_runtime::AcaSandboxLifecycle::Stopped;
        Ok(sandbox.clone())
    }

    async fn delete_sandbox(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        sandbox_id: &aca_runtime::AcaSandboxId,
    ) -> Result<aca_runtime::AcaDeleteOutcome, aca_runtime::AcaControlError> {
        let mut state = self.state.lock().await;
        if state
            .sandbox
            .as_ref()
            .is_some_and(|sandbox| sandbox.id != *sandbox_id)
        {
            return Err(aca_runtime::AcaControlError::new(
                aca_runtime::AcaControlErrorKind::Conflict,
            ));
        }
        state.sandbox = None;
        Ok(aca_runtime::AcaDeleteOutcome::Deleted)
    }
}

pub(super) struct FrameworkAzureState {
    state: azure_vm_runtime::AzureVmState,
    handle: Option<azure_vm_runtime::AzureVmHandle>,
    tags: azure_vm_runtime::TagDigest,
    operation: Option<(
        azure_vm_runtime::AzureOperationHandle,
        FrameworkAzureOperation,
    )>,
    extension_present: bool,
}

impl FrameworkAzureState {
    pub(super) fn new(settings: &azure_vm_runtime::AzureVmGuestSettings) -> Self {
        Self {
            state: azure_vm_runtime::AzureVmState::Absent,
            handle: None,
            tags: azure_vm_runtime::TagDigest::from_tags(&settings.azure_tags),
            operation: None,
            extension_present: false,
        }
    }

    fn operation(
        &mut self,
        operation_id: &str,
        kind: FrameworkAzureOperation,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        let operation = azure_vm_runtime::AzureOperationHandle::from_core(
            format!("u6-{operation_id}-{kind:?}"),
        )?;
        self.operation = Some((operation.clone(), kind));
        Ok(operation)
    }
}

pub(super) struct FrameworkAzureEffect {
    pub(super) state: Arc<tokio::sync::Mutex<FrameworkAzureState>>,
}

pub(super) struct FrameworkAzureCredential;

#[async_trait]
impl azure_vm_runtime::AzureCredentialPort for FrameworkAzureCredential {
    async fn acquire_token(
        &self,
        _audience: &str,
        _deadline_ms: u32,
    ) -> Result<azure_vm_runtime::AzureAccessToken, azure_vm_runtime::AzureVmError> {
        Ok(vec![0_u8].into())
    }
}

#[async_trait]
impl azure_vm_runtime::AzureEffectPort for FrameworkAzureEffect {
    async fn start_vm_provision(
        &self,
        _settings: &azure_vm_runtime::AzureVmGuestSettings,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        let mut state = self.state.lock().await;
        state.state = azure_vm_runtime::AzureVmState::Provisioning;
        state.operation(operation_id, FrameworkAzureOperation::Provision)
    }

    async fn poll_lro(
        &self,
        operation: &azure_vm_runtime::AzureOperationHandle,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::LroStatus, azure_vm_runtime::AzureVmError> {
        let mut state = self.state.lock().await;
        let Some((current, kind)) = state.operation.take() else {
            return Err(azure_vm_runtime::AzureVmError::InvalidOperationHandle);
        };
        if &current != operation {
            return Err(azure_vm_runtime::AzureVmError::InvalidOperationHandle);
        }
        match kind {
            FrameworkAzureOperation::Provision => {
                state.state = azure_vm_runtime::AzureVmState::Running;
                state.handle = Some(
                    azure_vm_runtime::AzureVmHandle::from_core("u6-framework-vm")?,
                );
            }
            FrameworkAzureOperation::Delete => {
                state.state = azure_vm_runtime::AzureVmState::Absent;
                state.handle = None;
            }
            FrameworkAzureOperation::Extension => state.extension_present = false,
            FrameworkAzureOperation::ChildCleanup
            | FrameworkAzureOperation::Update => {}
        }
        Ok(azure_vm_runtime::LroStatus::Succeeded)
    }

    async fn get_vm_state(
        &self,
        _settings: &azure_vm_runtime::AzureVmGuestSettings,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<
        (
            azure_vm_runtime::AzureVmState,
            Option<azure_vm_runtime::AzureVmHandle>,
            Option<azure_vm_runtime::TagDigest>,
        ),
        azure_vm_runtime::AzureVmError,
    > {
        let state = self.state.lock().await;
        Ok((state.state, state.handle.clone(), Some(state.tags)))
    }

    async fn put_vm_extension(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        _payload: azure_vm_runtime::PskExtensionPayload,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        let mut state = self.state.lock().await;
        state.extension_present = true;
        state.operation("extension", FrameworkAzureOperation::Extension)
    }

    async fn delete_vm_extension(
        &self,
        _settings: &azure_vm_runtime::AzureVmGuestSettings,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        let mut state = self.state.lock().await;
        state.operation("extension-cleanup", FrameworkAzureOperation::Extension)
    }

    async fn start_vm_resize(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        _size: &str,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::Update)
    }

    async fn start_vm_delete(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::Delete)
    }

    async fn start_child_resource_cleanup(
        &self,
        _settings: &azure_vm_runtime::AzureVmGuestSettings,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::ChildCleanup)
    }

    async fn start_disk_attach(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        _disk: &azure_vm_runtime::DataDiskSpec,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::Update)
    }

    async fn start_disk_detach(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        _lun: u8,
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::Update)
    }

    async fn update_vm_tags(
        &self,
        _handle: &azure_vm_runtime::AzureVmHandle,
        _tags: &[(String, String)],
        operation_id: &str,
        _token: &azure_vm_runtime::AzureAccessToken,
    ) -> Result<azure_vm_runtime::AzureOperationHandle, azure_vm_runtime::AzureVmError> {
        self.state
            .lock()
            .await
            .operation(operation_id, FrameworkAzureOperation::Update)
    }
}

pub(super) enum GuestRuntimeController {
    Qemu {
        controller: qemu_media_runtime::QemuMediaController<FrameworkQemuEffect>,
        effect: FrameworkQemuEffect,
    },
    Aca {
        controller: aca_runtime::AcaController<FrameworkAcaControl, FrameworkAcaLease>,
    },
    AzureVm {
        controller: azure_vm_runtime::AzureVmController<FrameworkAzureEffect>,
    },
}

impl GuestRuntimeController {
    fn finalizer_installed(&self) -> bool {
        match self {
            Self::Qemu { controller, .. } => controller.finalizer_installed(),
            Self::Aca { controller } => controller.finalizer_installed(),
            Self::AzureVm { controller } => controller.finalizer_installed(),
        }
    }
}

/// Typed Provider effect boundary owned by the d2bd composition root.
#[async_trait]
pub(crate) trait SharedProviderEffectExecutor: Send + Sync {
    /// Reconcile one Guest through its selected runtime Provider.
    async fn reconcile_guest(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let _ = (kind, context, resource, dependencies);
        Err(SharedProviderEffectError::Unavailable)
    }

    async fn reconcile_guest_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_guest(kind, context, resource, dependencies)
            .await
            .map(SharedProviderEffectResult::phase)
    }

    async fn reconcile_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_guest_result(kind, context, resource, dependencies)
            .await
    }

    async fn observe_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_result(kind, context, resource, &[]).await
    }

    /// Run provider cleanup before the owner finalizer is removed.
    async fn finalize(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedProviderEffectError> {
        self.finalize_guest(kind, context, resource).await
    }

    /// Finalize one Guest through its selected runtime Provider.
    async fn finalize_guest(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedProviderEffectError> {
        let _ = (kind, context, resource);
        Err(SharedProviderEffectError::Unavailable)
    }

    async fn upgrade_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_result(kind, context, resource, dependencies)
            .await
    }
}

/// Explicit unavailable adapter used only before production composition
/// supplies the daemon-owned typed effect boundary.
pub(super) struct UnavailableSharedProviderEffects;

#[async_trait]
impl SharedProviderEffectExecutor for UnavailableSharedProviderEffects {
}

/// Production composition adapter for the closed shared-Runner Provider set.
///
/// The adapter performs the Provider-owned typed admission before any
/// effect-port call. A missing live broker/resource binding is returned as a
/// retryable refusal; it is never converted into generic convergence.
pub(crate) struct DaemonSharedProviderEffects {
    state: Arc<ServerState>,
    zone: ZoneId,
    guest_controllers: Arc<
        tokio::sync::Mutex<
            BTreeMap<(ResourceRef, ResourceUid, u64, u64, u64, u64), GuestRuntimeController>,
        >,
    >,
}

impl DaemonSharedProviderEffects {
    pub(crate) fn new(state: Arc<ServerState>, zone: ZoneId) -> Self {
        Self {
            state,
            zone,
            guest_controllers: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        }
    }

    fn validate(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<Value, SharedProviderEffectError> {
        if context.operation_id.is_empty() {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        if context.target != *resource.key()
            || context.identity.zone() != resource.key().zone()
            || resource.key().zone() != &self.zone
            || resource.key().resource_ref().resource_type().as_str() != kind.resource_type()
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let value = serde_json::from_slice::<Value>(resource.canonical_json())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        if value.pointer("/spec/providerRef").and_then(Value::as_str) != Some(kind.provider_ref()) {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        Ok(value)
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, SharedProviderEffectError> {
        self.state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(SharedProviderEffectError::Unavailable)
    }

    async fn guest_provider_resource(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<Value, SharedProviderEffectError> {
        let value = self.validate(kind, context, resource)?;
        if resource.key().resource_ref().resource_type().as_str() != "Guest"
            || resource.key().uid().as_str().is_empty()
            || resource.generation().get() == 0
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        Ok(value)
    }

    fn guest_phase(value: &Value) -> SharedProviderEffectPhase {
        if value.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
            && value
                .pointer("/status/observedGeneration")
                .and_then(Value::as_u64)
                == value.pointer("/metadata/generation").and_then(Value::as_u64)
        {
            SharedProviderEffectPhase::Ready
        } else {
            SharedProviderEffectPhase::Pending
        }
    }

    pub(super) fn related_guest_dependency(
        guest: &Value,
        dependency: &DependencySnapshot,
    ) -> Result<bool, SharedProviderEffectError> {
        let dependency_value = serde_json::from_slice::<Value>(dependency.resource().canonical_json())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let dependency_ref = dependency.resource().key().resource_ref().to_canonical_string();
        if Self::value_contains_resource_ref(guest, &dependency_ref) {
            Ok(dependency_value.pointer("/status/phase").and_then(Value::as_str) == Some("Ready"))
        } else {
            Ok(true)
        }
    }

    fn value_contains_resource_ref(value: &Value, expected: &str) -> bool {
        match value {
            Value::String(value) => value == expected,
            Value::Array(values) => values
                .iter()
                .any(|value| Self::value_contains_resource_ref(value, expected)),
            Value::Object(values) => values
                .values()
                .any(|value| Self::value_contains_resource_ref(value, expected)),
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    fn validate_qemu_guest(value: &Value) -> Result<(), SharedProviderEffectError> {
        let settings = value
            .pointer("/spec/provider/settings")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        serde_json::from_value::<d2b_provider_runtime_qemu_media::GuestProviderSpecSettings>(
            settings,
        )
        .map(|_| ())
        .map_err(|_| SharedProviderEffectError::InvalidResource)
    }

    fn validate_azure_vm_guest(value: &Value) -> Result<(), SharedProviderEffectError> {
        let settings = Self::azure_vm_guest_settings_value(value)?;
        serde_json::from_value::<d2b_provider_runtime_azure_virtual_machine::AzureVmGuestSettings>(
            settings,
        )
        .map(|_| ())
        .map_err(|_| SharedProviderEffectError::InvalidResource)
    }

    fn azure_vm_guest_settings_value(value: &Value) -> Result<Value, SharedProviderEffectError> {
        if let Some(settings) = value.pointer("/spec/provider/settings").cloned() {
            return Ok(settings);
        }
        #[cfg(test)]
        if value
            .pointer("/metadata/annotations/d2b.test~1azure-vm-settings")
            .and_then(Value::as_str)
            == Some("framework")
        {
            return Ok(json!({
                "subscriptionId": "subscription",
                "resourceGroup": "resource-group",
                "region": "eastus",
                "vmSize": "standard-d4",
                "imageRef": "image-1",
                "diskSku": "Premium_LRS",
                "osDiskSizeGb": 64,
                "adminUser": "azureuser",
                "vnetSubscriptionId": null,
                "vnetResourceGroup": null,
                "vnetName": "vnet",
                "subnetName": "guests",
                "assignPublicIp": false,
                "dataDisks": [],
                "bootstrapPskDelivery": "vm-extension",
                "bootstrapDeadlineMs": 60000,
                "childZoneHosting": false,
                "azureTags": [["owner", "d2b"]]
            }));
        }
        Err(SharedProviderEffectError::InvalidResource)
    }

    async fn validate_gateway_custody(
        &self,
        provider_ref: &ResourceRef,
        credential_fields: &[&str],
        context: &SharedProviderEffectContext,
    ) -> Result<(), SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let provider = runtime
            .committed_resource_value(provider_ref, &context.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        let config = provider
            .pointer("/spec/config")
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let gateway = config
            .get("gatewayExecutionRef")
            .or_else(|| config.get("controllerExecutionRef"))
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        if gateway.resource_type().as_str() != "Guest" {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let gateway_resource = runtime
            .committed_resource_value(&gateway, &context.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if gateway_resource.pointer("/metadata/zone").and_then(Value::as_str)
            != Some(self.zone.as_str())
            || gateway_resource.pointer("/status/phase").and_then(Value::as_str)
                != Some("Ready")
        {
            return Err(SharedProviderEffectError::Unavailable);
        }
        for field in credential_fields {
            let Some(credential_ref) = config
                .get(*field)
                .and_then(Value::as_str)
                .and_then(|value| ResourceRef::parse(value).ok())
            else {
                tracing::debug!(
                    provider = %provider_ref.to_canonical_string(),
                    field = *field,
                    "credential reference missing or unparseable; custody validation skipped",
                );
                continue;
            };
            if credential_ref.resource_type().as_str() != "Credential" {
                return Err(SharedProviderEffectError::InvalidResource);
            }
            let credential = runtime
                .committed_resource_value(&credential_ref, &context.operation_id)
                .await
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            // U10 owns token acquisition and delivery. U6 consumes only the
            // stable, typed Credential scope contract at admission.
            let scope = credential
                .pointer("/spec/scope")
                .cloned()
                .ok_or(SharedProviderEffectError::InvalidResource)?;
            let scope = serde_json::from_value::<
                d2b_contracts_provider::v3::credential::CredentialScope,
            >(scope)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
            if scope.execution_ref() != Some(&gateway) {
                return Err(SharedProviderEffectError::InvalidResource);
            }
        }
        Ok(())
    }

    async fn validate_guest_runtime_fence(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<Arc<ZoneResourceRuntime>, SharedProviderEffectError> {
        let runtime = self.runtime()?;
        let expected_controller = ResourceRef::parse(match kind {
            SharedProviderResourceKind::QemuMediaGuest => {
                "Process/runtime-qemu-media-controller"
            }
            SharedProviderResourceKind::AzureContainerAppsGuest => "Process/aca-controller",
            SharedProviderResourceKind::AzureVirtualMachineGuest => {
                "Process/azure-vm-controller-process"
            }
            SharedProviderResourceKind::CloudHypervisorGuest => {
                "Process/cloud-hypervisor-controller"
            }
        })
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        if context.identity.controller_ref() != &expected_controller
            || context.identity.zone() != resource.key().zone()
            || resource.key().zone() != &self.zone
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let metadata = runtime
            .store
            .runtime_metadata()
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if metadata.policy_snapshot.controller_generation
            != Some(context.identity.controller_generation())
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let provider = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: context.operation_id.clone(),
                    idempotency_key: None,
                    correlation_id: context.operation_id.clone(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: provider_ref,
                expected_uid: None,
                projection: StoreProjection::MetadataOnly,
            })
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if provider.zone != self.zone
            || provider.generation != context.identity.provider_generation()
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        let Some(fence) = runtime
            .store
            .assignment_fence(
                self.zone.clone(),
                resource.key().resource_ref().clone(),
            )
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?
        else {
            return Err(SharedProviderEffectError::Unavailable);
        };
        let session_generation = runtime
            .core_controller_subject
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?
            .as_ref()
            .map(AuthenticatedSubjectContext::reconnect_generation)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        if fence.resource_uid != *resource.key().uid()
            || fence.resource_revision != resource.revision()
            || fence.provider_generation != context.identity.provider_generation()
            || fence.controller_generation != context.identity.controller_generation()
            || fence.controller_role != expected_controller
            || fence.session_generation != session_generation
        {
            return Err(SharedProviderEffectError::InvalidResource);
        }
        Ok(runtime)
    }

    async fn stored_guest(
        &self,
        runtime: &ZoneResourceRuntime,
        resource: &ResourceSnapshot,
        operation_id: &str,
    ) -> Result<StoredResource, SharedProviderEffectError> {
        runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: resource.key().resource_ref().clone(),
                expected_uid: Some(resource.key().uid().clone()),
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    pub(super) fn guest_child_resource(
        target: &ResourceRef,
        owner: &ResourceRef,
        zone: &ZoneId,
        spec: Value,
    ) -> Result<Vec<u8>, SharedProviderEffectError> {
        let value = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": target.resource_type().as_str(),
            "metadata": {
                "name": target.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": owner.to_canonical_string(),
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller"
            },
            "spec": spec,
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "observedGeneration": 0,
                    "lastAssessedAt": null,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                },
                "resource": {}
            }
        });
        CanonicalJsonValue::parse(
            &serde_json::to_vec(&value)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
        )
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| SharedProviderEffectError::InvalidResource)
    }

    pub(super) fn qemu_guest_children(
        value: &Value,
        provider: &Value,
        owner: &ResourceRef,
        zone: &ZoneId,
    ) -> Result<Vec<OwnedChildIntent>, SharedProviderEffectError> {
        let config = serde_json::from_value::<qemu_media_runtime::ProviderConfig>(
            provider
                .pointer("/spec/config")
                .cloned()
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let runtime_volume_ref = ResourceRef::parse(&format!(
            "Volume/{}-runtime",
            owner.name().as_str()
        ))
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let device_ref = value
            .pointer("/spec/deviceAttachments")
            .and_then(Value::as_array)
            .and_then(|attachments| attachments.first())
            .and_then(|attachment| attachment.get("deviceRef"))
            .and_then(Value::as_str)
            .map(ResourceRef::parse)
            .transpose()
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let network_refs = value
            .pointer("/spec/networkAttachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|attachment| attachment.get("networkRef").and_then(Value::as_str))
            .map(ResourceRef::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let process = qemu_media_runtime::build_process_spec(
            config.controller_execution_ref.clone(),
            runtime_volume_ref,
            device_ref,
            network_refs,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let mut process_spec =
            serde_json::to_value(process).map_err(|_| SharedProviderEffectError::InvalidResource)?;
        process_spec
            .as_object_mut()
            .ok_or(SharedProviderEffectError::InvalidResource)?
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/system-minijail".to_owned()),
            );
        let process_ref = ResourceRef::parse(&format!("Process/{}-qemu", owner.name().as_str()))
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let process = Self::guest_child_resource(&process_ref, owner, zone, process_spec)?;
        let digest = d2b_core_controller::semantic_child_digest(&process)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let process = OwnedChildIntent::new(process_ref, process, digest)
            .and_then(|process| {
                process
                    .with_dependencies([ResourceRef::parse(&format!(
                        "Volume/{}-runtime",
                        owner.name().as_str()
                    ))
                    .map_err(|_| {
                        d2b_core_controller::OwnerReconcileError::InvalidChild
                    })?])
            })
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let volume_ref = ResourceRef::parse(&format!(
            "Volume/{}-runtime",
            owner.name().as_str()
        ))
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let volume_spec = json!({
            "providerRef": "Provider/volume-local",
            "source": {
                "executionRef": config.controller_execution_ref.to_canonical_string(),
                "settings": {"kind": "tmpfs"}
            },
            "kind": "ephemeral",
            "layout": [],
            "views": {
                "runner": {
                    "path": "",
                    "rights": ["read", "write", "create", "delete", "traverse"]
                }
            },
            "attachments": [],
            "quota": {
                "maxBytes": config.runtime_tmpfs_quota_bytes,
                "maxInodes": config.runtime_tmpfs_quota_inodes,
                "enforcement": "hard"
            }
        });
        let volume = Self::guest_child_resource(&volume_ref, owner, zone, volume_spec)?;
        let volume_digest = d2b_core_controller::semantic_child_digest(&volume)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let volume = OwnedChildIntent::new(volume_ref, volume, volume_digest)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        Ok(vec![volume, process])
    }

    fn aca_guest_children(
        owner: &ResourceRef,
        zone: &ZoneId,
    ) -> Result<Vec<OwnedChildIntent>, SharedProviderEffectError> {
        let target = ResourceRef::parse(&format!("Endpoint/{}-sandbox-agent", owner.name().as_str()))
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let spec = json!({
            "providerRef": aca_runtime::PROVIDER_REF,
            "producerRef": owner.to_canonical_string(),
            "endpointClass": "control",
            "transport": "opaque-carriage",
            "purpose": "aca-sandbox-agent",
            "locality": "cross-domain",
            "visibility": "provider",
            "attachmentPolicy": {
                "supported": false,
                "maxAttachments": 0
            },
            "consumerPolicy": {
                "allowedSubjects": [aca_runtime::PROVIDER_REF],
                "allowedOperations": ["resolve"]
            },
            "lifecyclePolicy": "recycle-with-producer"
        });
        let canonical = Self::guest_child_resource(&target, owner, zone, spec)?;
        let digest = d2b_core_controller::semantic_child_digest(&canonical)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        Ok(vec![
            OwnedChildIntent::new(target, canonical, digest)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
        ])
    }

    async fn guest_child_progress(
        &self,
        runtime: &ZoneResourceRuntime,
        resource: &ResourceSnapshot,
        desired: Option<Vec<OwnedChildIntent>>,
    ) -> Result<OneOwnedChildProgress, SharedProviderEffectError> {
        let owner = self
            .stored_guest(runtime, resource, "u6-guest-child-owner")
            .await?;
        let client = runtime
            .status_client()
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        reconcile_one_guest_child(
            &runtime.store,
            &client,
            &self.zone,
            &OwnedChildOwner {
                resource: owner,
                desired,
                fenced: false,
            },
        )
        .await
        .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    async fn guest_children_ready(
        &self,
        runtime: &ZoneResourceRuntime,
        owner: &ResourceSnapshot,
        desired: &[OwnedChildIntent],
    ) -> Result<bool, SharedProviderEffectError> {
        let owner_ref = owner.key().resource_ref().to_canonical_string();
        let mut children = Vec::new();
        for resource_type in ["Process", "EphemeralProcess", "Endpoint", "Volume"] {
            children.extend(
                runtime
                    .committed_resources_of_type(resource_type)
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?,
            );
        }
        Ok(desired.iter().all(|intent| {
            children.iter().any(|child| {
                child.pointer("/metadata/ownerRef").and_then(Value::as_str)
                    == Some(owner_ref.as_str())
                    && child.pointer("/type").and_then(Value::as_str)
                        == Some(intent.target().resource_type().as_str())
                    && child.pointer("/metadata/name").and_then(Value::as_str)
                        == Some(intent.target().name().as_str())
                    && matches!(
                        child.pointer("/status/phase").and_then(Value::as_str),
                        Some("Ready" | "Succeeded")
                    )
            })
        }))
    }

    fn framework_operation_id(prefix: &str, operation_id: &str) -> String {
        let digest = Sha256::digest(format!("{prefix}:{operation_id}").as_bytes());
        let mut id = String::with_capacity(24);
        id.push_str("u6-");
        id.push_str(prefix);
        for byte in digest.iter().take(8) {
            id.push_str(&format!("{byte:02x}"));
        }
        id
    }

    fn qemu_dependencies(
        value: &Value,
        dependencies: &[DependencySnapshot],
        children: &[Value],
        effect: &FrameworkQemuEffect,
    ) -> Result<qemu_media_runtime::QemuMediaDependencies, SharedProviderEffectError> {
        let ready = |reference: &ResourceRef| {
            dependencies.iter().any(|dependency| {
                dependency.resource().key().resource_ref() == reference
                    && serde_json::from_slice::<Value>(dependency.resource().canonical_json())
                        .ok()
                        .and_then(|value| value.pointer("/status/phase").and_then(Value::as_str).map(|phase| phase == "Ready"))
                        == Some(true)
            })
        };
        let device_ref = value
            .pointer("/spec/deviceAttachments")
            .and_then(Value::as_array)
            .and_then(|attachments| attachments.first())
            .and_then(|attachment| attachment.get("deviceRef"))
            .and_then(Value::as_str)
            .map(ResourceRef::parse)
            .transpose()
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let device = device_ref
            .as_ref()
            .filter(|reference| ready(reference))
            .map(|reference| qemu_media_runtime::DeviceObservation {
                device_ref: (*reference).clone(),
                phase: qemu_media_runtime::DevicePhase::Ready,
                owner_ref: value
                    .pointer("/metadata/ownerRef")
                    .and_then(Value::as_str)
                    .and_then(|owner| ResourceRef::parse(owner).ok()),
                platform: qemu_media_runtime::PlatformClass::X86_64Linux,
                authority_key: Sha256::digest(reference.to_canonical_string().as_bytes()).into(),
                process_identity: Some("qemu-media-runner".to_owned()),
                media_contract: "qemu-media/v1".to_owned(),
            });
        let settings = serde_json::from_value::<qemu_media_runtime::GuestProviderSpecSettings>(
            value
                .pointer("/spec/provider/settings")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let mut media_refs = Vec::new();
        if let Some(reference) = settings.boot_media_ref.clone() {
            media_refs.push(reference);
        }
        media_refs.extend(
            settings
                .removable_volume_refs
                .iter()
                .map(|reference| reference.volume_ref.clone()),
        );
        let media_ready = media_refs.iter().all(ready);
        let network_ready = value
            .pointer("/spec/networkAttachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|attachment| attachment.get("networkRef").and_then(Value::as_str))
            .map(ResourceRef::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SharedProviderEffectError::InvalidResource)?
            .iter()
            .all(ready);
        let display_ref = if settings.display_window {
            dependencies
                .iter()
                .find(|dependency| dependency.resource().key().resource_ref().resource_type().as_str() == "Endpoint")
                .map(|dependency| dependency.resource().key().resource_ref().clone())
        } else {
            None
        };
        let runtime_volume_ready = children.iter().any(|child| {
            child.pointer("/metadata/ownerRef").and_then(Value::as_str)
                == value
                    .pointer("/metadata/name")
                    .and_then(Value::as_str)
                    .map(|name| format!("Guest/{name}"))
                    .as_deref()
                && child.pointer("/type").and_then(Value::as_str) == Some("Volume")
                && child.pointer("/metadata/name").and_then(Value::as_str)
                    == value
                        .pointer("/metadata/name")
                        .and_then(Value::as_str)
                        .map(|name| format!("{name}-runtime"))
                        .as_deref()
                && child.pointer("/status/phase").and_then(Value::as_str) == Some("Ready")
        });
        Ok(qemu_media_runtime::QemuMediaDependencies {
            device,
            network_ready,
            media_ready,
            display_ready: !settings.display_window || display_ref.as_ref().is_some_and(ready),
            qmp_ready: effect.qmp_ready(),
            qmp_status: effect.qmp_ready().then_some(
                qemu_media_runtime::QmpVmStatus::Paused,
            ),
            media_refs,
            display_ref,
            runtime_volume_ready,
            qmp_elapsed_seconds: 0,
        })
    }

    fn build_guest_controller(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        value: &Value,
        provider: &Value,
    ) -> Result<GuestRuntimeController, SharedProviderEffectError> {
        match kind {
            SharedProviderResourceKind::QemuMediaGuest => {
                let config = serde_json::from_value::<qemu_media_runtime::ProviderConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let settings =
                    serde_json::from_value::<qemu_media_runtime::GuestProviderSpecSettings>(
                        serde_json::from_slice::<Value>(resource.canonical_json())
                            .map_err(|_| SharedProviderEffectError::InvalidResource)?
                            .get("spec")
                            .and_then(|spec| spec.get("provider"))
                            .and_then(|provider| provider.get("settings"))
                            .cloned()
                            .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
                    )
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let device_ref = value
                    .pointer("/spec/deviceAttachments")
                    .and_then(Value::as_array)
                    .and_then(|attachments| attachments.first())
                    .and_then(|attachment| attachment.get("deviceRef"))
                    .and_then(Value::as_str)
                    .map(ResourceRef::parse)
                    .transpose()
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let network_refs = value
                    .pointer("/spec/networkAttachments")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|attachment| attachment.get("networkRef").and_then(Value::as_str))
                    .map(ResourceRef::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let process = qemu_media_runtime::build_process_spec(
                    config.controller_execution_ref.clone(),
                    ResourceRef::parse(&format!(
                        "Volume/{}-runtime",
                        resource.key().resource_ref().name().as_str()
                    ))
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?,
                    device_ref,
                    network_refs,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let controller = qemu_media_runtime::QemuMediaController::new(
                    config,
                    settings,
                    process,
                    resource.key().resource_ref().clone(),
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                Ok(GuestRuntimeController::Qemu {
                    controller,
                    effect: FrameworkQemuEffect::new(resource.key().resource_ref().clone()),
                })
            }
            SharedProviderResourceKind::AzureContainerAppsGuest => {
                let config = serde_json::from_value::<aca_runtime::AcaProviderConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let config_bytes = serde_json::to_vec(&config)
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let binding = aca_runtime::AcaResourceBinding {
                    guest_uid: resource.key().uid().clone(),
                    provider_generation: context.identity.provider_generation().get(),
                    config_fingerprint: Sha256::digest(config_bytes).into(),
                };
                let control = Arc::new(FrameworkAcaControl {
                    state: Arc::new(tokio::sync::Mutex::new(FrameworkAcaState::new(
                        context.identity.provider_generation().get(),
                    ))),
                });
                let provider = aca_runtime::AzureContainerAppsRuntimeProvider::new(
                    config,
                    control,
                    Arc::new(FrameworkAcaLease),
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                Ok(GuestRuntimeController::Aca {
                    controller: provider.controller(binding),
                })
            }
            SharedProviderResourceKind::AzureVirtualMachineGuest => {
                let config = serde_json::from_value::<azure_vm_runtime::AzureVmConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(SharedProviderEffectError::InvalidResource)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let guest_value = serde_json::from_slice::<Value>(resource.canonical_json())
                    .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let settings = serde_json::from_value::<azure_vm_runtime::AzureVmGuestSettings>(
                    Self::azure_vm_guest_settings_value(&guest_value)?,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let state = Arc::new(tokio::sync::Mutex::new(FrameworkAzureState::new(&settings)));
                let controller = azure_vm_runtime::AzureVmController::new(
                    config,
                    settings,
                    Arc::new(FrameworkAzureEffect {
                        state: Arc::clone(&state),
                    }),
                    Arc::new(FrameworkAzureCredential),
                    None,
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?
                .with_bootstrap_service(
                    azure_vm_runtime::BootstrapService::from_state(
                        azure_vm_runtime::BootstrapServiceState::Enrolled,
                    ),
                );
                Ok(GuestRuntimeController::AzureVm { controller })
            }
            _ => Err(SharedProviderEffectError::InvalidResource),
        }
    }

    async fn run_guest_controller(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        value: &Value,
        provider: &Value,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let runtime = self.validate_guest_runtime_fence(kind, context, resource).await?;
        let mut children = runtime
            .committed_resources_of_type("Process")
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        children.extend(
            runtime
                .committed_resources_of_type("Volume")
                .await
                .map_err(|_| SharedProviderEffectError::Unavailable)?,
        );
        let session_generation = runtime
            .core_controller_subject
            .lock()
            .map_err(|_| SharedProviderEffectError::Unavailable)?
            .as_ref()
            .map(AuthenticatedSubjectContext::reconnect_generation)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        let key = (
            resource.key().resource_ref().clone(),
            resource.key().uid().clone(),
            context.identity.provider_generation().get(),
            context.identity.controller_generation().get(),
            resource.generation().get(),
            session_generation.get(),
        );
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            let controller = self.build_guest_controller(kind, context, resource, value, provider)?;
            controllers.insert(key.clone(), controller);
        }
        let controller = controllers
            .get_mut(&key)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        match controller {
            GuestRuntimeController::Qemu {
                controller,
                effect,
            } => {
                let deps = Self::qemu_dependencies(value, dependencies, &children, effect)?;
                let outcome = controller
                    .reconcile(&deps, effect)
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                Ok(if matches!(
                    outcome,
                    qemu_media_runtime::QemuMediaReconcileOutcome::Ready
                ) {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                })
            }
            GuestRuntimeController::Aca { controller } => {
                let operation = aca_runtime::AcaOperationId::parse(
                    Self::framework_operation_id("aca", &context.operation_id),
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                let outcome = controller
                    .reconcile(operation, 30_000)
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                Ok(if outcome == aca_runtime::AcaReconcileOutcome::Converged {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                })
            }
            GuestRuntimeController::AzureVm { controller } => {
                let outcome = controller
                    .reconcile(
                        self.zone.as_str(),
                        resource.key().uid().as_str(),
                        resource.generation().get(),
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                Ok(if outcome == azure_vm_runtime::AzureVmReconcileOutcome::Converged {
                    SharedProviderEffectPhase::Ready
                } else {
                    SharedProviderEffectPhase::Pending
                })
            }
        }
    }

    async fn finalize_guest_controller(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<
        (
            bool,
            (ResourceRef, ResourceUid, u64, u64, u64, u64),
        ),
        SharedProviderEffectError,
    > {
        let value = self
            .guest_provider_resource(kind, context, resource)
            .await?;
        let runtime = self
            .validate_guest_runtime_fence(kind, context, resource)
            .await?;
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let provider = runtime
            .committed_resource_value(&provider_ref, &context.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if matches!(
            kind,
            SharedProviderResourceKind::AzureContainerAppsGuest
                | SharedProviderResourceKind::AzureVirtualMachineGuest
        ) {
            self.validate_gateway_custody(
                &provider_ref,
                match kind {
                    SharedProviderResourceKind::AzureContainerAppsGuest => {
                        &["controlCredentialRef", "pullCredentialRef"][..]
                    }
                    SharedProviderResourceKind::AzureVirtualMachineGuest => {
                        &["armCredentialRef"][..]
                    }
                    _ => &[][..],
                },
                context,
            )
            .await?;
        }
        let key = (
            resource.key().resource_ref().clone(),
            resource.key().uid().clone(),
            context.identity.provider_generation().get(),
            context.identity.controller_generation().get(),
            resource.generation().get(),
            runtime
                .core_controller_subject
                .lock()
                .map_err(|_| SharedProviderEffectError::Unavailable)?
                .as_ref()
                .map(AuthenticatedSubjectContext::reconnect_generation)
                .ok_or(SharedProviderEffectError::Unavailable)?
                .get(),
        );
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            controllers.insert(
                key.clone(),
                self.build_guest_controller(kind, context, resource, &value, &provider)?,
            );
        }
        let controller = controllers
            .get_mut(&key)
            .ok_or(SharedProviderEffectError::Unavailable)?;
        match controller {
            GuestRuntimeController::Qemu {
                controller,
                effect,
            } => {
                controller
                    .finalize(effect)
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
            }
            GuestRuntimeController::Aca { controller } => {
                let operation = aca_runtime::AcaOperationId::parse(
                    Self::framework_operation_id("aca-delete", &context.operation_id),
                )
                .map_err(|_| SharedProviderEffectError::InvalidResource)?;
                controller
                    .finalize(operation, 30_000)
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
            }
            GuestRuntimeController::AzureVm { controller } => {
                if let Some(operation) = controller.recovery_state().operation {
                    controller
                        .poll_operation(operation)
                        .await
                        .map_err(|_| SharedProviderEffectError::Unavailable)?;
                }
                controller
                    .finalize(
                        self.zone.as_str(),
                        resource.key().uid().as_str(),
                        resource.generation().get(),
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
            }
        }
        let complete = !controller.finalizer_installed();
        if complete {
            return Ok((true, key));
        }
        Ok((false, key))
    }

    async fn reconcile_guest_runtime(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        let value = self.guest_provider_resource(kind, context, resource).await?;
        for dependency in dependencies {
            if !Self::related_guest_dependency(&value, dependency)? {
                // Bring-up observability: a Guest held on an unready
                // dependency is invisible otherwise - log which
                // dependency holds it.
                tracing::warn!(
                    guest = %resource.key().resource_ref().to_canonical_string(),
                    dependency = %dependency
                        .resource()
                        .key()
                        .resource_ref()
                        .to_canonical_string(),
                    "guest held on unready dependency",
                );
                return Ok(SharedProviderEffectResult::phase(
                    SharedProviderEffectPhase::Pending,
                ));
            }
        }
        let runtime = self.validate_guest_runtime_fence(kind, context, resource).await?;
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let provider = runtime
            .committed_resource_value(&provider_ref, &context.operation_id)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?;
        if kind == SharedProviderResourceKind::AzureContainerAppsGuest {
            self.validate_gateway_custody(
                &provider_ref,
                &["controlCredentialRef", "pullCredentialRef"],
                context,
            )
            .await?;
        } else if kind == SharedProviderResourceKind::AzureVirtualMachineGuest {
            self.validate_gateway_custody(&provider_ref, &["armCredentialRef"], context)
                .await?;
        }
        if kind == SharedProviderResourceKind::QemuMediaGuest {
            Self::validate_qemu_guest(&value)?;
        }
        let desired = match kind {
            SharedProviderResourceKind::QemuMediaGuest => Some(Self::qemu_guest_children(
                &value,
                &provider,
                resource.key().resource_ref(),
                &self.zone,
            )?),
            SharedProviderResourceKind::AzureContainerAppsGuest => Some(
                Self::aca_guest_children(resource.key().resource_ref(), &self.zone)?,
            ),
            SharedProviderResourceKind::AzureVirtualMachineGuest
            | SharedProviderResourceKind::CloudHypervisorGuest => None,
        };
        let (children_ready, child_mutated) = if let Some(desired) = desired {
            let child_progress = self
                .guest_child_progress(&runtime, resource, Some(desired.clone()))
                .await?;
            (
                self.guest_children_ready(&runtime, resource, &desired)
                    .await?,
                child_progress == OneOwnedChildProgress::Mutated,
            )
        } else {
            (true, false)
        };
        match kind {
            SharedProviderResourceKind::CloudHypervisorGuest => {
                let runtime = self.runtime()?;
                let endpoint_outcome = runtime
                    .reconcile_cloud_hypervisor_guest(
                        Arc::clone(&self.state),
                        resource.key().resource_ref(),
                    )
                    .await
                    .map_err(|error| {
                        tracing::warn!(
                            resource = %resource.key().resource_ref().to_canonical_string(),
                            error = ?error,
                            "U6 Cloud Hypervisor Guest effect failed",
                        );
                        SharedProviderEffectError::Unavailable
                    })?;
                let fresh = runtime
                    .committed_resource_value(
                        resource.key().resource_ref(),
                        &context.operation_id,
                    )
                    .await
                    .map_err(|_| SharedProviderEffectError::Unavailable)?;
                Ok(SharedProviderEffectResult {
                    phase: if endpoint_outcome == CloudHypervisorReconcileOutcome::Ready {
                        Self::guest_phase(&fresh)
                    } else {
                        SharedProviderEffectPhase::Pending
                    },
                    child_mutated: false,
                    resource_projection: None,
                })
            }
            SharedProviderResourceKind::QemuMediaGuest => {
                let phase = self
                    .run_guest_controller(
                    kind,
                    context,
                    resource,
                    &value,
                    &provider,
                    dependencies,
                )
                .await?;
                Ok(SharedProviderEffectResult {
                    phase: if children_ready {
                        phase
                    } else {
                        SharedProviderEffectPhase::Pending
                    },
                    child_mutated,
                    resource_projection: None,
                })
            }
            SharedProviderResourceKind::AzureContainerAppsGuest => {
                let phase = self
                    .run_guest_controller(
                    kind,
                    context,
                    resource,
                    &value,
                    &provider,
                    dependencies,
                )
                .await?;
                Ok(SharedProviderEffectResult {
                    phase: if children_ready {
                        phase
                    } else {
                        SharedProviderEffectPhase::Pending
                    },
                    child_mutated,
                    resource_projection: None,
                })
            }
            SharedProviderResourceKind::AzureVirtualMachineGuest => {
                Self::validate_azure_vm_guest(&value)?;
                let phase = self
                    .run_guest_controller(
                    kind,
                    context,
                    resource,
                    &value,
                    &provider,
                    dependencies,
                )
                .await?;
                Ok(SharedProviderEffectResult {
                    phase: if children_ready {
                        phase
                    } else {
                        SharedProviderEffectPhase::Pending
                    },
                    child_mutated,
                    resource_projection: None,
                })
            }
        }
    }

    async fn finalize_guest_runtime(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedProviderEffectError> {
        let _value = self.guest_provider_resource(kind, context, resource).await?;
        if kind == SharedProviderResourceKind::CloudHypervisorGuest {
            self.runtime()?
                .reconcile_cloud_hypervisor_guest(
                    Arc::clone(&self.state),
                    resource.key().resource_ref(),
                )
                .await
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            return Ok(());
        }
        let runtime = self.runtime()?;
        let (complete, controller_key) = self
            .finalize_guest_controller(kind, context, resource)
            .await?;
        if !complete {
            return Err(SharedProviderEffectError::Pending);
        }
        let child_progress = self.guest_child_progress(&runtime, resource, None).await?;
        match child_progress {
            OneOwnedChildProgress::Converged => {
                self.guest_controllers.lock().await.remove(&controller_key);
                Ok(())
            }
            OneOwnedChildProgress::Mutated | OneOwnedChildProgress::Pending => {
                Err(SharedProviderEffectError::Pending)
            }
        }
    }
}

#[async_trait]
impl SharedProviderEffectExecutor for DaemonSharedProviderEffects {
    async fn reconcile_guest(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        self.reconcile_guest_runtime(kind, context, resource, dependencies)
            .await
            .map(|result| result.phase)
    }

    async fn reconcile_guest_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_guest_runtime(kind, context, resource, dependencies)
            .await
    }

    async fn observe_result(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedProviderEffectResult, SharedProviderEffectError> {
        self.reconcile_guest_runtime(kind, context, resource, &[])
            .await
    }

    async fn finalize_guest(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedProviderEffectError> {
        self.finalize_guest_runtime(kind, context, resource).await
    }

    async fn finalize(
        &self,
        kind: SharedProviderResourceKind,
        context: &SharedProviderEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedProviderEffectError> {
        self.finalize_guest_runtime(kind, context, resource).await
    }
}

/// Shared Runner adapter that delegates to one closed, typed Provider
/// controller rather than the generic Core metadata reconciler.
pub(crate) struct SharedProviderResourceReconciler {
    descriptor: ControllerDescriptor,
    kind: SharedProviderResourceKind,
    effects: Arc<dyn SharedProviderEffectExecutor>,
}

/// Shared Runner reconciler used by the selected Guest runtime Providers.
pub(crate) type GuestRuntimeReconciler = SharedProviderResourceReconciler;
const SHARED_PROVIDER_PROGRESS_REQUEUE_TICKS: u64 = 1_000;

impl SharedProviderResourceReconciler {
    pub(super) fn new(
        descriptor: ControllerDescriptor,
        kind: SharedProviderResourceKind,
        effects: Arc<dyn SharedProviderEffectExecutor>,
    ) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            kind,
            effects,
        })
    }

    fn effect_context(&self, context: &ReconcileContext) -> SharedProviderEffectContext {
        SharedProviderEffectContext {
            identity: context.identity().clone(),
            target: context.target().clone(),
            operation_id: context.operation().operation_id().to_owned(),
        }
    }

    fn has_finalizer(&self, resource: &ResourceSnapshot) -> Result<bool, SharedProviderReconcileError> {
        if self.descriptor.finalizers().is_empty() {
            return Ok(true);
        }
        let value = serde_json::from_slice::<Value>(resource.canonical_json())
            .map_err(|_| SharedProviderReconcileError::InvalidResource)?;
        Ok(self.descriptor.finalizers().iter().all(|expected| {
            value
                .pointer("/metadata/finalizers")
                .and_then(Value::as_array)
                .is_some_and(|finalizers| {
                    finalizers
                        .iter()
                        .any(|value| value.as_str() == Some(expected))
                })
        }))
    }

    fn status_candidate(
        resource: &ResourceSnapshot,
        phase: Option<SharedProviderEffectPhase>,
    ) -> Result<Vec<u8>, SharedProviderReconcileError> {
        let mut value = serde_json::from_slice::<Value>(resource.canonical_json())
            .map_err(|_| SharedProviderReconcileError::InvalidResource)?;
        let status = value
            .get_mut("status")
            .and_then(Value::as_object_mut)
            .ok_or(SharedProviderReconcileError::InvalidResource)?;
        if let Some(phase) = phase {
            status.insert(
                "phase".to_owned(),
                Value::String(match phase {
                    SharedProviderEffectPhase::Ready => "Ready".to_owned(),
                    SharedProviderEffectPhase::Pending => "Pending".to_owned(),
                }),
            );
        }
        serde_json::to_vec(status).map_err(|_| SharedProviderReconcileError::InvalidResource)
    }

    fn finalizer_mutation(
        resource: &ResourceSnapshot,
        finalizer: &str,
        add: bool,
    ) -> Result<ResourceMutationBatch, SharedProviderReconcileError> {
        let canonical = finalizer_candidate(resource.canonical_json(), finalizer, add)?;
        let mutation = d2b_core_controller::MutationIntent::new(
            resource.key().resource_ref().clone(),
            Some(resource.key().uid().clone()),
            Some(resource.revision()),
            d2b_core_controller::MutationIntentKind::UpdateFinalizers,
            Some(canonical),
        )
        .map_err(|_| SharedProviderReconcileError::InvalidResource)?;
        ResourceMutationBatch::new(vec![mutation])
            .map_err(|_| SharedProviderReconcileError::InvalidResource)
    }

    fn status_candidate_for_result(
        &self,
        resource: &ResourceSnapshot,
        result: &SharedProviderEffectResult,
    ) -> Result<Option<Vec<u8>>, SharedProviderReconcileError> {
        if self.kind == SharedProviderResourceKind::CloudHypervisorGuest {
            // The live CH controller owns its layered Guest status. Do not
            // replace its freshly committed conditions with this Runner's
            // bounded generic projection.
            Ok(None)
        } else {
            let mut status = serde_json::from_slice::<Value>(resource.canonical_json())
                .map_err(|_| SharedProviderReconcileError::InvalidResource)?
                .get("status")
                .cloned()
                .ok_or(SharedProviderReconcileError::InvalidResource)?;
            let status = status
                .as_object_mut()
                .ok_or(SharedProviderReconcileError::InvalidResource)?;
            status.insert(
                "phase".to_owned(),
                Value::String(match result.phase {
                    SharedProviderEffectPhase::Ready => "Ready",
                    SharedProviderEffectPhase::Pending => "Pending",
                }
                .to_owned()),
            );
            if matches!(
                self.kind,
                SharedProviderResourceKind::QemuMediaGuest
                    | SharedProviderResourceKind::AzureContainerAppsGuest
                    | SharedProviderResourceKind::AzureVirtualMachineGuest
            ) {
                status.insert(
                    "observedGeneration".to_owned(),
                    Value::from(resource.generation().get()),
                );
            }
            if let Some(projection) = &result.resource_projection {
                status.insert("resource".to_owned(), projection.clone());
            }
            serde_json::to_vec(status)
                .map(Some)
                .map_err(|_| SharedProviderReconcileError::InvalidResource)
        }
    }

    #[cfg(test)]
    pub(super) fn first_pass_for_test(
        &self,
        resource: &ResourceSnapshot,
    ) -> Result<ReconcileResult, SharedProviderReconcileError> {
        let Some(finalizer) = self.descriptor.finalizers().first() else {
            return ReconcileResult::new(
                resource.revision(),
                resource.generation(),
                None,
                None,
                ReconcileDisposition::Pending,
                None,
                None,
                StatusPersistence::NotRequested,
            )
            .map_err(|_| SharedProviderReconcileError::InvalidResource);
        };
        if resource.deleting() || self.has_finalizer(resource)? {
            return Ok(ReconcileResult::converged(
                resource.revision(),
                resource.generation(),
            ));
        }
        ReconcileResult::new(
            resource.revision(),
            resource.generation(),
            Some(Self::finalizer_mutation(resource, finalizer, true)?),
            None,
            ReconcileDisposition::Pending,
            None,
            None,
            StatusPersistence::NotRequested,
        )
        .map_err(|_| SharedProviderReconcileError::InvalidResource)
    }


}

pub(super) fn finalizer_candidate(
    canonical_json: &[u8],
    finalizer: &str,
    add: bool,
) -> Result<Vec<u8>, SharedProviderReconcileError> {
    let mut value = CanonicalJsonValue::parse(canonical_json)
        .map_err(|_| SharedProviderReconcileError::InvalidResource)?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(SharedProviderReconcileError::InvalidResource);
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(SharedProviderReconcileError::InvalidResource);
    };
    let Some(CanonicalJsonValue::Array(finalizers)) = metadata.get_mut("finalizers") else {
        return Err(SharedProviderReconcileError::InvalidResource);
    };
    if add {
        if !finalizers
            .iter()
            .any(|value| matches!(value, CanonicalJsonValue::String(value) if value == finalizer))
        {
            finalizers.push(CanonicalJsonValue::String(finalizer.to_owned()));
        }
    } else {
        finalizers.retain(
            |value| !matches!(value, CanonicalJsonValue::String(value) if value == finalizer),
        );
    }
    Ok(value.to_canonical_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderReconcileError {
    InvalidResource,
    Effect(SharedProviderEffectError),
}

impl core::fmt::Display for SharedProviderReconcileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidResource => formatter.write_str("shared-provider-resource-invalid"),
            Self::Effect(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SharedProviderReconcileError {}

impl ResourceReconciler for SharedProviderResourceReconciler {
    type Error = SharedProviderReconcileError;

    fn describe(
        &self,
    ) -> impl std::future::Future<Output = Result<ControllerDescriptor, Self::Error>> + Send {
        std::future::ready(Ok(self.descriptor.clone()))
    }

    fn validate_spec(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<ValidationResult, Self::Error>> + Send {
        let valid = context.identity().zone() == resource.key().zone()
            && resource.key().resource_ref().resource_type().as_str() == self.kind.resource_type()
            && serde_json::from_slice::<Value>(resource.canonical_json())
                .ok()
                .is_some_and(|value| {
                    value
                        .pointer("/spec/providerRef")
                        .and_then(Value::as_str)
                        == Some(self.kind.provider_ref())
                });
        std::future::ready(Ok(if valid {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid {
                reason: ReconcileReason::InvalidSpec,
            }
        }))
    }

    async fn plan(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> Result<ReconcilePlan, Self::Error> {
        let _ = self.has_finalizer(resource)?;
        ReconcilePlan::new(vec![self.kind.effect_id().to_owned()], false)
            .map_err(|_| SharedProviderReconcileError::InvalidResource)
    }

    fn reconcile(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> impl std::future::Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        let result = (|| {
            context
                .authorize_effect()
                .map_err(|_| SharedProviderReconcileError::Effect(
                    SharedProviderEffectError::Unavailable,
                ))?;
            let finalizer = self.descriptor.finalizers().first();
            if let Some(finalizer) = finalizer
                && !resource.deleting()
                && !self.has_finalizer(resource)?
            {
                return Ok(ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    Some(Self::finalizer_mutation(resource, finalizer, true)?),
                    None,
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::NotRequested,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource)?);
            }
            if finalizer.is_none() {
                return ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    None,
                    None,
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::NotRequested,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource);
            }
            if !self.has_finalizer(resource)? {
                return ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    None,
                    Some(Self::status_candidate(resource, Some(
                        SharedProviderEffectPhase::Pending,
                    ))?),
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::Pending,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource);
            }
            Ok(ReconcileResult::converged(
                resource.revision(),
                resource.generation(),
            ))
        })();
        std::future::ready(result)
    }

    async fn execute_effect(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> Result<ReconcileResult, Self::Error> {
        let _permit = context
            .authorize_effect()
            .map_err(|_| SharedProviderReconcileError::Effect(
                SharedProviderEffectError::Unavailable,
            ))?;
        let result = self
            .effects
            .reconcile_result(
                self.kind,
                &self.effect_context(context),
                resource,
                dependencies,
            )
            .await
            .map_err(SharedProviderReconcileError::Effect)?;
        let status = (!result.child_mutated)
            .then(|| self.status_candidate_for_result(resource, &result))
            .transpose()?
            .flatten();
        let (disposition, next_tick) = if self.kind.resource_type() == "Guest"
            && result.phase == SharedProviderEffectPhase::Pending
        {
            (
                ReconcileDisposition::RequeueAt,
                Some(context.now_tick().saturating_add(
                    SHARED_PROVIDER_PROGRESS_REQUEUE_TICKS,
                )),
            )
        } else {
            (ReconcileDisposition::Pending, None)
        };
        let status_persistence = if status.is_some() {
            StatusPersistence::Pending
        } else {
            StatusPersistence::NotRequested
        };
        Ok(ReconcileResult::new(
            resource.revision(),
            resource.generation(),
            None,
            status,
            disposition,
            next_tick,
            None,
            status_persistence,
        )
        .map_err(|_| SharedProviderReconcileError::InvalidResource)?)
    }

    async fn observe(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> Result<ObservationResult, Self::Error> {
        let _permit = context
            .authorize_effect()
            .map_err(|_| SharedProviderReconcileError::Effect(
                SharedProviderEffectError::Unavailable,
            ))?;
        if !self.has_finalizer(resource)? {
            return Ok(ObservationResult::new(
                ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    None,
                    Some(Self::status_candidate(
                        resource,
                        Some(SharedProviderEffectPhase::Pending),
                    )?),
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::Pending,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource)?,
            ));
        }
        let result = self
            .effects
            .observe_result(
                self.kind,
                &self.effect_context(context),
                resource,
            )
            .await
            .map_err(SharedProviderReconcileError::Effect)?;
        let status = (!result.child_mutated)
            .then(|| self.status_candidate_for_result(resource, &result))
            .transpose()?
            .flatten();
        let (disposition, next_tick) = if self.kind.resource_type() == "Guest"
            && result.phase == SharedProviderEffectPhase::Pending
        {
            (
                ReconcileDisposition::RequeueAt,
                Some(context.now_tick().saturating_add(
                    SHARED_PROVIDER_PROGRESS_REQUEUE_TICKS,
                )),
            )
        } else {
            (ReconcileDisposition::Pending, None)
        };
        let status_persistence = if status.is_some() {
            StatusPersistence::Pending
        } else {
            StatusPersistence::NotRequested
        };
        Ok(ObservationResult::new(
            ReconcileResult::new(
                resource.revision(),
                resource.generation(),
                None,
                status,
                disposition,
                next_tick,
                None,
                status_persistence,
            )
            .map_err(|_| SharedProviderReconcileError::InvalidResource)?,
        ))
    }

    fn finalize(
        &self,
        _context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<FinalizeResult, Self::Error>> + Send {
        std::future::ready(Ok(FinalizeResult::new(ReconcileResult::converged(
            deleting_resource.revision(),
            deleting_resource.generation(),
        ))))
    }

    fn prepare_finalize(
        &self,
        context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        let result = context
            .authorize_effect()
            .map(|_| ReconcileResult::converged(
                deleting_resource.revision(),
                deleting_resource.generation(),
            ))
            .map_err(|_| SharedProviderReconcileError::Effect(
                SharedProviderEffectError::Unavailable,
            ));
        std::future::ready(result)
    }

    async fn execute_finalize(
        &self,
        context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> Result<ReconcileResult, Self::Error> {
        let _permit = context
            .authorize_effect()
            .map_err(|_| SharedProviderReconcileError::Effect(
                SharedProviderEffectError::Unavailable,
            ))?;
        if !self.has_finalizer(deleting_resource)? {
            return Ok(ReconcileResult::converged(
                deleting_resource.revision(),
                deleting_resource.generation(),
            ));
        }
        let finalizer = self.descriptor.finalizers().first().cloned();
        match self
            .effects
            .finalize(
                self.kind,
                &self.effect_context(context),
                deleting_resource,
            )
            .await
        {
            Ok(()) => {}
            Err(SharedProviderEffectError::Pending) => {
                return ReconcileResult::new(
                    deleting_resource.revision(),
                    deleting_resource.generation(),
                    None,
                    None,
                    ReconcileDisposition::RequeueAt,
                    Some(
                        context
                            .now_tick()
                            .saturating_add(SHARED_PROVIDER_PROGRESS_REQUEUE_TICKS),
                    ),
                    None,
                    StatusPersistence::NotRequested,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource);
            }
            Err(error) => return Err(SharedProviderReconcileError::Effect(error)),
        }
        let Some(finalizer) = finalizer else {
            return Ok(ReconcileResult::converged(
                deleting_resource.revision(),
                deleting_resource.generation(),
            ));
        };
        Ok(
            ReconcileResult::new(
                deleting_resource.revision(),
                deleting_resource.generation(),
                Some(Self::finalizer_mutation(
                    deleting_resource,
                    &finalizer,
                    false,
                )?),
                None,
                ReconcileDisposition::Pending,
                None,
                None,
                StatusPersistence::NotRequested,
            )
            .map_err(|_| SharedProviderReconcileError::InvalidResource)?,
        )
    }

    fn health(
        &self,
    ) -> impl std::future::Future<Output = Result<d2b_core_controller::ControllerHealth, Self::Error>>
        + Send {
        std::future::ready(Ok(d2b_core_controller::ControllerHealth::Healthy))
    }

    fn drain(
        &self,
        _deadline_tick: u64,
    ) -> impl std::future::Future<Output = Result<DrainResult, Self::Error>> + Send {
        std::future::ready(Ok(DrainResult::Drained))
    }

    fn assess_update(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl std::future::Future<Output = Result<UpdateAssessment, Self::Error>> + Send {
        let state = serde_json::from_slice::<Value>(resource.canonical_json())
            .ok()
            .map(|value| {
                let observed_generation = value
                    .pointer("/status/observedGeneration")
                    .and_then(Value::as_u64);
                let initial_pending = observed_generation == Some(0)
                    && value.pointer("/status/phase").and_then(Value::as_str)
                        == Some("Pending");
                if observed_generation == Some(resource.generation().get()) || initial_pending {
                    UpdateAssessmentState::Current
                } else {
                    UpdateAssessmentState::UpgradeRequired
                }
            })
            .unwrap_or(UpdateAssessmentState::UpgradeRequired);
        std::future::ready(
            UpdateAssessment::new(state, Vec::new(), true)
                .map_err(|_| SharedProviderReconcileError::InvalidResource),
        )
    }

    fn plan_upgrade(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl std::future::Future<Output = Result<UpgradePlan, Self::Error>> + Send {
        std::future::ready(
            UpgradePlan::new(
                DisruptionClass::Restart,
                true,
                vec![UpgradeStage::Restart(resource.key().resource_ref().clone())],
            )
            .map_err(|_| SharedProviderReconcileError::InvalidResource),
        )
    }

    async fn execute_upgrade(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        _plan: &UpgradePlan,
    ) -> Result<ReconcileResult, Self::Error> {
        let _permit = context
            .authorize_effect()
            .map_err(|_| SharedProviderReconcileError::Effect(
                SharedProviderEffectError::Unavailable,
            ))?;
        if !self.has_finalizer(resource)? {
            return Ok(
                ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    None,
                    Some(Self::status_candidate(
                        resource,
                        Some(SharedProviderEffectPhase::Pending),
                    )?),
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::Pending,
                )
                .map_err(|_| SharedProviderReconcileError::InvalidResource)?,
            );
        }
        let result = self
            .effects
            .upgrade_result(
                self.kind,
                &self.effect_context(context),
                resource,
                dependencies,
            )
            .await
            .map_err(SharedProviderReconcileError::Effect)?;
        let status = (!result.child_mutated)
            .then(|| self.status_candidate_for_result(resource, &result))
            .transpose()?
            .flatten();
        let (disposition, next_tick) = if self.kind.resource_type() == "Guest"
            && result.phase == SharedProviderEffectPhase::Pending
        {
            (
                ReconcileDisposition::RequeueAt,
                Some(context.now_tick().saturating_add(
                    SHARED_PROVIDER_PROGRESS_REQUEUE_TICKS,
                )),
            )
        } else {
            (ReconcileDisposition::Pending, None)
        };
        let status_persistence = if status.is_some() {
            StatusPersistence::Pending
        } else {
            StatusPersistence::NotRequested
        };
        Ok(
            ReconcileResult::new(
                resource.revision(),
                resource.generation(),
                None,
                status,
                disposition,
                next_tick,
                None,
                status_persistence,
            )
            .map_err(|_| SharedProviderReconcileError::InvalidResource)?,
        )
    }
}

/// Compose exact Provider descriptors used by the production shared Runner.
/// The provider-generation map is supplied by authoritative Provider rows; no
/// generation or assignment epoch is guessed.
pub fn compose_shared_provider_runner_descriptors(
    registrations: impl IntoIterator<Item = SharedProviderRunnerRegistration>,
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    provider_generations: &BTreeMap<ResourceRef, ResourceGeneration>,
    _session_generation: ReconnectGeneration,
) -> Result<
    Vec<(SharedProviderRunnerRegistration, ControllerDescriptor)>,
    ResourceRuntimeError,
> {
    registrations
        .into_iter()
        .map(|registration| {
            if !registration.watched_configuration_is_dependency
                || !(30_000..=300_000).contains(&registration.repair_interval_ticks)
            {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            let provider_ref = ResourceRef::parse(registration.provider_ref)
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let provider_generation = provider_generations
                .get(&provider_ref)
                .copied()
                .ok_or(ResourceRuntimeError::HandlerNotReady)?;
            let resource_type = ResourceTypeName::parse(registration.resource_type.to_owned())
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let controller_ref = ResourceRef::parse(registration.controller_ref)
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let identity = ControllerIdentity::new(
                zone.clone(),
                controller_ref.clone(),
                controller_generation,
                provider_ref,
                provider_generation,
                controller_ref,
                ResourceRef::parse(CORE_CONTROLLER_HOST_REF)
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                None,
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let resource = ResourceRegistration::new(
                resource_type.clone(),
                vec![1],
                5_000,
                3,
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let provider_selector = Some(registration.provider_ref.to_owned());
            let mut selectors = vec![
                ControllerSelector::new(
                    resource_type.clone(),
                    SelectorField::Spec,
                    provider_selector,
                )
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
            ];
            for field in [
                SelectorField::Status,
                SelectorField::Metadata,
                SelectorField::Finalizers,
                SelectorField::Deletion,
            ] {
                selectors.push(
                    ControllerSelector::new(resource_type.clone(), field, None)
                        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                );
            }
            let dependency_types: &[&str] = match registration.resource_type {
                "Guest" => &[
                    "Provider",
                    "Process",
                    "EphemeralProcess",
                    "Endpoint",
                    "Volume",
                    "Network",
                    "Device",
                    "Credential",
                ],
                _ => &[],
            };
            let dependency_selectors = dependency_types
                .iter()
                .map(|resource_type| {
                    ControllerSelector::new(
                        ResourceTypeName::parse((*resource_type).to_owned())
                            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                        SelectorField::Metadata,
                        None,
                    )
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let execution = ControllerExecutionPolicy::new(
                8,
                4,
                256,
                8,
                256,
                ResyncPolicy::new(
                    Some(registration.repair_interval_ticks),
                    registration.repair_interval_ticks,
                )
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            let mut verbs = vec![
                ControllerVerb::ReadSpec,
                ControllerVerb::ReadStatus,
                ControllerVerb::WriteStatus,
                ControllerVerb::AddFinalizer,
                ControllerVerb::RemoveFinalizer,
            ];
            if registration.resource_type == "Guest" {
                verbs.push(ControllerVerb::WriteSpec);
            }
            let descriptor = ControllerDescriptor::new(
                identity,
                vec![resource],
                vec!["resource-api".to_owned()],
                vec!["system".to_owned()],
                verbs,
                selectors,
                dependency_selectors,
                true,
                if registration.finalizer.is_empty() {
                    Vec::new()
                } else {
                    vec![registration.finalizer.to_owned()]
                },
                vec!["d2b.resource.v3".to_owned()],
                vec!["resources.d2bus.org/v3".to_owned()],
                execution,
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            Ok((registration, descriptor))
        })
        .collect()
}