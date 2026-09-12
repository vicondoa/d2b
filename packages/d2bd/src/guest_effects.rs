//! Production effects for the converted Guest runtime-provider drivers (U12
//! wave 2).
//!
//! Every typed Provider effect the Guest family dispatches lives here: the
//! Cloud Hypervisor Guest controller session (the real host path), and the
//! preserved framework state machines for the qemu-media,
//! azure-container-apps, and azure-virtual-machine Providers. The port is the
//! dyn-erased [`GuestDriverEffects`] boundary; the daemon owns every side
//! effect behind it, and the driver owns the child rows.
//!
//! Live readiness is read through the manager view: a row's actor status is
//! the only status there is (R11), and U14 retired the durable store. The
//! driver never sees the read path.
//!
//! The Cloud Hypervisor provider controller publishes the Guest's layered
//! runtime status (`phase`, `runtimeReady`, `bootstrapReady`,
//! `activeProcessCount`). That status is the row's actor's to own (R11) and
//! there is no durable row to write: the session captures the controller's
//! write into the effect call's [`GuestStatusSink`] and the driver publishes
//! it as the row's `status.resource` projection. The provider controller's
//! finalizer requests are acknowledged without a store write for the same
//! reason - the manager's deleting-row hold replaces the old durable
//! finalizer (F3).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_resource_runtime::context::{LookupPlane, RowLookup};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_provider_runtime_azure_container_apps as aca_runtime;
use d2b_provider_runtime_azure_virtual_machine as azure_vm_runtime;
use d2b_provider_runtime_qemu_media as qemu_media_runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ServerState;
use crate::guest_driver::{
    GuestChildObservation, GuestDriverEffects, GuestEffectError, GuestEffectOutcome,
    GuestEffectPhase, GuestEffectRequest, GuestFinalizeStage, GuestKind,
    declared_dependency_refs, view_phase,
};
use crate::resource_plane_v3::ResourcePlaneV3;
use crate::resource_runtime::ZoneResourceRuntime;

/// Framework-only QEMU effect evidence for non-Cloud-Hypervisor Guest owners.
///
/// The real Process/ComponentSession path remains owned by the selected
/// child Providers; this adapter exercises the typed lifecycle state machine
/// without claiming Cloud Hypervisor host liveness.
struct FrameworkQemuEffect {
    guest_ref: ResourceRef,
    identity: Option<qemu_media_runtime::ProcessIdentity>,
    qmp_ready: bool,
}

