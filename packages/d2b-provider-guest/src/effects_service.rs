//! The provider-owned implementation of the Guest family's driver effects
//! (U10): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`GuestDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets). Its
//!   reconcile and finalize calls drive the Cloud Hypervisor controller
//!   session (the real host path) and the preserved framework state
//!   machines for the guest media runtime, azure-container-apps, and
//!   azure-virtual-machine Providers;
//! - the declared zone-plane service [`GUEST_EFFECTS_SERVICE`], hosted per
//!   zone by the daemon through [`GuestEffectsServiceFactory`]. Its one
//!   method (`guest-phase`) answers the live phase of one Guest resource
//!   from the zone's manager view - the same classified read the driver's
//!   effects gate on.
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
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the zone's manager view (live rows,
//! committed Provider identities, and the controller-session generation)
//! and the Cloud Hypervisor controller session. The framework state
//! machines are this crate's own in-memory controllers, so they read
//! nothing from the daemon. Nothing here names a daemon state type.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneId,
};
use d2b_resource_runtime::context::{LookupPlane, RowLookup};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::driver::{
    GuestChildObservation, GuestDriverEffects, GuestEffectError, GuestEffectOutcome,
    GuestEffectPhase, GuestEffectRequest, GuestFinalizeStage, GuestKind, GUEST_TYPE_NAME,
    declared_dependency_refs, view_phase,
};
use crate::facets::{
    CloudHypervisorGuestRuntime, GuestCloudHypervisorOutcome, GuestEffectFacets,
    GuestManagerView,
};
use d2b_provider_guest_azure_container_apps as aca_runtime;
use d2b_provider_guest_azure_virtual_machine as azure_vm_runtime;
use d2b_provider_guest_qemu_media as guest_media_runtime;

/// Framework-only QEMU effect evidence for non-Cloud-Hypervisor Guest owners.
///
/// The real Process/ComponentSession path remains owned by the selected
/// child Providers; this adapter exercises the typed lifecycle state machine
/// without claiming Cloud Hypervisor host liveness.
struct FrameworkQemuEffect {
    guest_ref: ResourceRef,
    identity: Option<guest_media_runtime::ProcessIdentity>,
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

impl guest_media_runtime::QemuMediaEffectPort for FrameworkQemuEffect {
    fn launch(
        &mut self,
        _ticket: &guest_media_runtime::LaunchTicket,
    ) -> Result<guest_media_runtime::ProcessIdentity, guest_media_runtime::QemuMediaError> {
        let template_digest: [u8; 32] = Sha256::digest(guest_media_runtime::PROCESS_TEMPLATE.as_bytes()).into();
        let identity_digest: [u8; 32] =
            Sha256::digest(self.guest_ref.to_canonical_string().as_bytes()).into();
        let identity = guest_media_runtime::ProcessIdentity {
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
    ) -> Result<Option<guest_media_runtime::ProcessIdentity>, guest_media_runtime::QemuMediaError> {
        Ok(self.identity.clone())
    }

    fn open_pidfd(
        &mut self,
        _identity: &guest_media_runtime::ProcessIdentity,
    ) -> Result<(), guest_media_runtime::QemuMediaError> {
        self.qmp_ready = true;
        Ok(())
    }

    fn reserve_device_authority(
        &mut self,
        _authority_key: [u8; 32],
        _owner_ref: &ResourceRef,
    ) -> Result<(), guest_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn close_media_effects(&mut self) -> Result<(), guest_media_runtime::QemuMediaError> {
        self.qmp_ready = false;
        Ok(())
    }

    fn continue_guest(&mut self) -> Result<(), guest_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn stop(
        &mut self,
        _identity: &guest_media_runtime::ProcessIdentity,
    ) -> Result<(), guest_media_runtime::QemuMediaError> {
        self.identity = None;
        self.qmp_ready = false;
        Ok(())
    }

    fn release_device_authority(&mut self) -> Result<(), guest_media_runtime::QemuMediaError> {
        Ok(())
    }

    fn delete_runtime_volume(&mut self) -> Result<(), guest_media_runtime::QemuMediaError> {
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
        aca_runtime::AcaSandboxCandidates::new(state.sandbox.iter().cloned().collect())
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
        aca_runtime::AcaDiskImageCandidates::new(state.disk_image.iter().cloned().collect())
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
        controller: Box<guest_media_runtime::QemuMediaController<FrameworkQemuEffect>>,
        effect: FrameworkQemuEffect,
    },
    Aca {
        controller: Box<aca_runtime::AcaController<FrameworkAcaControl, FrameworkAcaLease>>,
    },
    AzureVm {
        controller: Box<azure_vm_runtime::AzureVmController<FrameworkAzureEffect>>,
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

/// The provider-owned Guest effects (U10), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`GuestEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same runtime.
pub struct GuestEffectsService {
    /// The zone the plane serves (every effect fence binds it).
    zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    controller_generation: ControllerGeneration,
    /// The zone's manager view:live rows, committed Provider identities,
    /// and the controller-session generation.
    manager: Arc<dyn GuestManagerView>,
    /// The zone's Cloud Hypervisor controller session.
    cloud_hypervisor: Arc<dyn CloudHypervisorGuestRuntime>,
    /// Framework controllers (old `guest_controllers`): in-memory only, one
    /// slot per resource and generation set.
    guest_controllers: Arc<tokio::sync::Mutex<BTreeMap<GuestControllerKey, GuestRuntimeController>>>,
}

impl GuestEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):every
    /// daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: GuestEffectFacets) -> Self {
        Self {
            zone: facets.zone,
            controller_generation: facets.controller_generation,
            manager: facets.manager,
            cloud_hypervisor: facets.cloud_hypervisor,
            guest_controllers: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        }
    }

