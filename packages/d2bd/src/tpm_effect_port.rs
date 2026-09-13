//! Core-owned production adapter for the Device TPM Provider effect boundary.
//!
//! The Provider receives no broker handle, host locator, or Core migration
//! receipt. Core supplies the migration decision and the Device-owned child
//! rows; this adapter is the only place that maps the private decision onto
//! the typed broker operation the migration needs.
//!
//! Row ownership (U17, KTD13). The daemon-side effect is realized through the
//! manager, never through a broker spawn:
//!
//! - the controller-owned state Volume `Volume/device-<32hex>-tpm-state` is
//!   ensured as an owner-scoped child of the requiring Device (the Provider's
//!   own `build_tpm_state_volume_resource` authors the body);
//! - the Provider-declared `EphemeralProcess/swtpm-flush-<device>`,
//!   `Process/swtpm-<device>` and `Endpoint/tpm-<device>` rows are read
//!   through the same child surface and gated on their published phase, so
//!   exactly one component - the Process controller - decides when swtpm and
//!   the flush live, restart, are adopted across daemon restarts, drain, and
//!   are torn down;
//! - `stop_swtpm_process` / `delete_flush_process` retire those declared rows
//!   through the manager, which runs the preserved stop/finalize before
//!   removal.
//!
//! The declared rows are the launch authority: their specs stay the bundle's
//! (re-authoring them here would differ byte-wise from the seeded row and
//! mutate it on every pass), and the launch parameters their templates admit
//! travel on the Process controller's `launch_args` channel.

use std::sync::Mutex;

use d2b_contracts::types::{BundleOpId, PathClass, VmId};
use d2b_contracts_broker::broker_wire::{BrokerCallerRole, BrokerRequest, BrokerResponse};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_provider_device_tpm::{
    LegacyMigrationOutcome, TpmResourceController, TpmResourceEffectError, TpmResourceEffectPort,
    TpmResourceOutcome, build_tpm_state_volume_resource,
};
use d2b_resource_runtime::context::ChildEnsure;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::ResourceView;
use serde_json::Value;

use crate::provider_effects::{GuestLifecycleOperation, LifecycleAuthorization};
use crate::shared_provider_driver::SharedProviderChildSurface;