impl FrameworkQemuEffect {
    fn new(guest_ref: ResourceRef) -> Self {
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
    ) -> Result<Option<qemu_media_runtime::ProcessIdentity>, qemu_media_runtime::QemuMediaError> {
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

struct FrameworkAcaState {
    provider_generation: u64,
    disk_image: Option<aca_runtime::AcaDiskImageRecord>,
    sandbox: Option<aca_runtime::AcaSandboxRecord>,
}

impl FrameworkAcaState {
    fn new(provider_generation: u64) -> Self {
        Self {
            provider_generation,
            disk_image: None,
            sandbox: None,
        }
    }
}

struct FrameworkAcaControl {
    state: Arc<tokio::sync::Mutex<FrameworkAcaState>>,
}

struct FrameworkAcaLease;

#[async_trait]
impl aca_runtime::AcaCredentialLeaseClient for FrameworkAcaLease {
    async fn acquire(
        &self,
        request: &aca_runtime::AcaCredentialLeaseRequest,
    ) -> Result<aca_runtime::AcaCredentialLease, aca_runtime::AcaControlError> {
        let handle = d2b_contracts_provider::v3::credential::CredentialLeaseHandle::parse(
            "guest-framework-lease",
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
        Ok(if self.state.lock().await.sandbox.as_ref().is_some_and(|sandbox| {
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
    ) -> Result<aca_runtime::AcaSandboxCandidates, aca_runtime::AcaControlError> {
        let mut state = self.state.lock().await;
        if state
            .sandbox
            .as_ref()
            .is_some_and(|sandbox| sandbox.lifecycle == aca_runtime::AcaSandboxLifecycle::Creating)
            && let Some(sandbox) = state.sandbox.as_mut()
        {
            sandbox.lifecycle = aca_runtime::AcaSandboxLifecycle::Running;
        }
        aca_runtime::AcaSandboxCandidates::new(state.sandbox.clone().into_iter().collect())
            .map_err(|_| {
                aca_runtime::AcaControlError::new(aca_runtime::AcaControlErrorKind::InvalidResponse)
            })
    }

    async fn find_disk_images(
        &self,
        _lease: &aca_runtime::AcaCredentialLease,
        _context: &aca_runtime::AcaControlContext,
        _desired: &aca_runtime::AcaDesiredDiskImage,
    ) -> Result<aca_runtime::AcaDiskImageCandidates, aca_runtime::AcaControlError> {
        let state = self.state.lock().await;
        aca_runtime::AcaDiskImageCandidates::new(state.disk_image.clone().into_iter().collect())
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
            id: aca_runtime::AcaDiskImageId::parse("guest-framework-disk").map_err(|_| {
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
            id: aca_runtime::AcaSandboxId::parse("guest-framework-sandbox").map_err(|_| {
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

struct FrameworkAzureState {
    state: azure_vm_runtime::AzureVmState,
    handle: Option<azure_vm_runtime::AzureVmHandle>,
    tags: azure_vm_runtime::TagDigest,
    operation: Option<(azure_vm_runtime::AzureOperationHandle, FrameworkAzureOperation)>,
    extension_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameworkAzureOperation {
    Provision,
    Delete,
    ChildCleanup,
    Extension,
    Update,
}

impl FrameworkAzureState {
    fn new(settings: &azure_vm_runtime::AzureVmGuestSettings) -> Self {
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
            format!("guest-{operation_id}-{kind:?}"),
        )?;
        self.operation = Some((operation.clone(), kind));
        Ok(operation)
    }
}

struct FrameworkAzureEffect {
    state: Arc<tokio::sync::Mutex<FrameworkAzureState>>,
}

struct FrameworkAzureCredential;

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
                state.handle = Some(azure_vm_runtime::AzureVmHandle::from_core(
                    "guest-framework-vm",
                )?);
            }
            FrameworkAzureOperation::Delete => {
                state.state = azure_vm_runtime::AzureVmState::Absent;
                state.handle = None;
            }
            FrameworkAzureOperation::Extension => state.extension_present = false,
            FrameworkAzureOperation::ChildCleanup | FrameworkAzureOperation::Update => {}
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

/// In-memory Provider controller for one framework Guest.
enum GuestRuntimeController {
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

/// Per-resource key of one framework controller (old `guest_controllers`
/// key, unchanged: provider/controller/session generations fence reuse).
type GuestControllerKey = (ResourceRef, ResourceUid, u64, u64, u64, u64);

/// Production composition adapter for the closed Guest runtime-Provider set.
///
/// The adapter performs the Provider-owned typed admission before any effect
/// call. A missing live broker/resource binding is returned as a retryable
/// refusal; it is never converted into generic convergence.
pub(crate) struct ProductionGuestDriverEffects {
    state: Arc<ServerState>,
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    /// Framework controllers (old `guest_controllers`): in-memory only, one
    /// slot per resource and generation set.
    guest_controllers: Arc<tokio::sync::Mutex<BTreeMap<GuestControllerKey, GuestRuntimeController>>>,
}

impl ProductionGuestDriverEffects {
    pub(crate) fn new(
        state: Arc<ServerState>,
        zone: ZoneId,
        controller_generation: ControllerGeneration,
    ) -> Self {
        Self {
            state,
            zone,
            controller_generation,
            guest_controllers: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        }
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, GuestEffectError> {
        self.state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(GuestEffectError::Unavailable)
    }

    /// The published v3 plane (manager rows and their live status).
    fn plane(&self) -> Result<Arc<ResourcePlaneV3>, GuestEffectError> {
        self.runtime()?
            .v3_plane()
            .map_err(|_| GuestEffectError::Unavailable)
    }

    /// The old-shape document of one resource (`spec`, `metadata`, live
    /// `status.phase`) from the manager view, answered as one classified read
    /// (issue #511): `Present` carries the document, `Absent` is the honest
    /// not-created answer, `Unavailable` is a plane that could not answer,
    /// and `Error` carries the projection detail of a committed row that
    /// cannot be read.
    async fn resource_value(&self, target: &ResourceRef) -> RowLookup<Value> {
        let Ok(plane) = self.plane() else {
            return RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            };
        };
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        let view = match plane.client().get(key).await {
            Ok(view) => view,
            Err(_) => {
                return RowLookup::Unavailable {
                    plane: LookupPlane::Manager,
                }
            }
        };
        let Some(view) = view else {
            return RowLookup::Absent {
                plane: LookupPlane::Manager,
            };
        };
        let spec = match serde_json::from_slice::<Value>(&view.spec) {
            Ok(spec) => spec,
            Err(error) => {
                return RowLookup::Error {
                    plane: LookupPlane::Manager,
                    detail: error.to_string(),
                }
            }
        };
        let metadata = if view.metadata.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice::<Value>(&view.metadata) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return RowLookup::Error {
                        plane: LookupPlane::Manager,
                        detail: error.to_string(),
                    }
                }
            }
        };
        let uid = match crate::guest_driver::resource_uid(&view.uid) {
            Ok(uid) => uid,
            Err(_) => {
                return RowLookup::Error {
                    plane: LookupPlane::Manager,
                    detail: "manager row uid is not a valid ResourceUid".to_owned(),
                }
            }
        };
        RowLookup::Present {
            row: json!({
                "spec": spec,
                "metadata": metadata,
                "status": {"phase": view_phase(&view)},
                "uid": uid.as_str(),
                "generation": view.generation,
            }),
            plane: LookupPlane::Manager,
        }
    }

    /// One row read that answered with an unreadable row: the plane and the
    /// projection detail are logged, and the effect refuses with its named
    /// terminal evidence (`InvalidResource`) - retrying cannot make a
    /// committed row decode.
    fn unreadable_row(plane: LookupPlane, detail: &str) -> GuestEffectError {
        tracing::warn!(
            plane = ?plane,
            detail,
            "guest effect row read answered with an unreadable row",
        );
        GuestEffectError::InvalidResource
    }

    /// Apply issue #511 to one classified row read on the effect surface:
    /// `Present` is the document, `Absent` and `Unavailable` defer through
    /// the caller's retryable refusal (`None`; the caller maps it to
    /// [`GuestEffectError::Unavailable`]), and `Error` names its terminal
    /// evidence. The plane and the detail never cross the driver boundary.
    fn projected_row(lookup: RowLookup<Value>) -> Result<Option<Value>, GuestEffectError> {
        match lookup {
            RowLookup::Present { row, .. } => Ok(Some(row)),
            RowLookup::Absent { .. } | RowLookup::Unavailable { .. } => Ok(None),
            RowLookup::Error { plane, detail } => Err(Self::unreadable_row(plane, &detail)),
        }
    }

    /// The live phase of one resource: the manager view for converted rows,
    /// the durable row's `/status/phase` for unconverted rows.
    async fn live_phase(&self, target: &ResourceRef) -> Result<Option<&'static str>, GuestEffectError> {
        let value = match self.resource_value(target).await {
            RowLookup::Present { row, .. } => row,
            RowLookup::Absent { .. } => return Ok(None),
            RowLookup::Unavailable { .. } => return Err(GuestEffectError::Unavailable),
            RowLookup::Error { plane, detail } => {
                return Err(Self::unreadable_row(plane, &detail))
            }
        };
        Ok(value
            .pointer("/status/phase")
            .and_then(Value::as_str)
            .map(|phase| match phase {
                "Ready" => "Ready",
                "Failed" => "Failed",
                "Deleted" => "Deleted",
                _ => "Pending",
            }))
    }

    /// Whether one resource is present, not deleting, and live-Ready.
    async fn resource_ready(&self, target: &ResourceRef) -> bool {
        matches!(self.live_phase(target).await, Ok(Some("Ready")))
    }

    /// The old `dependencies_ready` barrier over the row's declared
    /// dependency references: a Guest is held Pending while any dependency
    /// its spec names is not live-Ready.
    async fn dependencies_ready(
        &self,
        request: &GuestEffectRequest<'_>,
    ) -> Result<bool, GuestEffectError> {
        for dependency in declared_dependency_refs(&request.spec) {
            if !self.resource_ready(&dependency).await {
                tracing::warn!(
                    guest = %request.target.to_canonical_string(),
                    dependency = %dependency.to_canonical_string(),
                    "guest held on unready dependency",
                );
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The Provider row document of this Guest's `providerRef`, through the
    /// manager for the converted `Provider` type.
    async fn provider_document(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<Value, GuestEffectError> {
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| GuestEffectError::InvalidResource)?;
        if let Some(spec) = request.provider_spec.as_ref() {
            return Ok(json!({"spec": spec.clone()}));
        }
        Self::projected_row(self.resource_value(&provider_ref).await)?
            .ok_or(GuestEffectError::Unavailable)
    }

    /// Compose the full row document the old effect bodies read (they were
    /// handed the durable row JSON: `spec` beside `metadata`).
    fn row_document(request: &GuestEffectRequest<'_>) -> Value {
        json!({
            "spec": request.spec.clone(),
            "metadata": request.metadata.clone(),
        })
    }

    fn validate(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<Value, GuestEffectError> {
        if request.operation_id.is_empty() {
            return Err(GuestEffectError::InvalidResource);
        }
        if request.key.zone != self.zone.as_str()
            || request.controller_generation != self.controller_generation
            || request.key.type_name != crate::guest_driver::GUEST_TYPE_NAME
        {
            return Err(GuestEffectError::InvalidResource);
        }
        if request.spec.get("providerRef").and_then(Value::as_str) != Some(kind.provider_ref()) {
            return Err(GuestEffectError::InvalidResource);
        }
        Ok(Self::row_document(request))
    }

    fn guest_provider_resource(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<Value, GuestEffectError> {
        let value = self.validate(kind, request)?;
        if request.uid.as_str().is_empty() || request.generation.get() == 0 {
            return Err(GuestEffectError::InvalidResource);
        }
        Ok(value)
    }

    fn validate_qemu_guest(value: &Value) -> Result<(), GuestEffectError> {
        let settings = value
            .pointer("/spec/provider/settings")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        serde_json::from_value::<d2b_provider_runtime_qemu_media::GuestProviderSpecSettings>(
            settings,
        )
        .map(|_| ())
        .map_err(|_| GuestEffectError::InvalidResource)
    }

    fn validate_azure_vm_guest(value: &Value) -> Result<(), GuestEffectError> {
        let settings = Self::azure_vm_guest_settings_value(value)?;
        serde_json::from_value::<d2b_provider_runtime_azure_virtual_machine::AzureVmGuestSettings>(
            settings,
        )
        .map(|_| ())
        .map_err(|_| GuestEffectError::InvalidResource)
    }

    fn azure_vm_guest_settings_value(value: &Value) -> Result<Value, GuestEffectError> {
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
        Err(GuestEffectError::InvalidResource)
    }

    async fn validate_gateway_custody(
        &self,
        provider_ref: &ResourceRef,
        credential_fields: &[&str],
        request: &GuestEffectRequest<'_>,
    ) -> Result<(), GuestEffectError> {
        let provider = Self::projected_row(self.resource_value(provider_ref).await)?
            .ok_or(GuestEffectError::Unavailable)?;
        let config = provider
            .pointer("/spec/config")
            .ok_or(GuestEffectError::InvalidResource)?;
        let gateway = config
            .get("gatewayExecutionRef")
            .or_else(|| config.get("controllerExecutionRef"))
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .ok_or(GuestEffectError::InvalidResource)?;
        if gateway.resource_type().as_str() != "Guest" {
            return Err(GuestEffectError::InvalidResource);
        }
        let gateway_resource = Self::projected_row(self.resource_value(&gateway).await)?
            .ok_or(GuestEffectError::Unavailable)?;
        if gateway_resource.pointer("/metadata/zone").and_then(Value::as_str)
            != Some(self.zone.as_str())
            || gateway_resource.pointer("/status/phase").and_then(Value::as_str) != Some("Ready")
        {
            return Err(GuestEffectError::Unavailable);
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
                return Err(GuestEffectError::InvalidResource);
            }
            let credential = Self::projected_row(self.resource_value(&credential_ref).await)?
                .ok_or(GuestEffectError::Unavailable)?;
            // U10 owns token acquisition and delivery. The Guest leg consumes
            // only the stable, typed Credential scope contract at admission.
            let scope = credential
                .pointer("/spec/scope")
                .cloned()
                .ok_or(GuestEffectError::InvalidResource)?;
            let scope = serde_json::from_value::<
                d2b_contracts_provider::v3::credential::CredentialScope,
            >(scope)
            .map_err(|_| GuestEffectError::InvalidResource)?;
            if scope.execution_ref() != Some(&gateway) {
                return Err(GuestEffectError::InvalidResource);
            }
        }
        let _ = request;
        Ok(())
    }

    /// The KTD7 identity fence for one effect call.
    ///
    /// The old fence read the Provider row and the assignment fence from the
    /// pre-v3 store; on the converted plane the plane's registry is the
    /// authority for a Provider's committed identity (the composition seeds
    /// it from the durable authority), and the driver binds the controller
    /// generation every pass. A Provider the plane does not hold, or a pass
    /// that carried a different controller generation, refuses closed.
    async fn validate_guest_runtime_fence(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<(), GuestEffectError> {
        if request.controller_generation != self.controller_generation
            || request.key.zone != self.zone.as_str()
        {
            return Err(GuestEffectError::InvalidResource);
        }
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| GuestEffectError::InvalidResource)?;
        let plane = self.plane()?;
        {
            use crate::process_driver::CommittedProviderIdentitySource;
            let source = plane.registry().as_ref() as &dyn CommittedProviderIdentitySource;
            if source.committed_provider_identity(&provider_ref).is_none() {
                return Err(GuestEffectError::Unavailable);
            }
        }
        let runtime = self.runtime()?;
        runtime
            .controller_session_generation()
            .ok_or(GuestEffectError::Unavailable)?;
        Ok(())
    }

    fn framework_operation_id(prefix: &str, operation_id: &str) -> String {
        let digest = Sha256::digest(format!("{prefix}:{operation_id}").as_bytes());
        let mut id = String::with_capacity(24);
        id.push_str("guest-");
        id.push_str(prefix);
        for byte in digest.iter().take(8) {
            id.push_str(&format!("{byte:02x}"));
        }
        id
    }

    async fn qemu_dependencies(
        &self,
        request: &GuestEffectRequest<'_>,
        value: &Value,
        children: &[GuestChildObservation],
        effect: &FrameworkQemuEffect,
    ) -> Result<qemu_media_runtime::QemuMediaDependencies, GuestEffectError> {
        let mut ready_refs = Vec::new();
        for reference in declared_dependency_refs(&request.spec) {
            let phase = self.live_phase(&reference).await?;
            if phase == Some("Ready") {
                ready_refs.push(reference);
            }
        }
        let ready = |reference: &ResourceRef| ready_refs.contains(reference);
        let device_ref = value
            .pointer("/spec/deviceAttachments")
            .and_then(Value::as_array)
            .and_then(|attachments| attachments.first())
            .and_then(|attachment| attachment.get("deviceRef"))
            .and_then(Value::as_str)
            .map(ResourceRef::parse)
            .transpose()
            .map_err(|_| GuestEffectError::InvalidResource)?;
        let device = device_ref.as_ref().filter(|reference| ready(reference)).map(|reference| {
            qemu_media_runtime::DeviceObservation {
                device_ref: (*reference).clone(),
                phase: qemu_media_runtime::DevicePhase::Ready,
                owner_ref: request.owner_ref().ok(),
                platform: qemu_media_runtime::PlatformClass::X86_64Linux,
                authority_key: Sha256::digest(reference.to_canonical_string().as_bytes()).into(),
                process_identity: Some("qemu-media-runner".to_owned()),
                media_contract: "qemu-media/v1".to_owned(),
            }
        });
        let settings = serde_json::from_value::<qemu_media_runtime::GuestProviderSpecSettings>(
            value
                .pointer("/spec/provider/settings")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        )
        .map_err(|_| GuestEffectError::InvalidResource)?;
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
            .map_err(|_| GuestEffectError::InvalidResource)?
            .iter()
            .all(ready);
        let display_ref = if settings.display_window {
            declared_dependency_refs(&request.spec)
                .into_iter()
                .find(|reference| reference.resource_type().as_str() == "Endpoint")
        } else {
            None
        };
        let runtime_volume_ready = children.iter().any(|child| {
            child.key.type_name == "Volume"
                && child.key.name == format!("{}-runtime", request.target.name().as_str())
                && child.ready()
        });
        Ok(qemu_media_runtime::QemuMediaDependencies {
            device,
            network_ready,
            media_ready,
            display_ready: !settings.display_window
                || display_ref.as_ref().is_some_and(ready),
            qmp_ready: effect.qmp_ready(),
            qmp_status: effect.qmp_ready().then_some(qemu_media_runtime::QmpVmStatus::Paused),
            media_refs,
            display_ref,
            runtime_volume_ready,
            qmp_elapsed_seconds: 0,
        })
    }

    fn build_guest_controller(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
        value: &Value,
        provider: &Value,
    ) -> Result<GuestRuntimeController, GuestEffectError> {
        match kind {
            GuestKind::QemuMedia => {
                let config = serde_json::from_value::<qemu_media_runtime::ProviderConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(GuestEffectError::InvalidResource)?,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let settings = serde_json::from_value::<
                    qemu_media_runtime::GuestProviderSpecSettings,
                >(
                    value
                        .pointer("/spec/provider/settings")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let device_ref = value
                    .pointer("/spec/deviceAttachments")
                    .and_then(Value::as_array)
                    .and_then(|attachments| attachments.first())
                    .and_then(|attachment| attachment.get("deviceRef"))
                    .and_then(Value::as_str)
                    .map(ResourceRef::parse)
                    .transpose()
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                let network_refs = value
                    .pointer("/spec/networkAttachments")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|attachment| attachment.get("networkRef").and_then(Value::as_str))
                    .map(ResourceRef::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                let process = qemu_media_runtime::build_process_spec(
                    config.controller_execution_ref.clone(),
                    ResourceRef::parse(&format!("Volume/{}-runtime", request.target.name().as_str()))
                        .map_err(|_| GuestEffectError::InvalidResource)?,
                    device_ref,
                    network_refs,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let controller = qemu_media_runtime::QemuMediaController::new(
                    config,
                    settings,
                    process,
                    request.target.clone(),
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                Ok(GuestRuntimeController::Qemu {
                    controller,
                    effect: FrameworkQemuEffect::new(request.target.clone()),
                })
            }
            GuestKind::AzureContainerApps => {
                let config = serde_json::from_value::<aca_runtime::AcaProviderConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(GuestEffectError::InvalidResource)?,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let config_bytes =
                    serde_json::to_vec(&config).map_err(|_| GuestEffectError::InvalidResource)?;
                let provider_ref = ResourceRef::parse(kind.provider_ref())
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                let binding = aca_runtime::AcaResourceBinding {
                    guest_uid: request.uid.clone(),
                    provider_generation: self.provider_generation(&provider_ref)?.get(),
                    config_fingerprint: Sha256::digest(config_bytes).into(),
                };
                let control = Arc::new(FrameworkAcaControl {
                    state: Arc::new(tokio::sync::Mutex::new(FrameworkAcaState::new(
                        self.provider_generation(&provider_ref)?.get(),
                    ))),
                });
                let provider = aca_runtime::AzureContainerAppsRuntimeProvider::new(
                    config,
                    control,
                    Arc::new(FrameworkAcaLease),
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                Ok(GuestRuntimeController::Aca {
                    controller: provider.controller(binding),
                })
            }
            GuestKind::AzureVirtualMachine => {
                let config = serde_json::from_value::<azure_vm_runtime::AzureVmConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(GuestEffectError::InvalidResource)?,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let settings = serde_json::from_value::<azure_vm_runtime::AzureVmGuestSettings>(
                    Self::azure_vm_guest_settings_value(value)?,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
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
                .map_err(|_| GuestEffectError::InvalidResource)?
                .with_bootstrap_service(azure_vm_runtime::BootstrapService::from_state(
                    azure_vm_runtime::BootstrapServiceState::Enrolled,
                ));
                Ok(GuestRuntimeController::AzureVm { controller })
            }
            GuestKind::CloudHypervisor => Err(GuestEffectError::InvalidResource),
        }
    }

    /// The Provider generation the old shared Runner resolved from the
    /// Provider row (KTD7: the plane registry is the authority); never
    /// guessed.
    fn provider_generation(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<ResourceGeneration, GuestEffectError> {
        let plane = self.plane()?;
        use crate::process_driver::CommittedProviderIdentitySource;
        let source = plane.registry().as_ref() as &dyn CommittedProviderIdentitySource;
        source
            .committed_provider_identity(provider_ref)
            .map(|(_, generation)| generation)
            .ok_or(GuestEffectError::Unavailable)
    }

    fn controller_key(
        &self,
        request: &GuestEffectRequest<'_>,
        runtime: &ZoneResourceRuntime,
    ) -> Result<GuestControllerKey, GuestEffectError> {
        Ok((
            request.target.clone(),
            request.uid.clone(),
            self.request_provider_generation(request)?.get(),
            self.controller_generation.get(),
            request.generation.get(),
            runtime
                .controller_session_generation()
                .ok_or(GuestEffectError::Unavailable)?
                .get(),
        ))
    }

    fn request_provider_generation(
        &self,
        request: &GuestEffectRequest<'_>,
    ) -> Result<ResourceGeneration, GuestEffectError> {
        let provider_ref = request
            .spec
            .get("providerRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .ok_or(GuestEffectError::InvalidResource)?;
        self.provider_generation(&provider_ref)
    }

    async fn run_guest_controller(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
        value: &Value,
        provider: &Value,
    ) -> Result<GuestEffectPhase, GuestEffectError> {
        let runtime = {
            let _ = self.validate_guest_runtime_fence(kind, request).await?;
            self.runtime()?
        };
        let children = request.children.owned().await?;
        let key = self.controller_key(request, &runtime)?;
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            let controller = self.build_guest_controller(kind, request, value, provider)?;
            controllers.insert(key.clone(), controller);
        }
        let controller = controllers
            .get_mut(&key)
            .ok_or(GuestEffectError::Unavailable)?;
        match controller {
            GuestRuntimeController::Qemu { controller, effect } => {
                let deps = self.qemu_dependencies(request, value, &children, effect).await?;
                let outcome = controller
                    .reconcile(&deps, effect)
                    .map_err(|_| GuestEffectError::Unavailable)?;
                Ok(
                    if matches!(outcome, qemu_media_runtime::QemuMediaReconcileOutcome::Ready) {
                        GuestEffectPhase::Ready
                    } else {
                        GuestEffectPhase::Pending
                    },
                )
            }
            GuestRuntimeController::Aca { controller } => {
                let operation = aca_runtime::AcaOperationId::parse(Self::framework_operation_id(
                    "aca",
                    &request.operation_id,
                ))
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let outcome = controller
                    .reconcile(operation, 30_000)
                    .await
                    .map_err(|_| GuestEffectError::Unavailable)?;
                Ok(if outcome == aca_runtime::AcaReconcileOutcome::Converged {
                    GuestEffectPhase::Ready
                } else {
                    GuestEffectPhase::Pending
                })
            }
            GuestRuntimeController::AzureVm { controller } => {
                let outcome = controller
                    .reconcile(
                        self.zone.as_str(),
                        request.uid.as_str(),
                        request.generation.get(),
                    )
                    .await
                    .map_err(|_| GuestEffectError::Unavailable)?;
                Ok(if outcome == azure_vm_runtime::AzureVmReconcileOutcome::Converged {
                    GuestEffectPhase::Ready
                } else {
                    GuestEffectPhase::Pending
                })
            }
        }
    }

    async fn finalize_guest_controller(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<(bool, GuestControllerKey), GuestEffectError> {
        let value = self.guest_provider_resource(kind, request)?;
        let runtime = self.runtime()?;
        self.validate_guest_runtime_fence(kind, request).await?;
        let provider = self.provider_document(kind, request).await?;
        if matches!(
            kind,
            GuestKind::AzureContainerApps | GuestKind::AzureVirtualMachine
        ) {
            let provider_ref = ResourceRef::parse(kind.provider_ref())
                .map_err(|_| GuestEffectError::InvalidResource)?;
            self.validate_gateway_custody(
                &provider_ref,
                match kind {
                    GuestKind::AzureContainerApps => {
                        &["controlCredentialRef", "pullCredentialRef"][..]
                    }
                    GuestKind::AzureVirtualMachine => &["armCredentialRef"][..],
                    GuestKind::CloudHypervisor | GuestKind::QemuMedia => &[][..],
                },
                request,
            )
            .await?;
        }
        let key = self.controller_key(request, &runtime)?;
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            controllers.insert(
                key.clone(),
                self.build_guest_controller(kind, request, &value, &provider)?,
            );
        }
        let controller = controllers
            .get_mut(&key)
            .ok_or(GuestEffectError::Unavailable)?;
        match controller {
            GuestRuntimeController::Qemu { controller, effect } => {
                controller
                    .finalize(effect)
                    .map_err(|_| GuestEffectError::Unavailable)?;
            }
            GuestRuntimeController::Aca { controller } => {
                let operation = aca_runtime::AcaOperationId::parse(Self::framework_operation_id(
                    "aca-delete",
                    &request.operation_id,
                ))
                .map_err(|_| GuestEffectError::InvalidResource)?;
                controller
                    .finalize(operation, 30_000)
                    .await
                    .map_err(|_| GuestEffectError::Unavailable)?;
            }
            GuestRuntimeController::AzureVm { controller } => {
                if let Some(operation) = controller.recovery_state().operation {
                    controller
                        .poll_operation(operation)
                        .await
                        .map_err(|_| GuestEffectError::Unavailable)?;
                }
                controller
                    .finalize(
                        self.zone.as_str(),
                        request.uid.as_str(),
                        request.generation.get(),
                    )
                    .await
                    .map_err(|_| GuestEffectError::Unavailable)?;
            }
        }
        Ok((!controller.finalizer_installed(), key))
    }

    /// The Cloud Hypervisor Guest arm: drive the controller session and
    /// capture the layered status it publishes for the row.
    async fn reconcile_cloud_hypervisor_guest(
        &self,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError> {
        // U13: establish the Guest's ComponentSession and register its live
        // generation with the Zone target directory before the controller
        // session runs. The controller's own seeding then rides the session
        // the target layer already holds, and a reconnect re-adopts the
        // target-local realizations instead of inheriting them.
        if let Err(reason) =
            crate::ensure_guest_target_session(&self.state, &self.zone, &request.target).await
        {
            tracing::debug!(
                guest = %request.target.to_canonical_string(),
                reason = %reason,
                "Guest target session not established on this pass",
            );
        }
        let runtime = self.runtime()?;
        let outcome = runtime
            .reconcile_cloud_hypervisor_guest_with_status(
                Arc::clone(&self.state),
                &request.target,
                Some(Arc::clone(&request.status_sink)),
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    resource = %request.target.to_canonical_string(),
                    error = ?error,
                    "Cloud Hypervisor Guest effect failed",
                );
                GuestEffectError::Unavailable
            })?;
        let published = request
            .status_sink
            .lock()
            .clone()
            .or_else(|| request.status.clone());
        let phase = match published.as_ref().and_then(|status| status.get("phase")).and_then(Value::as_str) {
            Some("Ready") if outcome == crate::resource_runtime::CloudHypervisorReconcileOutcome::Ready => {
                GuestEffectPhase::Ready
            }
            _ => GuestEffectPhase::Pending,
        };
        Ok(GuestEffectOutcome {
            phase,
            resource_projection: published,
        })
    }
}

#[async_trait]
impl GuestDriverEffects for ProductionGuestDriverEffects {
    async fn reconcile(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError> {
        let value = self.guest_provider_resource(kind, request)?;
        if !self.dependencies_ready(request).await? {
            return Ok(GuestEffectOutcome::phase(GuestEffectPhase::Pending));
        }
        self.validate_guest_runtime_fence(kind, request).await?;
        let provider = self.provider_document(kind, request).await?;
        match kind {
            GuestKind::CloudHypervisor => self.reconcile_cloud_hypervisor_guest(request).await,
            GuestKind::QemuMedia => {
                Self::validate_qemu_guest(&value)?;
                let phase = self
                    .run_guest_controller(kind, request, &value, &provider)
                    .await?;
                Ok(GuestEffectOutcome::phase(phase))
            }
            GuestKind::AzureContainerApps => {
                let provider_ref = ResourceRef::parse(kind.provider_ref())
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                self.validate_gateway_custody(
                    &provider_ref,
                    &["controlCredentialRef", "pullCredentialRef"],
                    request,
                )
                .await?;
                let phase = self
                    .run_guest_controller(kind, request, &value, &provider)
                    .await?;
                Ok(GuestEffectOutcome::phase(phase))
            }
            GuestKind::AzureVirtualMachine => {
                Self::validate_azure_vm_guest(&value)?;
                let provider_ref = ResourceRef::parse(kind.provider_ref())
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                self.validate_gateway_custody(&provider_ref, &["armCredentialRef"], request)
                    .await?;
                let phase = self
                    .run_guest_controller(kind, request, &value, &provider)
                    .await?;
                Ok(GuestEffectOutcome::phase(phase))
            }
        }
    }

    async fn finalize(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError> {
        let _value = self.guest_provider_resource(kind, request)?;
        if kind == GuestKind::CloudHypervisor {
            // The controller session owns the Cloud Hypervisor teardown: its
            // finalize path runs inside the same reconcile entry the driver
            // calls, and the manager's deleting-row hold replaces the old
            // durable finalizer (F3).
            self.runtime()?
                .reconcile_cloud_hypervisor_guest_with_status(
                    Arc::clone(&self.state),
                    &request.target,
                    Some(Arc::clone(&request.status_sink)),
                )
                .await
                .map_err(|_| GuestEffectError::Unavailable)?;
            return Ok(GuestFinalizeStage::Complete);
        }
        let (complete, controller_key) = self.finalize_guest_controller(kind, request).await?;
        if !complete {
            return Ok(GuestFinalizeStage::Pending);
        }
        self.guest_controllers.lock().await.remove(&controller_key);
        Ok(GuestFinalizeStage::Complete)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};

    use super::{
        FrameworkAcaControl, FrameworkAcaLease, FrameworkAcaState, FrameworkAzureCredential,
        FrameworkAzureEffect, FrameworkAzureState, FrameworkQemuEffect, GuestRuntimeController,
        aca_runtime, azure_vm_runtime, qemu_media_runtime,
    };

    /// The qemu-media framework state machine drives its controller to
    /// `PausedAtBoot` and converges the finalizer through the effect port.
    #[test]
    fn qemu_controller_contract_invokes_controller_and_finalizes() {
        let guest_ref = ResourceRef::parse("Guest/qemu").unwrap();
        let config = qemu_media_runtime::ProviderConfig::new(
            "Host/host-system",
            "qemu-system-x86-64",
            "Provider/network-local",
            "Provider/volume-local",
            None,
        )
        .unwrap();
        let process = qemu_media_runtime::build_process_spec(
            config.controller_execution_ref.clone(),
            ResourceRef::parse("Volume/qemu-runtime").unwrap(),
            Some(ResourceRef::parse("Device/host-kvm").unwrap()),
            [],
        )
        .unwrap();
        let mut controller = qemu_media_runtime::QemuMediaController::new(
            config,
            qemu_media_runtime::GuestProviderSpecSettings::default(),
            process,
            guest_ref.clone(),
        )
        .unwrap();
        let mut effect = FrameworkQemuEffect::new(guest_ref.clone());
        let dependencies = qemu_media_runtime::QemuMediaDependencies::ready(
            qemu_media_runtime::DeviceObservation {
                device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
                phase: qemu_media_runtime::DevicePhase::Ready,
                owner_ref: None,
                platform: qemu_media_runtime::PlatformClass::X86_64Linux,
                authority_key: [1; 32],
                process_identity: Some("qemu-media-runner".to_owned()),
                media_contract: "qemu-media/v1".to_owned(),
            },
        );
        assert_eq!(
            controller.reconcile(&dependencies, &mut effect).unwrap(),
            qemu_media_runtime::QemuMediaReconcileOutcome::Ready
        );
        assert_eq!(
            controller.phase(),
            qemu_media_runtime::QemuMediaPhase::PausedAtBoot
        );
        controller.finalize(&mut effect).unwrap();
        assert!(!controller.finalizer_installed());
    }

    /// The ACA framework control/lease ports drive the controller through
    /// one progressing pass to `Ready` and converge its finalizer.
    #[tokio::test]
    async fn aca_controller_contract_invokes_controller_and_finalizes() {
        let profile = aca_runtime::AcaSandboxProfile::new(
            aca_runtime::AcaProfileId::parse("default").unwrap(),
            aca_runtime::AcaDiskImageSource::ConfiguredDisk {
                binding_id: aca_runtime::AcaConfiguredDiskId::parse("image-1").unwrap(),
            },
            aca_runtime::AcaCpuMillis::new(500).unwrap(),
            aca_runtime::AcaMemoryMib::new(2_048).unwrap(),
            300,
            None,
        )
        .unwrap();
        let defaults = aca_runtime::AcaRuntimeConfig::new(
            profile,
            aca_runtime::AcaReadinessPolicy::new(3, 10).unwrap(),
            1_000,
            4,
        )
        .unwrap();
        let config = aca_runtime::AcaProviderConfig::new(
            ResourceRef::parse("Guest/gateway").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("tenant").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("client").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("subscription").unwrap(),
            ResourceRef::parse("Credential/control").unwrap(),
            None,
            aca_runtime::AcaConfiguredImageId::parse("environment").unwrap(),
            aca_runtime::AcaConfiguredImageId::parse("resource-group").unwrap(),
            None,
            aca_runtime::AcaProfileId::parse("relay").unwrap(),
            defaults,
        )
        .unwrap();
        let controller = aca_runtime::AzureContainerAppsRuntimeProvider::new(
            config,
            Arc::new(FrameworkAcaControl {
                state: Arc::new(tokio::sync::Mutex::new(FrameworkAcaState::new(1))),
            }),
            Arc::new(FrameworkAcaLease),
        )
        .unwrap()
        .controller(aca_runtime::AcaResourceBinding {
            guest_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            provider_generation: 1,
            config_fingerprint: [2; 32],
        });
        let mut controller = GuestRuntimeController::Aca { controller };
        let GuestRuntimeController::Aca { controller } = &mut controller else {
            unreachable!();
        };
        let operation = aca_runtime::AcaOperationId::parse("u12-aca-test").unwrap();
        assert_eq!(
            controller.reconcile(operation.clone(), 30_000).await.unwrap(),
            aca_runtime::AcaReconcileOutcome::Progressing { after_ms: 10 }
        );
        assert_eq!(
            controller.reconcile(operation, 30_000).await.unwrap(),
            aca_runtime::AcaReconcileOutcome::Converged
        );
        assert_eq!(controller.phase(), aca_runtime::AcaPhase::Ready);
        controller
            .finalize(
                aca_runtime::AcaOperationId::parse("u12-aca-delete").unwrap(),
                30_000,
            )
            .await
            .unwrap();
        assert!(!controller.finalizer_installed());
    }

    /// The AzureVM framework effect/credential ports drive the controller to
    /// `Ready` and converge its finalizer through the bounded poll loop.
    #[tokio::test]
    async fn azure_vm_controller_contract_invokes_controller_and_finalizes() {
        let opaque = |value: &str| d2b_contracts::OpaqueAzureRef::parse(value).unwrap();
        let config = azure_vm_runtime::AzureVmConfig {
            tenant_id: None,
            client_id: None,
            arm_credential_ref: ResourceRef::parse("Credential/arm").unwrap(),
            controller_execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
            network_ref: None,
        };
        let settings = azure_vm_runtime::AzureVmGuestSettings {
            subscription_id: opaque("subscription"),
            resource_group: opaque("resource-group"),
            region: opaque("eastus"),
            vm_size: opaque("standard"),
            image_ref: opaque("image"),
            disk_sku: azure_vm_runtime::DiskSku::PremiumLrs,
            os_disk_size_gb: None,
            admin_user: "azureuser".to_owned(),
            vnet_subscription_id: None,
            vnet_resource_group: None,
            vnet_name: opaque("vnet"),
            subnet_name: opaque("subnet"),
            assign_public_ip: false,
            data_disks: Vec::new(),
            bootstrap_psk_delivery: azure_vm_runtime::BootstrapPskDelivery::VmExtension,
            bootstrap_deadline_ms: 60_000,
            child_zone_hosting: false,
            azure_tags: Vec::new(),
        };
        let effect = Arc::new(FrameworkAzureEffect {
            state: Arc::new(tokio::sync::Mutex::new(FrameworkAzureState::new(&settings))),
        });
        let mut controller = azure_vm_runtime::AzureVmController::new(
            config,
            settings,
            effect,
            Arc::new(FrameworkAzureCredential),
            None,
        )
        .unwrap()
        .with_bootstrap_service(azure_vm_runtime::BootstrapService::from_state(
            azure_vm_runtime::BootstrapServiceState::Enrolled,
        ));
        assert_eq!(
            controller
                .reconcile("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap(),
            azure_vm_runtime::AzureVmReconcileOutcome::Progressing { after_ms: 1_000 }
        );
        for _ in 0..2 {
            controller
                .reconcile("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap();
            if controller.phase() == azure_vm_runtime::AzureVmPhase::Ready {
                break;
            }
        }
        assert_eq!(controller.phase(), azure_vm_runtime::AzureVmPhase::Ready);
        for _ in 0..8 {
            if let Some(operation) = controller.recovery_state().operation {
                controller.poll_operation(operation).await.unwrap();
            }
            let outcome = controller
                .finalize("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap();
            let _ = outcome;
            if !controller.finalizer_installed() {
                break;
            }
        }
        assert!(!controller.finalizer_installed());
    }
}
