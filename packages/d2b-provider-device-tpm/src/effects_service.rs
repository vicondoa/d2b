//! The provider-owned implementation of the TPM family's resource effect
//! port (U12 tpm step): the family serves its effects over the
//! daemon-supplied facets instead of a daemon-built port.
//!
//! The port is the [`TpmResourceEffectPort`] the Device controller calls:
//! every effect is a manager child mutation or a manager view of a
//! Device-owned row, plus the one-time legacy state adoption and the
//! broker-owned state-directory preparation, neither of which launches a
//! process. The daemon state the port reaches - the trusted bundle the
//! state-directory intent is resolved from, the authenticated origination
//! socket and caller authority one kernel invocation goes over, and the
//! owning Guest's lifecycle admission - crosses the provider boundary as
//! declared facets ([`crate::facets`]).
//!
//! The declared zone-plane service [`TPM_EFFECTS_SERVICE`] is hosted per
//! zone by the daemon through [`TpmEffectsServiceFactory`]; its one method
//! (`inspect-tpm`) answers the family's committed surface: the declared
//! Device-owned rows and the kernel the state-directory preparation
//! invokes.

use std::path::PathBuf;
use std::sync::Arc;

use d2b_contracts::types::{BundleOpId, VmId};
use d2b_contracts_broker::kernel_client::{KernelInvocation, envelope_invoke_kernel};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::storage::StoragePathSpec;
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
    SharedProviderChildSurface,
};
use d2b_resource_runtime::context::ChildEnsure;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::ResourceStatus;
use d2b_resource_types::{ServiceDecl, ServiceMethod};
use serde_json::Value;

use crate::facets::TpmEffectFacets;
use crate::resource_controller::{
    TpmResourceController, TpmResourceControllerError, TpmResourceOutcome,
};
use crate::resource_effect::{TpmResourceEffectError, TpmResourceEffectPort};