    /// The old-shape document of one resource (`spec`, `metadata`, live
    /// `status.phase`) from the manager view, answered as one classified read
    /// (issue #511): `Present` carries the document, `Absent` is the honest
    /// not-created answer, `Unavailable` is a plane that could not answer,
    /// and `Error` carries the projection detail of a committed row that
    /// cannot be read.
    async fn resource_value(&self, target: &ResourceRef) -> RowLookup<Value> {
        let key = ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        );
        let view = match self.manager.row_view(&key).await {
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
        let uid = match crate::driver::resource_uid(&view.uid) {
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
            || request.key.type_name != GUEST_TYPE_NAME
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

    fn validate_qemu_guest(
        value: &Value,
    ) -> Result<guest_media_runtime::GuestProviderSpecSettings, GuestEffectError> {
        let settings = value
            .pointer("/spec/provider/settings")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        serde_json::from_value(settings).map_err(|_| GuestEffectError::InvalidResource)
    }

    fn validate_azure_vm_guest(
        value: &Value,
    ) -> Result<azure_vm_runtime::AzureVmGuestSettings, GuestEffectError> {
        let settings = Self::azure_vm_guest_settings_value(value)?;
        serde_json::from_value(settings).map_err(|_| GuestEffectError::InvalidResource)
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
        if gateway_resource.pointer("/status/phase").and_then(Value::as_str) != Some("Ready")
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
        {
            if self
                .manager
                .committed_provider_identity(&provider_ref)
                .map_err(|_| GuestEffectError::Unavailable)?
                .is_none()
            {
                return Err(GuestEffectError::Unavailable);
            }
        }
        self.manager
            .controller_session_generation()
            .map_err(|_| GuestEffectError::Unavailable)?
            .ok_or(GuestEffectError::Unavailable)?;
        Ok(())
    }

    fn framework_operation_id(prefix: &str, operation_id: &str) -> String {
        use std::fmt::Write as _;
        let digest = Sha256::digest(format!("{prefix}:{operation_id}").as_bytes());
        let mut id = String::with_capacity(24);
        id.push_str("guest-");
        id.push_str(prefix);
        for byte in digest.iter().take(8) {
            let _ = write!(id, "{byte:02x}");
        }
        id
    }

    async fn qemu_dependencies(
        &self,
        request: &GuestEffectRequest<'_>,
        value: &Value,
        children: &[GuestChildObservation],
        effect: &FrameworkQemuEffect,
    ) -> Result<guest_media_runtime::QemuMediaDependencies, GuestEffectError> {
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
            guest_media_runtime::DeviceObservation {
                device_ref: (*reference).clone(),
                phase: guest_media_runtime::DevicePhase::Ready,
                owner_ref: request.owner_ref().ok(),
                platform: guest_media_runtime::PlatformClass::X86_64Linux,
                authority_key: Sha256::digest(reference.to_canonical_string().as_bytes()).into(),
                process_identity: Some(guest_media_runtime::PROCESS_TEMPLATE.to_owned()),
                media_contract: guest_media_runtime::MEDIA_CONTRACT_ID.to_owned(),
            }
        });
        let settings = serde_json::from_value::<guest_media_runtime::GuestProviderSpecSettings>(
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
                && child.key.name == guest_media_runtime::runtime_volume_name(request.target.name().as_str())
                && child.ready()
        });
        Ok(guest_media_runtime::QemuMediaDependencies {
            device,
            network_ready,
            media_ready,
            display_ready: !settings.display_window
                || display_ref.as_ref().is_some_and(ready),
            qmp_ready: effect.qmp_ready(),
            qmp_status: effect.qmp_ready().then_some(guest_media_runtime::QmpVmStatus::Paused),
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
        qemu_settings: Option<guest_media_runtime::GuestProviderSpecSettings>,
        azure_settings: Option<azure_vm_runtime::AzureVmGuestSettings>,
    ) -> Result<GuestRuntimeController, GuestEffectError> {
        match kind {
            GuestKind::QemuMedia => {
                let config = serde_json::from_value::<guest_media_runtime::ProviderConfig>(
                    provider
                        .pointer("/spec/config")
                        .cloned()
                        .ok_or(GuestEffectError::InvalidResource)?,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let settings = qemu_settings.ok_or(GuestEffectError::InvalidResource)?;
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
                let process = guest_media_runtime::build_process_spec(
                    config.controller_execution_ref.clone(),
                    ResourceRef::parse(&format!(
                        "Volume/{}",
                        guest_media_runtime::runtime_volume_name(request.target.name().as_str())
                    ))
                    .map_err(|_| GuestEffectError::InvalidResource)?,
                    device_ref,
                    network_refs,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                let controller = guest_media_runtime::QemuMediaController::new(
                    config,
                    settings,
                    process,
                    request.target.clone(),
                )
                .map_err(|_| GuestEffectError::InvalidResource)?;
                Ok(GuestRuntimeController::Qemu {
                    controller: Box::new(controller),
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
                    controller: Box::new(provider.controller(binding)),
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
                let settings = azure_settings.ok_or(GuestEffectError::InvalidResource)?;
                let state = Arc::new(tokio::sync::Mutex::new(FrameworkAzureState::new(&settings)));
                let controller = azure_vm_runtime::AzureVmController::new(
                    config,
                    settings,
                    FrameworkAzureEffect {
                        state: Arc::clone(&state),
                    },
                    Arc::new(FrameworkAzureCredential),
                    None,
                )
                .map_err(|_| GuestEffectError::InvalidResource)?
                .with_bootstrap_service(azure_vm_runtime::BootstrapService::from_state(
                    azure_vm_runtime::BootstrapServiceState::Enrolled,
                ));
                Ok(GuestRuntimeController::AzureVm {
                    controller: Box::new(controller),
                })
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
        self.manager
            .committed_provider_identity(provider_ref)
            .map_err(|_| GuestEffectError::Unavailable)?
            .map(|(_, generation)| generation)
            .ok_or(GuestEffectError::Unavailable)
    }

    fn controller_key(
        &self,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestControllerKey, GuestEffectError> {
        Ok((
            request.target.clone(),
            request.uid.clone(),
            self.request_provider_generation(request)?.get(),
            self.controller_generation.get(),
            request.generation.get(),
            self.manager
                .controller_session_generation()
                .map_err(|_| GuestEffectError::Unavailable)?
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
        qemu_settings: Option<guest_media_runtime::GuestProviderSpecSettings>,
        azure_settings: Option<azure_vm_runtime::AzureVmGuestSettings>,
    ) -> Result<GuestEffectPhase, GuestEffectError> {
        self.validate_guest_runtime_fence(kind, request).await?;
        let children = request.children.owned().await?;
        let key = self.controller_key(request)?;
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            let controller =
                self.build_guest_controller(kind, request, value, provider, qemu_settings, azure_settings)?;
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
                    if matches!(outcome, guest_media_runtime::QemuMediaReconcileOutcome::Ready) {
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
        let (qemu_settings, azure_settings) = match kind {
            GuestKind::QemuMedia => (Some(Self::validate_qemu_guest(&value)?), None),
            GuestKind::AzureVirtualMachine => (None, Some(Self::validate_azure_vm_guest(&value)?)),
            GuestKind::AzureContainerApps => (None, None),
            GuestKind::CloudHypervisor => return Err(GuestEffectError::InvalidResource),
        };
        let key = self.controller_key(request)?;
        let mut controllers = self.guest_controllers.lock().await;
        if !controllers.contains_key(&key) {
            controllers.insert(
                key.clone(),
                self.build_guest_controller(
                    kind, request, &value, &provider, qemu_settings, azure_settings,
                )?,
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
                if let Some(in_flight) = controller.recovery_state().in_flight_operation {
                    controller
                        .poll_operation(in_flight.operation)
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
        // target-local realizations instead of inheriting them. The session
        // establishment is a daemon-supplied facet (U10): a failure is
        // logged and the pass continues, exactly as the old effect did.
        if let Err(reason) = self
            .cloud_hypervisor
            .ensure_target_session(&request.target)
            .await
        {
            tracing::debug!(
                guest = %request.target.to_canonical_string(),
                reason = %reason,
                "Guest target session not established on this pass",
            );
        }
        let outcome = self
            .cloud_hypervisor
            .reconcile_guest(&request.target, Some(Arc::clone(&request.status_sink)))
            .await
            .map_err(|error| {
                tracing::warn!(
                    resource = %request.target.to_canonical_string(),
                    error = %error,
                    "Cloud Hypervisor Guest effect failed",
                );
                GuestEffectError::Unavailable
            })?;
        let published = request
            .status_sink
            .lock()
            .await
            .clone()
            .or_else(|| request.status.clone());
        let phase = match published.as_ref().and_then(|status| status.get("phase")).and_then(Value::as_str) {
            Some("Ready") if outcome == GuestCloudHypervisorOutcome::Ready => {
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
impl GuestDriverEffects for GuestEffectsService {
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
                let settings = Self::validate_qemu_guest(&value)?;
                let phase = self
                    .run_guest_controller(kind, request, &value, &provider, Some(settings), None)
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
                    .run_guest_controller(kind, request, &value, &provider, None, None)
                    .await?;
                Ok(GuestEffectOutcome::phase(phase))
            }
            GuestKind::AzureVirtualMachine => {
                let settings = Self::validate_azure_vm_guest(&value)?;
                let provider_ref = ResourceRef::parse(kind.provider_ref())
                    .map_err(|_| GuestEffectError::InvalidResource)?;
                self.validate_gateway_custody(&provider_ref, &["armCredentialRef"], request)
                    .await?;
                let phase = self
                    .run_guest_controller(kind, request, &value, &provider, None, Some(settings))
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
            // durable finalizer (F3). The session is a daemon-supplied
            // facet (U10).
            self.cloud_hypervisor
                .reconcile_guest(&request.target, Some(Arc::clone(&request.status_sink)))
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

/// The Guest family's declared effects service.
///
/// One zone-plane method, `guest-phase`: it answers the live phase of one
/// Guest resource from the zone's manager view - the same classified read
/// the driver's effects gate on. Payload:
///
/// ```json
/// { "zone": "<zone>", "zoneUid": null, "resourceRef": "Guest/<name>" }
/// ```
///
/// The invocation's zone is the authoritative one the host addressed: a
/// payload naming another zone refuses with its own closed code. The
/// zone-authority uid is host-supplied scope, not a caller assertion - the
/// capability object carries none today, so a payload that asserts one
/// refuses instead of being trusted.
///
/// Response: `{ "family": "guest", "resourceType": "Guest",
/// "guestRef": "<ref>", "phase": "Ready"|"Pending"|"Failed"|"Deleted"|"Absent" }`.
/// A row the manager holds answers its live phase; a row the manager
/// answers it does not hold answers `Absent`; a plane that cannot answer
/// refuses with its own closed code.
///
/// The service is declared on the `Guest` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const GUEST_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "guest.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("guest-phase")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `guest-phase` response payload: the family's live-phase report.
fn guest_phase_response(
    guest_ref: &ResourceRef,
    phase: &str,
) -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(json!({
        "family": "guest",
        "resourceType": GUEST_TYPE_NAME,
        "guestRef": guest_ref.to_canonical_string(),
        "phase": phase,
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: GUEST_EFFECTS_SERVICE.id.to_owned(),
        reason: "guest-phase-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// Serve the `guest-phase` method: answer the live phase of one Guest from
/// the zone's manager view. A plane that cannot answer refuses with its own
/// closed code instead of answering a half-built report.
async fn serve_guest_phase(
    manager: &dyn GuestManagerView,
    invocation_zone: &str,
    payload: &CanonicalJsonObject,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: GUEST_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let zone = match payload.get("zone") {
        Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value))
            if !value.is_empty() =>
        {
            value
        }
        _ => return Err(declined("guest-phase-zone-missing")),
    };
    // The invocation's zone is the authoritative one the host addressed; a
    // payload naming a different zone is refused instead of answered.
    if zone != invocation_zone {
        return Err(declined("guest-phase-zone-mismatch"));
    }
    // The zone-authority uid is host-supplied scope, never a caller
    // assertion: the capability object carries none today, so a payload
    // that asserts one is refused rather than trusted.
    match payload.get("zoneUid") {
        None | Some(d2b_contracts_resource::v3::CanonicalJsonValue::Null) => {}
        _ => return Err(declined("guest-phase-zone-uid-unsupplied")),
    }
    let resource_ref = match payload.get("resourceRef") {
        Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value))
            if !value.is_empty() =>
        {
            value
        }
        _ => return Err(declined("guest-phase-resource-ref-missing")),
    };
    let guest_ref =
        ResourceRef::parse(resource_ref).map_err(|_| declined("guest-phase-resource-ref-invalid"))?;
    if guest_ref.resource_type().as_str() != GUEST_TYPE_NAME {
        return Err(declined("guest-phase-not-a-guest"));
    }
    let key = ResourceKey::new(
        zone,
        guest_ref.resource_type().as_str(),
        guest_ref.name().as_str(),
    );
    let view = manager
        .row_view(&key)
        .await
        .map_err(|_| declined("guest-phase-manager-unavailable"))?;
    let Some(view) = view else {
        return guest_phase_response(&guest_ref, "Absent");
    };
    guest_phase_response(&guest_ref, view_phase(&view))
}

#[async_trait]
impl EffectService for GuestEffectsService {
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the zone's
        // manager view facet.
        serve_guest_phase(&*self.manager, invocation.zone, invocation.payload).await
    }
}

/// The composition-root factory that hosts the Guest effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set, and the host rebuilds the service from it on respawn.
pub struct GuestEffectsServiceFactory {
    facets: GuestEffectFacets,
}

impl GuestEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: GuestEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for GuestEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(GuestEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{
        CanonicalJsonObject, ControllerGeneration, ResourceGeneration,
        ResourceRef, ResourceUid, ZoneId,
    };
    use d2b_provider_toolkit::{EffectService, EffectServiceError};
    use d2b_resource_runtime::identity::ResourceKey;
    use d2b_resource_runtime::ResourceStatus;
    use serde_json::json;

    use super::{
        FrameworkAcaControl, FrameworkAcaLease, FrameworkAcaState, FrameworkAzureCredential,
        FrameworkAzureEffect, FrameworkAzureState, FrameworkQemuEffect, GUEST_EFFECTS_SERVICE,
        GuestRuntimeController, aca_runtime, azure_vm_runtime, guest_media_runtime,
    };
    use crate::driver::{
        GuestChildSurface, GuestDriverEffects, GuestEffectRequest, guest_status_sink,
    };
    use crate::test_support::{ScriptedFacets, row_fixture};

    /// One `guest-phase` service invocation over the canonical payload.
    fn invocation<'a>(
        payload: &'a CanonicalJsonObject,
        resources: &'a mut d2b_resource_runtime::context::ServiceResourceContext,
    ) -> d2b_provider_toolkit::ServiceInvocation<'a> {
        d2b_provider_toolkit::ServiceInvocation {
            zone: "work",
            method: GUEST_EFFECTS_SERVICE.methods[0].name,
            invocation_id: "invocation-guest-phase-test",
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: d2b_resource_types::MethodFdContract::NONE,
            payload_schema: None,
            chain_identities: &[],
        }
    }

    /// An inert child surface for effect requests that never read children.
    struct NoChildren;

    #[async_trait::async_trait]
    impl GuestChildSurface for NoChildren {
        async fn owned(&self) -> Result<Vec<crate::driver::GuestChildObservation>, crate::driver::GuestEffectError> {
            Ok(Vec::new())
        }
    }

    /// The guest media framework state machine drives its controller to
    /// `PausedAtBoot` and converges the finalizer through the effect port.
    #[test]
    fn qemu_controller_contract_invokes_controller_and_finalizes() {
        let guest_ref = ResourceRef::parse("Guest/qemu").unwrap();
        let config = guest_media_runtime::ProviderConfig::new(
            "Host/host-system",
            "qemu-system-x86-64",
            "Provider/network-local",
            "Provider/volume-local",
            None,
        )
        .unwrap();
        let process = guest_media_runtime::build_process_spec(
            config.controller_execution_ref.clone(),
            ResourceRef::parse("Volume/qemu-runtime").unwrap(),
            Some(ResourceRef::parse("Device/host-kvm").unwrap()),
            [],
        )
        .unwrap();
        let mut controller = guest_media_runtime::QemuMediaController::new(
            config,
            guest_media_runtime::GuestProviderSpecSettings::default(),
            process,
            guest_ref.clone(),
        )
        .unwrap();
        let mut effect = FrameworkQemuEffect::new(guest_ref.clone());
        let dependencies = guest_media_runtime::QemuMediaDependencies::ready(
            guest_media_runtime::DeviceObservation {
                device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
                phase: guest_media_runtime::DevicePhase::Ready,
                owner_ref: None,
                platform: guest_media_runtime::PlatformClass::X86_64Linux,
                authority_key: [1; 32],
                process_identity: Some(guest_media_runtime::PROCESS_TEMPLATE.to_owned()),
                media_contract: guest_media_runtime::MEDIA_CONTRACT_ID.to_owned(),
            },
        );
        assert_eq!(
            controller.reconcile(&dependencies, &mut effect).unwrap(),
            guest_media_runtime::QemuMediaReconcileOutcome::Ready
        );
        assert_eq!(
            controller.phase(),
            guest_media_runtime::QemuMediaPhase::PausedAtBoot
        );
        controller.finalize(&mut effect).unwrap();
        assert!(!controller.finalizer_installed());
    }

    /// The ACA framework control/lease ports drive the controller through
    /// one progressing pass to `Ready` and converge its finalizer.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
            d2b_contracts_provider::v3::credential::OpaqueAzureRef::parse("tenant").unwrap(),
            d2b_contracts_provider::v3::credential::OpaqueAzureRef::parse("client").unwrap(),
            d2b_contracts_provider::v3::credential::OpaqueAzureRef::parse("subscription").unwrap(),
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
        let mut controller = GuestRuntimeController::Aca {
            controller: Box::new(controller),
        };
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
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn azure_vm_controller_contract_invokes_controller_and_finalizes() {
        let opaque = |value: &str| {
            d2b_contracts_provider::v3::credential::OpaqueAzureRef::parse(value).unwrap()
        };
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
        let effect = FrameworkAzureEffect {
            state: Arc::new(tokio::sync::Mutex::new(FrameworkAzureState::new(&settings))),
        };
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
            if let Some(in_flight) = controller.recovery_state().in_flight_operation {
                controller.poll_operation(in_flight.operation).await.unwrap();
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

    // -- `guest-phase` admission (U10) ---------------------------------------

    /// The canonical payload every admission test starts from.
    fn guest_phase_payload() -> CanonicalJsonObject {
        serde_json::from_value(json!({
            "zone": "work",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Guest/worker",
        }))
        .expect("canonical payload")
    }

    /// Run one `guest-phase` invocation over the scripted facets and return
    /// the service's refusal, asserting it refused.
    async fn guest_phase_refusal(
        facets: &Arc<ScriptedFacets>,
        payload: CanonicalJsonObject,
    ) -> EffectServiceError {
        let service = super::GuestEffectsService::new(facets.facet_set());
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        service
            .handle(invocation(&payload, &mut resources))
            .await
            .expect_err("the admission gate refuses")
    }

    /// A payload that names a different zone than the invocation's
    /// authoritative one is refused with its own closed code, never
    /// answered from the wrong zone's view.
    #[tokio::test]
    async fn guest_phase_refuses_a_payload_that_names_another_zone() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zone": "other",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Guest/worker",
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-zone-mismatch".to_owned(),
            },
        );
    }

    /// A payload that carries no zone at all is refused with its own closed
    /// code.
    #[tokio::test]
    async fn guest_phase_refuses_a_payload_without_zone() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Guest/worker",
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-zone-missing".to_owned(),
            },
        );
    }

    /// The zone-authority uid is host-supplied scope: a payload that asserts
    /// one is refused instead of trusted.
    #[tokio::test]
    async fn guest_phase_refuses_a_payload_that_asserts_a_zone_uid() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zone": "work",
            "zoneUid": "123e4567-e89b-42d3-a456-426614174000",
            "resourceRef": "Guest/worker",
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-zone-uid-unsupplied".to_owned(),
            },
        );
    }

    /// A payload that names no resource is refused with its own closed code.
    #[tokio::test]
    async fn guest_phase_refuses_a_payload_without_a_resource_ref() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zone": "work",
            "zoneUid": serde_json::Value::Null,
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-resource-ref-missing".to_owned(),
            },
        );
    }

    /// A resource reference that cannot parse is refused with its own closed
    /// code.
    #[tokio::test]
    async fn guest_phase_refuses_an_invalid_resource_ref() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zone": "work",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "not-a-reference",
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-resource-ref-invalid".to_owned(),
            },
        );
    }

    /// The service answers Guest rows only: a payload naming another
    /// resource type is refused with its own closed code.
    #[tokio::test]
    async fn guest_phase_refuses_a_non_guest_resource() {
        let facets = ScriptedFacets::new();
        let payload = serde_json::from_value(json!({
            "zone": "work",
            "zoneUid": serde_json::Value::Null,
            "resourceRef": "Process/worker",
        }))
        .expect("canonical payload");
        let error = guest_phase_refusal(&facets, payload).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-not-a-guest".to_owned(),
            },
        );
    }