fn map_legacy_migration_outcome(
    outcome: d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome,
) -> LegacyMigrationOutcome {
    match outcome {
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::Migrated => {
            LegacyMigrationOutcome::Migrated
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::AlreadyMigrated => {
            LegacyMigrationOutcome::AlreadyMigrated
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::NotApplicable => {
            LegacyMigrationOutcome::NotApplicable
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::Pending => {
            LegacyMigrationOutcome::Pending
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::Failed => {
            LegacyMigrationOutcome::Failed
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::Ambiguous => {
            LegacyMigrationOutcome::Ambiguous
        }
        d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::AdoptionRequired
        | d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome::NeverProvisioned => {
            LegacyMigrationOutcome::Ambiguous
        }
    }
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
/// `process_driver.rs::publish_ephemeral_outcome`). A row that has not
/// published one yet is retryable (the flush is still in flight); a failed
/// outcome is the flush's own failure and fails the device path closed; an
/// unreadable projection is refused rather than read as success.
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
        ResourceRef::parse(&format!("Process/swtpm-{}", self.device_ref.name().as_str()))
            .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The declared pre-start flush row
    /// (`EphemeralProcess/swtpm-flush-<device>`).
    fn flush_ref(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        ResourceRef::parse(&format!(
            "EphemeralProcess/swtpm-flush-{}",
            self.device_ref.name().as_str()
        ))
        .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The declared TPM Endpoint (`Endpoint/tpm-<device>`).
    fn endpoint_ref(&self) -> Result<ResourceRef, TpmResourceEffectError> {
        ResourceRef::parse(&format!("Endpoint/tpm-{}", self.device_ref.name().as_str()))
            .map_err(|_| TpmResourceEffectError::InvalidDevice)
    }

    /// The controller-owned state Volume document and reference
    /// (`Volume/device-<32hex>-tpm-state`).
    fn state_volume(
        &self,
    ) -> Result<(ResourceRef, ChildEnsure), TpmResourceEffectError> {
        let document = build_tpm_state_volume_resource(
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
            .map(crate::shared_provider_effects::view_phase))
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
            .map_err(|_| TpmResourceEffectError::Transient)?;
        self.wait_ready(&reference).await?;
        Ok(reference)
    }
}

/// Where one Device's Guest lifecycle admission comes from.
///
/// The public Device-start dispatch admits the lease from the requesting peer
/// and hands the issued lease over; a row-driven reconcile has no peer, so it
/// resolves the owning Guest's admission itself.
enum TpmLifecycleAdmission {
    /// The caller already admitted the lease (the public peer path).
    Issued(LifecycleAuthorization),
    /// Resolve the owning Guest's admission on first use (the row-driven
    /// path).
    Internal { operation_id: String },
}

/// Concrete daemon-side TPM resource effect port.
///
/// The port holds no broker spawn surface by construction: every effect is a
/// manager child mutation or a manager view of a Device-owned row. The two
/// broker calls that remain are the one-time legacy state adoption and the
/// broker-owned state-directory preparation, neither of which launches a
/// process.
struct LiveTpmResourceEffectPort<'a> {
    state: &'a crate::ServerState,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    caller_role: BrokerCallerRole,
    rows: DeclaredTpmRows<'a>,
    device_uid: ResourceUid,
    device_ref: ResourceRef,
    execution_ref: ResourceRef,
    /// The Device's owning Guest lifecycle admission, resolved the first time
    /// the pass reaches the launchable row the lease is consumed by.
    lifecycle_admission: Mutex<TpmLifecycleAdmission>,
    /// The guest lifecycle lease is consumed at most once, by the first
    /// effect that reaches a launchable row (the preserved
    /// `lifecycle_lease_consumed` gate of the old executor).
    lifecycle_lease_consumed: Mutex<bool>,
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

    /// Resolve the owning Guest's lifecycle admission.
    ///
    /// Only the launch admission consumes it (`consume_lifecycle_lease`
    /// below). The controller's earlier stages - the state Volume the Device
    /// owns, its layout effect, the pre-start flush - are Device-owned rows
    /// that no Guest lifecycle lease covers, so they must not be gated behind
    /// this admission: the state directory its worker opens is provisioned
    /// first, and the launch fails closed here instead.
    fn lifecycle_authorization(&self) -> Result<LifecycleAuthorization, TpmResourceEffectError> {
        let mut slot = self
            .lifecycle_admission
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        if let TpmLifecycleAdmission::Issued(authorization) = &*slot {
            return Ok(authorization.clone());
        }
        let TpmLifecycleAdmission::Internal { operation_id } = &*slot else {
            unreachable!("the admission source is Issued or Internal");
        };
        let operation_id = operation_id.clone();
        let zone = ZoneId::parse(self.rows.zone.as_str())
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?;
        let runtime = self
            .state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&zone).ok()))
            .ok_or(TpmResourceEffectError::Transient)?;
        let guest_ref = ResourceRef::parse(&format!("Guest/{}", self.vm_id.as_str()))
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?;
        let admission = crate::block_on_future(
            runtime.admit_internal_guest_lifecycle(guest_ref.clone(), &operation_id),
        )
        .map_err(|_| TpmResourceEffectError::Transient)?;
        let authorization = LifecycleAuthorization::from_lease(
            admission.lease,
            guest_ref,
            admission.guest_uid,
            admission.guest_generation,
            admission.provider_assignment_generation,
        )
        .map_err(|_| TpmResourceEffectError::StateIntegrity)?;
        *slot = TpmLifecycleAdmission::Issued(authorization.clone());
        Ok(authorization)
    }

    /// Consume the Core-issued guest lifecycle lease exactly once. The lease
    /// authorized this Device's start operation; the row's Process controller
    /// owns the process from here, so the port only retires the admission.
    fn consume_lifecycle_lease(&self) -> Result<(), TpmResourceEffectError> {
        let mut consumed = self
            .lifecycle_lease_consumed
            .lock()
            .map_err(|_| TpmResourceEffectError::Transient)?;
        if *consumed {
            return Ok(());
        }
        let authorization = self.lifecycle_authorization()?;
        crate::consume_lifecycle_lease(
            self.state,
            &authorization,
            GuestLifecycleOperation::Start,
            &self.caller_role,
        )
        .map_err(|_| TpmResourceEffectError::EffectRejected)?;
        *consumed = true;
        Ok(())
    }

    /// Complete or resume the broker-owned one-time legacy state adoption.
    /// Not a spawn: the broker migrates the trusted on-disk inventory and
    /// answers with the closed outcome.
    fn migrate_legacy_state(&self) -> Result<(), TpmResourceEffectError> {
        if !self.migration_decision.requires_migration()
            || !self
                .migration_decision
                .validates_binding(self.vm_id.as_str(), self.migration_intent_ref.as_str())
        {
            return Err(TpmResourceEffectError::StateIntegrity);
        }
        let outcome = crate::dispatch_broker_legacy_tpm_migration(
            self.state,
            self.vm_id.clone(),
            self.migration_intent_ref.clone(),
        )
        .map_err(|_| TpmResourceEffectError::Transient)?;
        match map_legacy_migration_outcome(outcome) {
            LegacyMigrationOutcome::Migrated
            | LegacyMigrationOutcome::AlreadyMigrated
            | LegacyMigrationOutcome::NotApplicable => Ok(()),
            LegacyMigrationOutcome::Pending => Err(TpmResourceEffectError::Transient),
            LegacyMigrationOutcome::Failed | LegacyMigrationOutcome::Ambiguous => {
                Err(TpmResourceEffectError::StateIntegrity)
            }
        }
    }

    /// The broker-owned state-directory preparation (the trusted marker and
    /// hardening step the legacy TPM connector supplies). Not a spawn.
    fn prepare_state_dir(&self) -> Result<(), TpmResourceEffectError> {
        let response = crate::dispatch_broker_request_as(
            self.state,
            BrokerRequest::PrepareStateDir(
                d2b_contracts_broker::broker_wire::PrepareDirRequest {
                    vm_id: self.vm_id.clone(),
                    path_class: PathClass::Vm,
                    tracing_span_id: None,
                },
            ),
            self.caller_role.clone(),
        )
        .map_err(|_| TpmResourceEffectError::Transient)?;
        match response {
            BrokerResponse::Ack(_) => Ok(()),
            other => {
                tracing::warn!(
                    device = %self.device_ref.to_canonical_string(),
                    response = ?other,
                    "broker state-directory preparation refused",
                );
                Err(TpmResourceEffectError::StateIntegrity)
            }
        }
    }
}

impl TpmResourceEffectPort for LiveTpmResourceEffectPort<'_> {
    async fn ensure_state_volume(
        &self,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        self.identity_matches(device_uid, device_ref, execution_ref)?;
        if self.migration_decision.requires_migration() {
            self.migrate_legacy_state()?;
        }
        self.prepare_state_dir()?;
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
        gate_declared_phase(Some(crate::shared_provider_effects::view_phase(&view)))?;
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
        self.consume_lifecycle_lease()?;
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
/// supplies only the manager-routed child surface. The migration receipt
/// never crosses into the Provider crate.
pub(crate) struct AdmittedTpmDevice {
    device_uid: ResourceUid,
    device_ref: ResourceRef,
    zone: String,
    execution_ref: ResourceRef,
    lifecycle_admission: TpmLifecycleAdmission,
}

impl AdmittedTpmDevice {
    /// One Device admitted from an already issued Guest lifecycle lease (the
    /// public peer path, retained by the legacy dispatch simulation).
    #[cfg(test)]
    pub(crate) fn new(
        device_uid: ResourceUid,
        device_ref: ResourceRef,
        zone: impl Into<String>,
        execution_ref: ResourceRef,
        lifecycle_authorization: LifecycleAuthorization,
    ) -> Self {
        Self {
            device_uid,
            device_ref,
            zone: zone.into(),
            execution_ref,
            lifecycle_admission: TpmLifecycleAdmission::Issued(lifecycle_authorization),
        }
    }

    /// One Device reconciled from its own row: the owning Guest's lifecycle
    /// admission is resolved when the pass reaches the launchable row.
    pub(crate) fn from_row(
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
            lifecycle_admission: TpmLifecycleAdmission::Internal {
                operation_id: operation_id.into(),
            },
        }
    }

    fn into_port<'a>(
        self,
        state: &'a crate::ServerState,
        vm_id: VmId,
        migration_intent_ref: BundleOpId,
        migration_decision: LegacyTpmMigrationDecision,
        caller_role: BrokerCallerRole,
        children: &'a dyn SharedProviderChildSurface,
    ) -> LiveTpmResourceEffectPort<'a> {
        LiveTpmResourceEffectPort {
            state,
            vm_id,
            migration_intent_ref,
            migration_decision,
            caller_role,
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
            lifecycle_admission: Mutex::new(self.lifecycle_admission),
            lifecycle_lease_consumed: Mutex::new(false),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconcile_device_tpm_controller(
    state: &crate::ServerState,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    admitted_device: AdmittedTpmDevice,
    caller_role: BrokerCallerRole,
    children: &dyn SharedProviderChildSurface,
    controller: &mut TpmResourceController,
) -> Result<TpmResourceOutcome, d2b_provider_device_tpm::TpmResourceControllerError> {
    let resource_effect = admitted_device.into_port(
        state,
        vm_id,
        migration_intent_ref,
        migration_decision,
        caller_role,
        children,
    );
    crate::block_on_future(controller.reconcile(&resource_effect))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_device_tpm_controller(
    state: &crate::ServerState,
    vm_id: VmId,
    migration_intent_ref: BundleOpId,
    migration_decision: LegacyTpmMigrationDecision,
    admitted_device: AdmittedTpmDevice,
    caller_role: BrokerCallerRole,
    children: &dyn SharedProviderChildSurface,
    controller: &mut TpmResourceController,
) -> Result<TpmResourceOutcome, d2b_provider_device_tpm::TpmResourceControllerError> {
    let resource_effect = admitted_device.into_port(
        state,
        vm_id,
        migration_intent_ref,
        migration_decision,
        caller_role,
        children,
    );
    crate::block_on_future(controller.finalize(&resource_effect))
}

/// Fail-closed child surface for callers that hold no manager context.
///
/// The retained legacy dispatch simulation
/// (`composition::dispatch_device_tpm_reconcile`) runs outside a driver and
/// cannot realize Device-owned rows; it refuses instead of re-introducing a
/// spawn. The v3 path is the SharedProvider driver's `reconcile_tpm` effect,
/// which passes its own [`SharedProviderChildSurface`].
#[cfg(test)]
pub(crate) struct NoManagerChildSurface;

#[cfg(test)]
#[async_trait::async_trait]
impl SharedProviderChildSurface for NoManagerChildSurface {
    async fn ensure(
        &self,
        _child: ChildEnsure,
    ) -> Result<d2b_resource_runtime::spec_store::EnsureOutcome, crate::shared_provider_driver::SharedProviderEffectError>
    {
        Err(crate::shared_provider_driver::SharedProviderEffectError::Unavailable)
    }

    async fn delete(&self, _key: &ResourceKey) -> Result<(), crate::shared_provider_driver::SharedProviderEffectError> {
        Err(crate::shared_provider_driver::SharedProviderEffectError::Unavailable)
    }

    async fn view(
        &self,
        _key: &ResourceKey,
    ) -> Result<Option<ResourceView>, crate::shared_provider_driver::SharedProviderEffectError> {
        Err(crate::shared_provider_driver::SharedProviderEffectError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use d2b_contracts_broker::broker_wire::LegacySwtpmMigrationOutcome;
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::ResourceStatus;

    use super::*;

    const DEVICE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const ZONE: &str = "work";
    const DEVICE_REF: &str = "Device/tpm-0";
    const EXECUTION_REF: &str = "Host/host-system";
    const STATE_VOLUME: &str = "Volume/device-123e4567e89b42d3a456426614174000-tpm-state";

    #[test]
    fn broker_migration_outcomes_are_preserved_at_the_provider_boundary() {
        for (broker, provider) in [
            (
                LegacySwtpmMigrationOutcome::Migrated,
                LegacyMigrationOutcome::Migrated,
            ),
            (
                LegacySwtpmMigrationOutcome::AlreadyMigrated,
                LegacyMigrationOutcome::AlreadyMigrated,
            ),
            (
                LegacySwtpmMigrationOutcome::NotApplicable,
                LegacyMigrationOutcome::NotApplicable,
            ),
            (
                LegacySwtpmMigrationOutcome::Pending,
                LegacyMigrationOutcome::Pending,
            ),
            (
                LegacySwtpmMigrationOutcome::Failed,
                LegacyMigrationOutcome::Failed,
            ),
            (
                LegacySwtpmMigrationOutcome::Ambiguous,
                LegacyMigrationOutcome::Ambiguous,
            ),
        ] {
            assert_eq!(map_legacy_migration_outcome(broker), provider);
        }
    }

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
    impl SharedProviderChildSurface for RecordingChildSurface {
        async fn ensure(
            &self,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, crate::shared_provider_driver::SharedProviderEffectError> {
            self.rows
                .lock()
                .expect("rows")
                .ensured
                .push(format!("{}/{}", child.type_name.as_str(), child.name));
            Ok(EnsureOutcome::Created(stored_row(&child)))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), crate::shared_provider_driver::SharedProviderEffectError> {
            self.rows
                .lock()
                .expect("rows")
                .deleted
                .push(format!("{}/{}", key.type_name, key.name));
            Ok(())
        }

        async fn view(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<ResourceView>, crate::shared_provider_driver::SharedProviderEffectError> {
            Ok(self
                .rows
                .lock()
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
            gate_declared_phase(Some(crate::shared_provider_effects::view_phase(&failed))),
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
}