/// The TPM family's declared effects service.
///
/// One zone-plane method, `inspect-tpm`: it answers the family's committed
/// surface - the Device-owned rows the controller's effects ensure and
/// read, and the kernel the state-directory preparation invokes. The report
/// is hermetic: no host state is read or mutated.
pub const TPM_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "tpm.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-tpm")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-tpm` response payload: the family's committed surface.
/// The payload is built through the canonical JSON object path, so a
/// structural character in a trusted value yields a correctly escaped
/// report rather than an unparseable one; the refusal is unreachable and
/// names its own code.
fn inspect_tpm_response() -> Result<EffectResponse, EffectServiceError> {
    let payload = serde_json::from_value(serde_json::json!({
        "family": "device-tpm",
        "provider": crate::PROVIDER_REF,
        "resourceType": "Device",
        "rows": [
            crate::vocabulary::TPM_PROCESS_ROW_PREFIX,
            crate::vocabulary::TPM_FLUSH_ROW_PREFIX,
            crate::vocabulary::TPM_ENDPOINT_ROW_PREFIX,
        ],
        "operation": "prepare-directory",
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: TPM_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-tpm-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The hosted `inspect-tpm` service: answers the family's committed surface
/// report. The report is static (the family's own vocabulary), so the
/// service holds no runtime state.
struct TpmEffectsService;

#[async_trait::async_trait]
impl EffectService for TpmEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        inspect_tpm_response()
    }
}

/// The composition-root factory that hosts the TPM effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set for the respawn path, which is not yet wired.
pub struct TpmEffectsServiceFactory {
    // The facet set is carried for the R5 respawn contract: the daemon
    // registers one factory per zone with that zone's facet set. The
    // respawn path that rebuilds the service from the facets is not wired
    // yet, and the current static inspect service does not read them.
    #[allow(dead_code)]
    facets: TpmEffectFacets,
}

impl TpmEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: TpmEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for TpmEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(TpmEffectsService)
    }
}

/// The canonical wire phase of one manager view (the daemon's shared
/// `view_phase` helper, moved with the port): the published status
/// classification, or `Pending` for a row without one.
fn view_phase(view: &ResourceView) -> &'static str {
    view.observed_status()
        .as_ref()
        .map(ResourceStatus::wire_phase)
        .unwrap_or("Pending")
}

/// Fail closed (retryable) while a declared row is absent or still
/// converging, and terminally when its controller reported `Failed`.
fn gate_declared_phase(phase: Option<&'static str>) -> Result<(), TpmResourceEffectError> {
    match phase {
        Some("Ready") => Ok(()),
        Some("Failed") => Err(TpmResourceEffectError::EffectRejected),
        _ => Err(TpmResourceEffectError::Transient),
    }
}

/// Gate the pre-start flush row on its typed one-shot outcome.
///
/// The row's published phase cannot carry this: a driver pass that concluded
/// publishes the runtime's ready classification whatever the one-shot
/// outcome was, so the outcome - and only the outcome - rides the row's
/// status projection (`{"ephemeral": {"state": ..., "code": ...}}`,
/// `ProcessDriver::publish_ephemeral_outcome` in `d2b-provider-process`). A
/// row that has not published one yet is retryable (the flush is still in
/// flight); a failed outcome is the flush's own failure and fails the device
/// path closed; an unreadable projection is refused rather than read as
/// success.
fn gate_flush_outcome(view: &ResourceView) -> Result<(), TpmResourceEffectError> {
    let Some(projection) = view.observed_status_projection() else {
        return Err(TpmResourceEffectError::Transient);
    };
    match projection.pointer("/ephemeral/state").and_then(Value::as_str) {
        Some("succeeded") => Ok(()),
        Some("failed") => Err(TpmResourceEffectError::EffectRejected),
        _ => Err(TpmResourceEffectError::StateIntegrity),
    }
}

/// One Provider-authored resource document as a manager child ensure.
fn declared_child(document: Value) -> Result<(ResourceRef, ChildEnsure), TpmResourceEffectError> {
    let type_name = document
        .get("type")
        .and_then(Value::as_str)
        .ok_or(TpmResourceEffectError::EffectRejected)?;
    let name = document
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .ok_or(TpmResourceEffectError::EffectRejected)?;
    let reference = ResourceRef::parse(&format!("{type_name}/{name}"))
        .map_err(|_| TpmResourceEffectError::InvalidDevice)?;
    let spec = document
        .get("spec")
        .ok_or(TpmResourceEffectError::EffectRejected)?;
    Ok((
        reference,
        ChildEnsure {
            type_name: ResourceTypeName::new(type_name),
            name: name.to_owned(),
            spec: serde_json::to_vec(spec).map_err(|_| TpmResourceEffectError::EffectRejected)?,
            metadata: Vec::new(),
        },
    ))
}

/// The Device-owned TPM rows, resolved through the manager child surface.
///
/// The child surface is the driver's context surface, so every row it names
/// is a child of the requiring Device row: the manager derives the child's
/// uid and owner from the parent, and the declared launch identity is the
/// ChildEnsure `(type, name)` pair.
struct DeclaredTpmRows<'a> {
    children: &'a dyn SharedProviderChildSurface,
    zone: String,
    device_uid: ResourceUid,
    device_ref: ResourceRef,
    execution_ref: ResourceRef,
}

impl DeclaredTpmRows<'_> {
    /// The manager key of one declared row in this Zone.
    fn key(&self, reference: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            reference.resource_type().as_str(),
            reference.name().as_str(),
        )
    }

    /// The declared long-lived swtpm row (`Process/swtpm-<device>`).
    fn process_ref(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        ResourceRef::parse(&format!(
            "{}{}",
            crate::vocabulary::TPM_PROCESS_ROW_PREFIX,
            self.device_ref.name().as_str()
        ))
        .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The declared pre-start flush row
    /// (`EphemeralProcess/swtpm-flush-<device>`).
    fn flush_ref(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        ResourceRef::parse(&format!(
            "{}{}",
            crate::vocabulary::TPM_FLUSH_ROW_PREFIX,
            self.device_ref.name().as_str()
        ))
        .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The declared TPM Endpoint (`Endpoint/tpm-<device>`).
    fn endpoint_ref(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        ResourceRef::parse(&format!(
            "{}{}",
            crate::vocabulary::TPM_ENDPOINT_ROW_PREFIX,
            self.device_ref.name().as_str()
        ))
        .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The controller-owned state Volume document and reference
    /// (`Volume/device-<32hex>-tpm-state`).
    fn state_volume(
        &self,
    ) -> Result<(ResourceRef, ChildEnsure), TpmResourceEffectError> {
        let document = crate::build_tpm_state_volume_resource(
            &self.device_uid,
            &self.device_ref,
            &self.zone,
            &self.execution_ref,
        )?;
        declared_child(document)
    }

    /// The live view of one declared row, refusing a row that is not owned by
    /// the requiring Device (the owner fence the durable adoption
    /// classification used to carry): a row under another owner's uid is
    /// never read as this Device's evidence.
    async fn view(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<ResourceView>, TpmResourceEffectError> {
        let view = self
            .children
            .view(&self.key(reference))
            .await
            .map_err(|_| TpmResourceEffectError::Transient)?;
        if let Some(view) = view.as_ref() {
            let owner = view
                .owner_key
                .as_ref()
                .ok_or(TpmResourceEffectError::StateIntegrity)?;
            if owner != &self.key(&self.device_ref.clone()) {
                return Err(TpmResourceEffectError::StateIntegrity);
            }
        }
        Ok(view)
    }

    /// The published phase of one declared row.
    async fn phase(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<&'static str>, TpmResourceEffectError> {
        Ok(self
            .view(reference)
            .await?
            .as_ref()
            .map(view_phase))
    }

    /// Wait for one declared row's controller to reach Ready.
    async fn wait_ready(&self, reference: &ResourceRef) -> Result<(), TpmResourceEffectError> {
        gate_declared_phase(self.phase(reference).await?)
    }

    /// Retire one declared row through the manager (idempotent).
    async fn delete(&self, reference: &ResourceRef) -> Result<(), TpmResourceEffectError> {
        self.children
            .delete(&self.key(reference))
            .await
            .map_err(|_| TpmResourceEffectError::Transient)
    }

    /// Ensure the controller-owned state Volume row and wait for its
    /// controller.
    async fn ensure_state_volume(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        let (reference, child) = self.state_volume()?;
        self.children
            .ensure(child)
            .await
            .map_err(|error| {
                tracing::warn!(
                    device = %self.device_ref.to_canonical_string(),
                    error = ?error,
                    "tpm state volume child ensure failed"
                );
                TpmResourceEffectError::Transient
            })?;
        self.wait_ready(&reference).await?;
        Ok(reference)
    }
}

/// Concrete provider-side TPM resource effect port (U12 tpm step): the
/// retired daemon adapter moved into the declaring crate, with the daemon
/// state it reached supplied through the declared facets.
///
/// The port holds no broker spawn surface by construction: every effect is a
/// manager child mutation or a manager view of a Device-owned row. The two
/// broker calls that remain are the one-time legacy state adoption and the
/// broker-owned state-directory preparation, neither of which launches a
/// process.
pub struct LiveTpmResourceEffectPort<'a> {
    facets: TpmEffectFacets,
    vm_id: VmId,
    /// The Zone the Device row lives in: every manager/broker surface this
    /// port touches (declared children, the prepare-directory kernel) is
    /// anchored here, not on the Device's Guest target - a nested Guest VM
    /// is never registered with the host daemon's zone coordinator.
    zone: String,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    rows: DeclaredTpmRows<'a>,
    device_uid: ResourceUid,
    device_ref: ResourceRef,
    execution_ref: ResourceRef,
    /// The operation id the owning Guest's lifecycle admission is resolved
    /// for (the retired adapter's `Internal` admission source).
    operation_id: String,
    /// The guest lifecycle lease is consumed at most once, by the first
    /// effect that reaches a launchable row (the preserved
    /// `lifecycle_lease_consumed` gate of the old executor).
    lifecycle_lease_consumed: tokio::sync::Mutex<bool>,
}

impl LiveTpmResourceEffectPort<'_> {
    fn identity_matches(
        &self,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<(), TpmResourceEffectError> {
        if device_uid != &self.device_uid
            || device_ref != &self.device_ref
            || execution_ref != &self.execution_ref
        {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        Ok(())
    }

    /// Consume the Core-issued guest lifecycle lease exactly once. The lease
    /// authorized this Device's start operation; the row's Process controller
    /// owns the process from here, so the port only retires the admission.
    async fn consume_lifecycle_lease(&self) -> Result<(), TpmResourceEffectError> {
        let mut consumed = self
            .lifecycle_lease_consumed
            .try_lock()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        if *consumed {
            return Ok(());
        }
        self.facets
            .runtime
            .consume_lifecycle_lease(self.vm_id.as_str(), &self.operation_id)
            .await?;
        *consumed = true;
        Ok(())
    }

    /// The broker-owned state-directory preparation (the trusted marker and
    /// hardening step the legacy TPM connector supplies). Not a spawn.
    ///
    /// The retired typed `PrepareStateDir` arm resolved the subject's trusted
    /// state directory from the bundle; the U10 leg resolves the same intent
    /// from the daemon's bundle copy and invokes the prepare-directory kernel
    /// with the resolved parameters. A v3 zone-native Guest carries no
    /// legacy per-VM state-directory intent, so the retired arm's zone-native
    /// fallback is preserved: its TPM state directory is the
    /// controller-created state Volume the trusted `path:swtpm-state:<guest>`
    /// storage row roots - the same row the worker derivation, the
    /// spawn-time swtpm-dir fence and the volume-local controller's root all
    /// agree on.
    async fn prepare_state_dir(&self) -> Result<(), TpmResourceEffectError> {
        let resolver = self
            .facets
            .runtime
            .load_bundle()
            .await
            .map_err(|error| {
                tracing::warn!(error = ?error, "tpm prepare: bundle resolver load failed");
                TpmResourceEffectError::Transient
            })?;
        let (base_dir, owner_uid, owner_gid, mode) = match resolver
            .resolve_prepare_dir_intent(self.vm_id.as_str(), false)
        {
            Some(intent) => (
                intent.base_dir,
                intent.owner_uid,
                intent.owner_gid,
                intent.mode,
            ),
            None => {
                // Zone-native posture (retired-arm parity): no legacy
                // state-directory intent for this Guest. Prepare the state
                // root the trusted storage row names. Narrowing that root to
                // the unique TPM Device's state Volume directory is the
                // broker's job - its `PrepareStateDir` resolves it from the
                // same trusted artifacts - so this shared daemon never
                // assembles a per-Device directory name itself.
                let (spec, state_root) =
                    zone_native_swtpm_state_row(&resolver, self.vm_id.as_str())
                        .ok_or(TpmResourceEffectError::StateIntegrity)?;
                let (owner_uid, owner_gid, mode) = row_posture(spec)
                    .ok_or(TpmResourceEffectError::StateIntegrity)?;
                (state_root, owner_uid, owner_gid, mode)
            }
        };
        // The Device row's own Zone - never a zone-authority lookup of the
        // Guest target VM, which the host daemon's coordinator does not
        // register (the guest's plane lives inside the nested VM).
        let zone = self.zone.clone();
        let invocation = KernelInvocation {
            operation: "prepare-directory",
            zone: zone.as_str(),
            payload: serde_json::json!({
                "kind": "state",
                "baseDir": base_dir.display().to_string(),
                "vmIdOrScope": self.vm_id.as_str(),
                "mode": mode,
                "ownerUid": owner_uid,
                "ownerGid": owner_gid,
                "createdPaths": [],
            }),
            fds: &[],
            chain_root_invocation_id: None,
            chain_identities: None,
        };
        match envelope_invoke_kernel(
            self.facets.runtime.broker_socket_path(),
            self.facets.runtime.kernel_io_timeout(),
            self.facets.runtime.caller_role(),
            invocation,
        ) {
            Ok(_) => Ok(()),
            Err(error) => {
                tracing::warn!(
                    device = %self.device_ref.to_canonical_string(),
                    error = %error,
                    "broker state-directory preparation refused",
                );
                Err(TpmResourceEffectError::StateIntegrity)
            }
        }
    }
}

/// The trusted `path:swtpm-state:<guest>` storage row of one zone-native
/// Guest: the row plus the absolute state root it names. `None` for a
/// subject no trusted artifact names - the caller keeps failing closed
/// rather than inventing a directory (retired-arm parity).
fn zone_native_swtpm_state_row<'a>(
    resolver: &'a BundleResolver,
    guest: &str,
) -> Option<(&'a StoragePathSpec, PathBuf)> {
    let spec = resolver.find_storage_path_spec(&format!(
        "{}{guest}",
        crate::vocabulary::TPM_STATE_STORAGE_ROW_PREFIX
    ))?;
    let path = PathBuf::from(spec.path_template.as_str());
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some((spec, path))
}

/// The numeric posture one trusted storage row declares for its path -
/// `(owner_uid, owner_gid, mode)` - resolved against this host. `None` when
/// the row's principals or mode cannot be resolved here (retired-arm parity:
/// a caller that must record a posture fails closed instead of inventing
/// one).
fn row_posture(spec: &StoragePathSpec) -> Option<(u32, u32, u32)> {
    use d2b_core::storage::PrincipalKind;
    let owner_uid = match spec.owner.kind {
        PrincipalKind::Uid => spec.owner.value.as_str().parse::<u32>().ok()?,
        PrincipalKind::User => nix::unistd::User::from_name(spec.owner.value.as_str())
            .ok()?
            .map(|user| user.uid.as_raw())?,
        _ => return None,
    };
    let owner_gid = match spec.group.kind {
        PrincipalKind::Gid => spec.group.value.as_str().parse::<u32>().ok()?,
        PrincipalKind::Group => nix::unistd::Group::from_name(spec.group.value.as_str())
            .ok()?
            .map(|group| group.gid.as_raw())?,
        _ => return None,
    };
    let trimmed = spec.mode.trim_start_matches('0');
    let normalized = if trimmed.is_empty() { "0" } else { trimmed };
    let mode = u32::from_str_radix(normalized, 8).ok()?;
    Some((owner_uid, owner_gid, mode))
}

impl TpmResourceEffectPort for LiveTpmResourceEffectPort<'_> {
    async fn ensure_state_volume(
        &self,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        self.identity_matches(device_uid, device_ref, execution_ref)?;
        // The broker-owned legacy swTPM adoption op is removed; every v3
        // admission is anchorless, so a decision that still requires
        // migration - or whose intent no longer binds the current VM -
        // fail-closes instead of dispatching a removed op.
        if self.migration_decision.requires_migration()
            || !self
                .migration_decision
                .validates_binding(self.vm_id.as_str(), self.migration_intent_ref.as_str())
        {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        self.prepare_state_dir().await?;
        // The Volume row is committed before the flush and swtpm rows can use
        // it, so the caller waits for its own controller here.
        self.rows.ensure_state_volume().await
    }

    async fn request_flush_process(
        &self,
        device_uid: &ResourceUid,
        execution_ref: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        if device_uid != &self.device_uid || execution_ref != &self.execution_ref {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        let flush = self.rows.flush_ref()?;
        // The declared row's phase says the controller published a
        // classification; its status projection says which one-shot outcome
        // that was, so a failed flush fails this device path instead of being
        // read as a completed flush off a `Ready` phase.
        let view = self
            .rows
            .view(&flush)
            .await?
            .ok_or(TpmResourceEffectError::Transient)?;
        gate_declared_phase(Some(view_phase(&view)))?;
        gate_flush_outcome(&view)?;
        Ok(flush)
    }

    async fn request_swtpm_process(
        &self,
        device_uid: &ResourceUid,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        if device_uid != &self.device_uid || execution_ref != &self.execution_ref {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        let (expected_volume, _) = self.rows.state_volume()?;
        if volume_ref != &expected_volume {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        self.rows.wait_ready(&expected_volume).await?;
        let process = self.rows.process_ref()?;
        self.rows.wait_ready(&process).await?;
        self.consume_lifecycle_lease().await?;
        Ok(process)
    }

    async fn stop_swtpm_process(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<(), TpmResourceEffectError> {
        let expected = self.rows.process_ref()?;
        if process_ref != &expected {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        self.rows.delete(&expected).await
    }

    async fn delete_flush_process(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<(), TpmResourceEffectError> {
        let expected = self.rows.flush_ref()?;
        if process_ref != &expected {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        self.rows.delete(&expected).await
    }

    async fn watch_tpm_endpoint(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        let expected_process = self.rows.process_ref()?;
        if process_ref != &expected_process {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        let endpoint = self.rows.endpoint_ref()?;
        self.rows.wait_ready(&endpoint).await?;
        Ok(endpoint)
    }
}

/// Production Device controller reconcile callsite.
///
/// Core supplies the migration decision and opaque state intent; the daemon
/// supplies only the manager-routed child surface and the declared facets.
/// The migration receipt never crosses into the Provider crate.
pub struct AdmittedTpmDevice {
    device_uid: ResourceUid,
    device_ref: ResourceRef,
    zone: String,
    execution_ref: ResourceRef,
    operation_id: String,
}

impl AdmittedTpmDevice {
    /// One Device reconciled from its own row: the owning Guest's lifecycle
    /// admission is resolved by the daemon runtime facet when the pass
    /// reaches the launchable row.
    pub fn from_row(
        device_uid: ResourceUid,
        device_ref: ResourceRef,
        zone: impl Into<String>,
        execution_ref: ResourceRef,
        operation_id: impl Into<String>,
    ) -> Self {
        Self {
            device_uid,
            device_ref,
            zone: zone.into(),
            execution_ref,
            operation_id: operation_id.into(),
        }
    }

    fn into_port<'a>(
        self,
        facets: TpmEffectFacets,
        vm_id: VmId,
        migration_intent_ref: BundleOpId,
        migration_decision: LegacyTpmMigrationDecision,
        children: &'a dyn SharedProviderChildSurface,
    ) -> LiveTpmResourceEffectPort<'a> {
        LiveTpmResourceEffectPort {
            facets,
            vm_id,
            zone: self.zone.clone(),
            migration_intent_ref,
            migration_decision,
            rows: DeclaredTpmRows {
                children,
                zone: self.zone.clone(),
                device_uid: self.device_uid.clone(),
                device_ref: self.device_ref.clone(),
                execution_ref: self.execution_ref.clone(),
            },
            device_uid: self.device_uid,
            device_ref: self.device_ref,
            execution_ref: self.execution_ref,
            operation_id: self.operation_id,
            lifecycle_lease_consumed: tokio::sync::Mutex::new(false),
        }
    }
}

/// Reconcile one Device's TPM controller through the provider-owned port.
pub async fn reconcile_device_tpm_controller(
    facets: TpmEffectFacets,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    admitted_device: AdmittedTpmDevice,
    children: &dyn SharedProviderChildSurface,
    controller: &mut TpmResourceController,
) -> Result<TpmResourceOutcome, TpmResourceControllerError> {
    let resource_effect = admitted_device.into_port(
        facets,
        vm_id,
        migration_intent_ref,
        migration_decision,
        children,
    );
    controller.reconcile(&resource_effect).await
}

/// Finalize one Device's TPM controller through the provider-owned port.
pub async fn finalize_device_tpm_controller(
    facets: TpmEffectFacets,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    admitted_device: AdmittedTpmDevice,
    children: &dyn SharedProviderChildSurface,
    controller: &mut TpmResourceController,
) -> Result<TpmResourceOutcome, TpmResourceControllerError> {
    let resource_effect = admitted_device.into_port(
        facets,
        vm_id,
        migration_intent_ref,
        migration_decision,
        children,
    );
    controller.finalize(&resource_effect).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;

    use d2b_contracts_broker::broker_wire::BrokerCallerRole;
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::ResourceStatus;

    use crate::facets::TpmRuntime;

    use super::*;

    const DEVICE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const ZONE: &str = "work";
    const DEVICE_REF: &str = "Device/tpm-0";
    const EXECUTION_REF: &str = "Host/host-system";
    const STATE_VOLUME: &str = "Volume/device-123e4567e89b42d3a456426614174000-tpm-state";

    /// Scripted manager-over-child-surface double: rows keyed by canonical
    /// reference, plus the ensure/delete call log.
    #[derive(Default)]
    struct ScriptedRows {
        views: HashMap<String, ResourceView>,
        ensured: Vec<String>,
        deleted: Vec<String>,
    }

    struct RecordingChildSurface {
        rows: Mutex<ScriptedRows>,
        owner: ResourceKey,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl RecordingChildSurface {
        fn for_device(device_ref: &ResourceRef) -> Self {
            Self {
                rows: Mutex::new(ScriptedRows::default()),
                owner: ResourceKey::new(ZONE, "Device", device_ref.name().as_str()),
            }
        }

        fn publish(&self, reference: &str, status: ResourceStatus) {
            let parsed = ResourceRef::parse(reference).expect("canonical reference");
            let key = ResourceKey::new(
                ZONE,
                parsed.resource_type().as_str(),
                parsed.name().as_str(),
            );
            self.rows.lock().expect("rows").views.insert(
                reference.to_owned(),
                ResourceView {
                    key,
                    uid: [0x11; 16],
                    generation: 1,
                    deleting: false,
                    provenance: ResourceProvenance::Resource,
                    spec: b"{}".to_vec(),
                    metadata: Vec::new(),
                    owner_key: Some(self.owner.clone()),
                    status: Some(status),
                    status_generation: Some(1),
                    status_projection: None,
                },
            );
        }

        fn set_owner(&self, reference: &str, owner: ResourceKey) {
            if let Some(view) = self
                .rows
                .lock()
                .expect("rows")
                .views
                .get_mut(reference)
            {
                view.owner_key = Some(owner);
            }
        }

        fn ensured(&self) -> Vec<String> {
            self.rows.lock().expect("rows").ensured.clone()
        }

        fn deleted(&self) -> Vec<String> {
            self.rows.lock().expect("rows").deleted.clone()
        }

        /// Publish one declared row together with the driver-projected
        /// `status.resource` layer the Process controller writes for a
        /// one-shot's terminal outcome.
        fn publish_outcome(&self, reference: &str, status: ResourceStatus, projection: Value) {
            self.publish(reference, status);
            let mut rows = self.rows.lock().expect("rows");
            let view = rows
                .views
                .get_mut(reference)
                .expect("row published before its projection");
            view.status_projection = Some(projection);
        }
    }

    fn stored_row(child: &ChildEnsure) -> d2b_resource_runtime::identity::StoredDesiredResource {
        d2b_resource_runtime::identity::StoredDesiredResource {
            key: ResourceKey::new(ZONE, child.type_name.as_str(), child.name.clone()),
            uid: [0x22; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: child.spec.clone(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    #[async_trait::async_trait]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl SharedProviderChildSurface for RecordingChildSurface {
        async fn ensure(
            &self,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, d2b_provider_toolkit::SharedProviderEffectError> {
            self.rows
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("rows")
                .ensured
                .push(format!("{}/{}", child.type_name.as_str(), child.name));
            Ok(EnsureOutcome::Created(stored_row(&child)))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), d2b_provider_toolkit::SharedProviderEffectError> {
            self.rows
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("rows")
                .deleted
                .push(format!("{}/{}", key.type_name, key.name));
            Ok(())
        }

        async fn view(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<ResourceView>, d2b_provider_toolkit::SharedProviderEffectError> {
            Ok(self
                .rows
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("rows")
                .views
                .get(&format!("{}/{}", key.type_name, key.name))
                .cloned())
        }
    }

    fn rows<'a>(children: &'a RecordingChildSurface) -> DeclaredTpmRows<'a> {
        let device_ref = ResourceRef::parse(DEVICE_REF).expect("device ref");
        DeclaredTpmRows {
            children,
            zone: ZONE.to_owned(),
            device_uid: ResourceUid::parse(DEVICE_UID).expect("device uid"),
            device_ref,
            execution_ref: ResourceRef::parse(EXECUTION_REF).expect("execution ref"),
        }
    }

    /// The conversion's identity decision: the declared row names, exactly as
    /// the Provider's Nix projection authors them, replace the retired
    /// `device-<12hex>-*` derivations.
    #[test]
    fn declared_row_names_follow_the_provider_projection() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let rows = rows(&children);
        assert_eq!(
            rows.process_ref().expect("process").to_canonical_string(),
            "Process/swtpm-tpm-0"
        );
        assert_eq!(
            rows.flush_ref().expect("flush").to_canonical_string(),
            "EphemeralProcess/swtpm-flush-tpm-0"
        );
        assert_eq!(
            rows.endpoint_ref().expect("endpoint").to_canonical_string(),
            "Endpoint/tpm-tpm-0"
        );
        assert_eq!(
            rows.state_volume().expect("volume").0.to_canonical_string(),
            STATE_VOLUME
        );
    }

    /// Only the controller-owned state Volume is ensured; the declared
    /// Process/EphemeralProcess/Endpoint rows are the bundle's.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn only_the_state_volume_is_ensured() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        children.publish(STATE_VOLUME, ResourceStatus::Ready);
        let rows = rows(&children);
        assert_eq!(
            rows.ensure_state_volume()
                .await
                .expect("volume")
                .to_canonical_string(),
            STATE_VOLUME
        );
        assert_eq!(children.ensured(), vec![STATE_VOLUME.to_owned()]);
        // The declared worker rows are read straight from the manager.
        assert_eq!(rows.wait_ready(&rows.process_ref().unwrap()).await, Err(TpmResourceEffectError::Transient));
    }

    /// A declared row that has not converged is retryable, a failed one is
    /// terminal.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn declared_row_phases_gate_the_effects() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let rows = rows(&children);
        assert_eq!(
            rows.wait_ready(&rows.process_ref().unwrap()).await,
            Err(TpmResourceEffectError::Transient),
            "an absent row is not evidence",
        );
        children.publish("Process/swtpm-tpm-0", ResourceStatus::Pending);
        assert_eq!(
            rows.wait_ready(&rows.process_ref().unwrap()).await,
            Err(TpmResourceEffectError::Transient),
        );
        children.publish("Process/swtpm-tpm-0", ResourceStatus::Ready);
        assert_eq!(rows.wait_ready(&rows.process_ref().unwrap()).await, Ok(()));
        children.publish(
            "Process/swtpm-tpm-0",
            ResourceStatus::Failed(d2b_resource_runtime::error::DriverFailure::terminal(
                d2b_resource_runtime::error::DriverOp::Reconcile,
            )),
        );
        assert_eq!(
            rows.wait_ready(&rows.process_ref().unwrap()).await,
            Err(TpmResourceEffectError::EffectRejected),
        );
    }

    /// A row owned by another Device is never read as this Device's evidence
    /// (the owner fence the durable adoption classification carried).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_foreign_owned_row_fails_closed() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        children.publish("Process/swtpm-tpm-0", ResourceStatus::Ready);
        children.set_owner("Process/swtpm-tpm-0", ResourceKey::new(ZONE, "Device", "tpm-1"));
        let rows = rows(&children);
        assert_eq!(
            rows.wait_ready(&rows.process_ref().unwrap()).await,
            Err(TpmResourceEffectError::StateIntegrity),
        );
    }

    /// Read one published declared row's live view.
    async fn published_view(rows: &DeclaredTpmRows<'_>, reference: &ResourceRef) -> ResourceView {
        rows.view(reference).await.expect("view read").expect("row")
    }

    /// The pre-start flush row's completion is its **one-shot outcome**, not
    /// its phase: a concluded pass publishes `Ready` whatever the outcome was,
    /// so a flush that failed must still fail the device path (the exact
    /// regression the phase-only gate carried).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn the_flush_gate_reads_the_one_shot_outcome_not_only_the_phase() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let rows = rows(&children);
        let flush = rows.flush_ref().expect("flush");

        // No published outcome yet: the flush is still in flight.
        children.publish(&flush.to_canonical_string(), ResourceStatus::Pending);
        let live = published_view(&rows, &flush).await;
        assert_eq!(gate_flush_outcome(&live), Err(TpmResourceEffectError::Transient));

        // A failed one-shot publishes the ready phase and a failed outcome:
        // the phase gate alone would have read the flush as completed.
        children.publish_outcome(
            &flush.to_canonical_string(),
            ResourceStatus::Ready,
            serde_json::json!({"ephemeral": {"state": "failed", "code": "runtime-deadline"}}),
        );
        let failed = published_view(&rows, &flush).await;
        assert_eq!(
            gate_declared_phase(Some(view_phase(&failed))),
            Ok(()),
            "the runtime classifies a concluded pass as ready"
        );
        assert_eq!(
            gate_flush_outcome(&failed),
            Err(TpmResourceEffectError::EffectRejected),
            "the failed one-shot outcome fails the device path"
        );

        // A clean exit is the only outcome that completes the flush.
        children.publish_outcome(
            &flush.to_canonical_string(),
            ResourceStatus::Ready,
            serde_json::json!({"ephemeral": {"state": "succeeded", "code": "process-exited"}}),
        );
        assert_eq!(gate_flush_outcome(&published_view(&rows, &flush).await), Ok(()));

        // An unreadable projection is refused, never read as success.
        children.publish_outcome(
            &flush.to_canonical_string(),
            ResourceStatus::Ready,
            serde_json::json!({"ephemeral": {"state": "unclassified"}}),
        );
        assert_eq!(
            gate_flush_outcome(&published_view(&rows, &flush).await),
            Err(TpmResourceEffectError::StateIntegrity)
        );
    }

    /// Removal rides the manager: both declared worker rows are retired by
    /// reference through the same child surface.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn deletion_targets_the_declared_rows() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let rows = rows(&children);
        rows.delete(&rows.process_ref().unwrap()).await.expect("stop");
        rows.delete(&rows.flush_ref().unwrap()).await.expect("delete");
        assert_eq!(
            children.deleted(),
            vec![
                "Process/swtpm-tpm-0".to_owned(),
                "EphemeralProcess/swtpm-flush-tpm-0".to_owned(),
            ]
        );
    }

    /// A production port never reads another Device's uid, ref, or
    /// execution as its own (the adoption owner fence).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_state_volume_rejects_a_mismatched_identity() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let decision = LegacyTpmMigrationDecision::not_applicable("work-vm", "legacy-swtpm:vm:work");
        let port = make_port(
            crate::test_support::recording_facets(),
            &children,
            decision,
            "legacy-swtpm:vm:work",
        );
        let uid = ResourceUid::parse(DEVICE_UID).unwrap();
        let device = ResourceRef::parse(DEVICE_REF).unwrap();
        let execution = ResourceRef::parse(EXECUTION_REF).unwrap();
        for (uid, reference, execution) in [
            (
                ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
                device.clone(),
                execution.clone(),
            ),
            (
                uid.clone(),
                ResourceRef::parse("Device/tpm-1").unwrap(),
                execution.clone(),
            ),
            (
                uid.clone(),
                device.clone(),
                ResourceRef::parse("Host/other").unwrap(),
            ),
        ] {
            assert_eq!(
                port.ensure_state_volume(&uid, &reference, &execution).await.unwrap_err(),
                TpmResourceEffectError::StateIntegrity
            );
        }
    }

    /// The migration decision gates port admission closed: a decision that
    /// still requires migration, or whose intent no longer binds, fails
    /// before any state-directory preparation runs.

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_state_volume_fails_closed_on_requiring_or_unbound_migration() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        let uid = ResourceUid::parse(DEVICE_UID).unwrap();
        let device = ResourceRef::parse(DEVICE_REF).unwrap();
        let execution = ResourceRef::parse(EXECUTION_REF).unwrap();

        let decision = LegacyTpmMigrationDecision::adoption_required(
            "work-vm",
            "legacy-swtpm:vm:work",
            "legacy-swtpm:vm:work",
        );
        let port = make_port(
            crate::test_support::recording_facets(),
            &children,
            decision,
            "legacy-swtpm:vm:work",
        );
        assert_eq!(
            port.ensure_state_volume(&uid, &device, &execution).await.unwrap_err(),
            TpmResourceEffectError::StateIntegrity
        );

        let decision = LegacyTpmMigrationDecision::not_applicable("work-vm", "legacy-swtpm:vm:work");
        let port = make_port(
            crate::test_support::recording_facets(),
            &children,
            decision,
            "legacy-swtpm:vm:other",
        );
        assert_eq!(
            port.ensure_state_volume(&uid, &device, &execution).await.unwrap_err(),
            TpmResourceEffectError::StateIntegrity
        );

        // A decision that never applied passes the gates and fails closed when
        // the trusted bundle cannot be loaded (retired-arm load-failure parity).
        let decision = LegacyTpmMigrationDecision::not_applicable("work-vm", "legacy-swtpm:vm:work");
        let port = make_port(
            crate::test_support::recording_facets(),
            &children,
            decision,
            "legacy-swtpm:vm:work",
        );
        assert_eq!(
            port.ensure_state_volume(&uid, &device, &execution).await.unwrap_err(),
            TpmResourceEffectError::Transient
        );
    }

    /// The Core-issued lifecycle lease is consumed at most once per port:
    /// the second launchable pass retires without re-consuming.

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn the_lifecycle_lease_is_consumed_at_most_once() {
        let children = RecordingChildSurface::for_device(&ResourceRef::parse(DEVICE_REF).unwrap());
        children.publish(STATE_VOLUME, ResourceStatus::Ready);
        children.publish("Process/swtpm-tpm-0", ResourceStatus::Ready);

        let runtime = Arc::new(RecordingRuntime::default());
        let facets = TpmEffectFacets { runtime: runtime.clone() };
        let decision = LegacyTpmMigrationDecision::not_applicable("work-vm", "legacy-swtpm:vm:work");
        let port = make_port(facets, &children, decision, "legacy-swtpm:vm:work");
        let uid = ResourceUid::parse(DEVICE_UID).unwrap();
        let execution = ResourceRef::parse(EXECUTION_REF).unwrap();
        let volume = ResourceRef::parse(STATE_VOLUME).unwrap();
        for _ in 0..2 {
            assert_eq!(
                port
                    .request_swtpm_process(&uid, &volume, &execution)
                    .await
                    .unwrap()
                    .to_canonical_string(),
                "Process/swtpm-tpm-0"
            );
        }
        let calls = runtime.lease_calls.lock().expect("lease calls"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        assert_eq!(
            calls.as_slice(),
            [("work-vm".to_owned(), "operation-1".to_owned())],
        );
    }

    /// A test-helper runtime that records lifecycle-lease consumption instead
    /// of failing closed.

    #[derive(Default)]
    struct RecordingRuntime {
        socket_path: PathBuf,
        lease_calls: Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl TpmRuntime for RecordingRuntime {
        fn broker_socket_path(&self) -> &Path {
            &self.socket_path
        }

        fn caller_role(&self) -> BrokerCallerRole {
            BrokerCallerRole::AdminUid { uid: 0 }
        }

        fn kernel_io_timeout(&self) -> Duration {
            Duration::from_secs(5)
        }

        async fn load_bundle(
            &self,
        ) -> Result<Arc<BundleResolver>, TpmResourceEffectError> {
            Err(TpmResourceEffectError::Transient)
        }

        async fn consume_lifecycle_lease(
            &self,
            vm_id: &str,
            operation_id: &str,
        ) -> Result<(), TpmResourceEffectError> {
            self.lease_calls
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("lease calls")
                .push((vm_id.to_owned(), operation_id.to_owned()));
            Ok(())
        }
    }

    /// One production-port instance over scripted rows and a facet set.
    fn make_port<'a>(
        facets: TpmEffectFacets,
        children: &'a RecordingChildSurface,
        decision: LegacyTpmMigrationDecision,
        intent: &str,
    ) -> LiveTpmResourceEffectPort<'a> {
        LiveTpmResourceEffectPort {
            facets,
            vm_id: VmId::new("work-vm"),
            zone: ZONE.to_owned(),
            migration_intent_ref: BundleOpId::new(intent),
            migration_decision: decision,
            rows: rows(children),
            device_uid: ResourceUid::parse(DEVICE_UID).expect("device uid"),
            device_ref: ResourceRef::parse(DEVICE_REF).expect("device ref"),
            execution_ref: ResourceRef::parse(EXECUTION_REF).expect("execution ref"),
            operation_id: "operation-1".to_owned(),
            lifecycle_lease_consumed: tokio::sync::Mutex::new(false),
        }
    }

    /// Malformed manager-child documents fail closed before any mutation: a
    /// missing type, metadata.name, or spec is rejected, and an
    /// unparseable type/name pair can never name a row.

    #[test]
    fn declared_child_rejects_malformed_documents() {
        for document in [
            serde_json::json!({"metadata": {"name": "a"}, "spec": {}}),
            serde_json::json!({"type": "Volume", "spec": {}}),
            serde_json::json!({"type": "Volume", "metadata": {"name": "a"}}),
        ] {
            assert_eq!(
                declared_child(document).unwrap_err(),
                TpmResourceEffectError::EffectRejected
            );
        }
        assert_eq!(
            declared_child(serde_json::json!({
                "type": "Volume",
                "metadata": {"name": ""},
                "spec": {},
            }))
            .unwrap_err(),
            TpmResourceEffectError::InvalidDevice
        );
    }
}