    /// A manager view that cannot answer refuses with its own closed code
    /// instead of answering a half-built report.
    #[tokio::test]
    async fn guest_phase_refuses_when_the_manager_cannot_answer() {
        let facets = ScriptedFacets::new();
        facets.set_fail_reads(true);
        let error = guest_phase_refusal(&facets, guest_phase_payload()).await;
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: GUEST_EFFECTS_SERVICE.id.to_owned(),
                reason: "guest-phase-manager-unavailable".to_owned(),
            },
        );
    }

    /// A row the manager holds answers its live phase - the same
    /// manager-view read the driver's effects gate on.
    #[tokio::test]
    async fn guest_phase_answers_the_live_phase_of_a_held_row() {
        let facets = ScriptedFacets::new();
        facets
            .add_row(row_fixture(
                "work",
                "Guest",
                "worker",
                json!({ "providerRef": "Provider/runtime-qemu-media" }),
                ResourceStatus::Ready,
            ))
            .await;
        let service = super::GuestEffectsService::new(facets.facet_set());
        let payload = guest_phase_payload();
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources))
            .await
            .expect("the held row answers");
        assert_eq!(
            response.payload,
            serde_json::from_value::<CanonicalJsonObject>(json!({
                "family": "guest",
                "resourceType": "Guest",
                "guestRef": "Guest/worker",
                "phase": "Ready",
            }))
            .expect("canonical payload"),
        );
    }

    // -- Cloud Hypervisor round through the effects service (U10) ------------

    /// One Cloud Hypervisor reconcile request: the spec selects the Cloud
    /// Hypervisor provider, the manager holds that Provider row Ready, the
    /// plane registry holds its committed identity, and a controller session
    /// is enrolled - the full fence the old effect ran.
    fn cloud_hypervisor_request() -> GuestEffectRequest<'static> {
        GuestEffectRequest {
            zone: ZoneId::parse("work").expect("zone"),
            target: ResourceRef::parse("Guest/acceptance-guest").expect("guest"),
            key: ResourceKey::new("work", "Guest", "acceptance-guest"),
            uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid"),
            generation: ResourceGeneration::new(1).expect("generation"),
            controller_generation: ControllerGeneration::new(3).expect("generation"),
            operation_id: "guest-phase-round".to_owned(),
            spec: json!({ "providerRef": "Provider/runtime-cloud-hypervisor" }),
            metadata: json!({}),
            provider_spec: None,
            status: None,
            children: &NoChildren,
            status_sink: guest_status_sink(),
        }
    }

    /// The Cloud Hypervisor arm drives the daemon-supplied controller
    /// session facets through the real effects service: the target session
    /// is established, the reconcile runs over the scripted outcome, and the
    /// Provider's published status is captured for the row. The KTD7 fence
    /// reads the committed identity and the enrolled session generation
    /// first.
    #[tokio::test]
    async fn cloud_hypervisor_reconcile_drives_the_controller_session_facets() {
        let facets = ScriptedFacets::new();
        facets
            .add_row(row_fixture(
                "work",
                "Provider",
                "runtime-cloud-hypervisor",
                json!({ "config": {} }),
                ResourceStatus::Ready,
            ))
            .await;
        facets.add_committed_provider(
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("provider"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").expect("uid"),
            ResourceGeneration::new(4).expect("generation"),
        );
        facets.set_session_generation(Some(
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(2).expect("generation"),
        ));
        facets
            .set_cloud_hypervisor_outcome(crate::facets::GuestCloudHypervisorOutcome::Ready)
            .await;
        let service = super::GuestEffectsService::new(facets.facet_set());
        let request = cloud_hypervisor_request();
        // The controller session's status write is captured into the sink
        // before the pass, exactly as the driver's effect call observes it.
        *request.status_sink.lock().await = Some(json!({ "phase": "Ready" }));

        let outcome = service
            .reconcile(crate::driver::GuestKind::CloudHypervisor, &request)
            .await
            .expect("the Cloud Hypervisor arm reconciles");
        assert_eq!(outcome.phase, crate::driver::GuestEffectPhase::Ready);
        assert_eq!(
            outcome.resource_projection,
            Some(json!({ "phase": "Ready" })),
            "the controller's status write is the row's projection",
        );
        assert_eq!(
            facets.call_order(),
            vec![
                "row:work/Provider/runtime-cloud-hypervisor".to_owned(),
                "committed:Provider/runtime-cloud-hypervisor".to_owned(),
                "session-generation".to_owned(),
                "row:work/Provider/runtime-cloud-hypervisor".to_owned(),
                "ensure-session:Guest/acceptance-guest".to_owned(),
                "reconcile-ch:Guest/acceptance-guest".to_owned(),
            ],
            "the dependency barrier and fence reads precede the provider document, the session establishment, and the reconcile",
        );
    }

    /// The Cloud Hypervisor finalize completes through the controller
    /// session facet: the same reconcile entry the driver calls, with the
    /// manager's deleting-row hold replacing the old durable finalizer.
    #[tokio::test]
    async fn cloud_hypervisor_finalize_completes_through_the_controller_session() {
        let facets = ScriptedFacets::new();
        facets
            .add_row(row_fixture(
                "work",
                "Provider",
                "runtime-cloud-hypervisor",
                json!({ "config": {} }),
                ResourceStatus::Ready,
            ))
            .await;
        facets.add_committed_provider(
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("provider"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").expect("uid"),
            ResourceGeneration::new(4).expect("generation"),
        );
        facets.set_session_generation(Some(
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(2).expect("generation"),
        ));
        let service = super::GuestEffectsService::new(facets.facet_set());
        let request = cloud_hypervisor_request();

        let stage = service
            .finalize(crate::driver::GuestKind::CloudHypervisor, &request)
            .await
            .expect("the Cloud Hypervisor finalize completes");
        assert_eq!(stage, crate::driver::GuestFinalizeStage::Complete);
        assert_eq!(
            facets.call_order(),
            vec!["reconcile-ch:Guest/acceptance-guest".to_owned()],
            "the Cloud Hypervisor finalize rides the same controller-session reconcile entry alone",
        );
    }
}
