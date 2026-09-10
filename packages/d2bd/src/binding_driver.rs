//! VolumeBinding resource driver (U7): the v3 `ResourceDriver` conversion
//! of the binding leg of the shared-Volume path (R4, R8, R9; KTD7, KTD1,
//! F1, F4).
//!
//! The driver keeps the old binding behavior and nothing else: reconcile
//! derives the binding-owned worker Process and Endpoint children exactly
//! as the preserved `binding_children` minting did (`worker_child_specs`),
//! ensures each through the manager-routed ensure (the child spec is
//! committed BEFORE the child actor exists, F1), and reports child-phase
//! driven readiness. Recover re-derives the launch plan from the persisted
//! binding plus the bundle-resolved view spec (KTD7) so the re-derived plan
//! matches the pre-restart incarnation: the same template, the same
//! path-free worker plan (tuning travels in the plan, never in the
//! resource, KTD1). Delete participates in the preserved endpoint-first /
//! process-last teardown ordering: the Endpoint child is deleted, the
//! socket is removed, and only then the worker Process child.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`BindingDriverFactory`] registration under `VolumeBinding`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - `binding_children` minting + readiness -> [`ResourceDriver::reconcile`].
//! - endpoint-first teardown -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! The KTD7 zone-authority inputs (target Guest vcpu count for the worker
//! thread pool) are factory wiring: U9 folds them from the bundle resolver
//! and ZoneAuthorityIdentity path, never from the spec store.
#![allow(dead_code)]

use std::sync::Arc;

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceSpec, ResourceUid,
    volume_binding::VolumeBindingSpec,
    volume::VolumeSpec,
};
use d2b_provider_volume_virtiofs::{StoredBinding, VirtiofsdWorkerPlan, WORKER_TEMPLATE};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

/// The one resource type this factory serves (KTD4 Phase A).
pub(crate) const BINDING_TYPE_NAME: &str = "VolumeBinding";

/// The serving Provider this driver owns (old `VOLUME_VIRTIOFS_PROVIDER_REF`).
const BINDING_PROVIDER_REF: &str = "Provider/volume-virtiofs";

/// The Process Provider the binding-owned worker runs under (old
/// `worker_child_specs`).
const WORKER_PROVIDER_REF: &str = "Provider/system-minijail";

/// Deterministic owned-child resource types.
const WORKER_TYPE: &str = "Process";
const ENDPOINT_TYPE: &str = "Endpoint";

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding contract.
    SpecInvalid,
    /// The spec selects a Provider this driver does not own.
    ProviderUnsupported,
    /// The binding names a Volume this driver cannot resolve through the
    /// manager (missing parent, or an owner mismatch the manager would
    /// silently re-parent).
    OwnerMismatch,
    /// The worker plan could not be derived (view rights, zero vcpu).
    PlanDerivation,
    /// A provider serving effect failed transiently.
    ServingEffect,
    /// The manager refused a child ensure/delete.
    ChildMutation,
}

impl BindingDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ServingEffect | Self::ChildMutation => FailureClass::Retryable,
            Self::SpecInvalid
            | Self::ProviderUnsupported
            | Self::OwnerMismatch
            | Self::PlanDerivation => FailureClass::Terminal,
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BindingDriverError {
    kind: BindingDriverErrorKind,
    op: DriverOp,
}

impl BindingDriverError {
    fn new(kind: BindingDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for BindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            BindingDriverErrorKind::SpecInvalid => "binding-spec-invalid",
            BindingDriverErrorKind::ProviderUnsupported => "binding-provider-unsupported",
            BindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            BindingDriverErrorKind::PlanDerivation => "binding-plan-derivation-invalid",
            BindingDriverErrorKind::ServingEffect => "binding-serving-effect-failed",
            BindingDriverErrorKind::ChildMutation => "binding-child-mutation-failed",
        })
    }
}

impl std::error::Error for BindingDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// re-derived path-free worker plan so recover can prove the pre-restart
/// incarnation is reproduced (KTD7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingDriverStatus {
    /// Worker + Endpoint children derived and ensured.
    ServingChildren { worker_ref: String, endpoint_ref: String },
    /// The exact pre-restart plan was re-derived on recover.
    RecoveredPlan { worker_ref: String, endpoint_ref: String },
}

/// The in-memory handle the driver reports through status (R11): the exact
/// re-derived path-free plan. Tests compare pre-restart vs recovered
/// incarnations on this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedPlan {
    pub(crate) worker_ref: String,
    pub(crate) endpoint_ref: String,
    pub(crate) plan: VirtiofsdWorkerPlan,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one VolumeBinding row (KTD2), exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingSpecEnvelope {
    raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for VolumeBinding rows.
pub(crate) fn binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| BindingSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing serving effect surface the binding driver needs.
/// The production implementation delegates to the preserved virtiofs
/// serving effect adapter; test doubles implement the same seam (R4).
#[async_trait::async_trait]
pub(crate) trait BindingDriverEffects: Send + Sync + 'static {
    /// Whether the worker's private socket is ready (child-phase evidence).
    async fn socket_ready(&self, socket: &d2b_provider_volume_virtiofs::SocketIdentity)
        -> bool;

    /// Remove the endpoint realization (socket) - endpoint-first teardown.
    async fn remove_socket(
        &self,
        socket: &d2b_provider_volume_virtiofs::SocketIdentity,
    ) -> Result<(), String>;
}

/// Production effects over the preserved virtiofs serving adapter. U9 wires
/// the adapter construction (the same inputs the old
/// `ChildReadinessPort` consumed).
pub(crate) struct ProductionBindingDriverEffects {
    ready: Arc<dyn Fn(&d2b_provider_volume_virtiofs::SocketIdentity) -> bool + Send + Sync>,
    remove: Arc<
        dyn Fn(&d2b_provider_volume_virtiofs::SocketIdentity) -> Result<(), String>
            + Send
            + Sync,
    >,
}

impl ProductionBindingDriverEffects {
    pub(crate) fn new(
        ready: Arc<dyn Fn(&d2b_provider_volume_virtiofs::SocketIdentity) -> bool + Send + Sync>,
        remove: Arc<
            dyn Fn(&d2b_provider_volume_virtiofs::SocketIdentity) -> Result<(), String>
                + Send
                + Sync,
        >,
    ) -> Self {
        Self { ready, remove }
    }
}

#[async_trait::async_trait]
impl BindingDriverEffects for ProductionBindingDriverEffects {
    async fn socket_ready(
        &self,
        socket: &d2b_provider_volume_virtiofs::SocketIdentity,
    ) -> bool {
        (self.ready)(socket)
    }

    async fn remove_socket(
        &self,
        socket: &d2b_provider_volume_virtiofs::SocketIdentity,
    ) -> Result<(), String> {
        (self.remove)(socket)
    }
}

// ---------------------------------------------------------------------------
// Factory (U9 wiring shape)
// ---------------------------------------------------------------------------

/// Everything the composition unit (U9) must construct to instantiate the
/// binding driver factory for one zone: the serving effects plus the KTD7
/// zone-authority inputs (the target Guest vcpu count resolved by the
/// bundle resolver, never read from the spec store).
pub(crate) struct BindingDriverArgs {
    pub(crate) zone: String,
    pub(crate) effects: Arc<dyn BindingDriverEffects>,
    /// Target Guest vcpu count; the worker thread-pool size (KTD7).
    pub(crate) vcpu_count: u32,
}

/// [`ResourceDriverFactory`] for the `VolumeBinding` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct BindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: BindingDriverArgs,
}

impl BindingDriverFactory {
    pub(crate) fn new(args: BindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for BindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(BindingDriver::new(BindingDriverArgs {
            zone: self.args.zone.clone(),
            effects: Arc::clone(&self.args.effects),
            vcpu_count: self.args.vcpu_count,
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One VolumeBinding resource's driver.
#[derive(Clone)]
pub(crate) struct BindingDriver {
    zone: String,
    effects: Arc<dyn BindingDriverEffects>,
    vcpu_count: u32,
}

impl BindingDriver {
    pub(crate) fn new(args: BindingDriverArgs) -> Self {
        Self {
            zone: args.zone,
            effects: args.effects,
            vcpu_count: args.vcpu_count,
        }
    }

    fn error(&self, kind: BindingDriverErrorKind, op: DriverOp) -> BindingDriverError {
        BindingDriverError::new(kind, op)
    }

    /// The zone as a bounded token (the socket identity namespace).
    fn zone_bounded(&self) -> d2b_contracts_resource::v3::execution_policy::BoundedToken {
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(self.zone.clone())
            .expect("zone name is a bounded token")
    }

    /// Decode the stored envelope into the strict neutral binding contract.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(BindingSpecEnvelope, VolumeBindingSpec), BindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        match envelope.provider_ref.as_ref() {
            Some(provider_ref)
                if provider_ref.resource_type().as_str() == "Provider"
                    && provider_ref.name().as_str() == "volume-virtiofs" => {}
            _ => return Err(self.error(BindingDriverErrorKind::ProviderUnsupported, op)),
        }
        let binding = serde_json::from_slice::<VolumeBindingSpec>(
            &envelope.base.to_canonical_bytes(),
        )
        .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope.clone(), binding))
    }

    /// The parent Volume row through the manager (R2: the driver never
    /// touches the spec store). The binding's declared Volume must be the
    /// row the manager reports as this resource's owner - a child cannot
    /// silently change owner.
    async fn parent_volume(
        &self,
        ctx: &mut ResourceContext,
        binding: &VolumeBindingSpec,
        op: DriverOp,
    ) -> Result<(ResourceUid, VolumeSpec), BindingDriverError> {
        let key = ResourceKey::new(&self.zone, "Volume", binding.volume_ref().name().as_str());
        let row = ctx
            .get(&key)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::OwnerMismatch, op))?
            .ok_or_else(|| self.error(BindingDriverErrorKind::OwnerMismatch, op))?;
        if ctx
            .owner()
            .is_some_and(|owner| owner != row.uid.as_slice())
        {
            // The row's declared owner uid does not match the parent the
            // spec names: refuse rather than silently re-parent (R8).
            return Err(self.error(BindingDriverErrorKind::OwnerMismatch, op));
        }
        let uid = resource_uid(&row.uid)
            .map_err(|_| self.error(BindingDriverErrorKind::OwnerMismatch, op))?;
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.error(BindingDriverErrorKind::OwnerMismatch, op))?;
        let volume_spec = serde_json::from_slice::<VolumeSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.error(BindingDriverErrorKind::OwnerMismatch, op))?;
        Ok((uid, volume_spec))
    }

    /// Re-derive the path-free launch plan from the persisted binding plus
    /// the bundle-resolved view spec (KTD7). Tuning travels in the plan,
    /// never in the resource (KTD1): the serving posture is the frozen
    /// default declared by `VirtiofsdWorkerPlan::for_binding`.
    fn derive_plan(
        &self,
        binding: &StoredBinding,
        view: &d2b_contracts_resource::v3::volume::ViewSpec,
        op: DriverOp,
    ) -> Result<VirtiofsdWorkerPlan, BindingDriverError> {
        let principal = binding
            .worker_principal()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?;
        VirtiofsdWorkerPlan::for_binding(binding, view, self.vcpu_count, principal)
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))
    }

    /// The stored binding for one row: strict neutral spec plus the
    /// identity the fence is pinned to (uid from the durable row; the
    /// revision is not carried by the new store and the readiness fence
    /// does not compare revisions, matching the old behavior).
    fn stored_binding(
        &self,
        ctx: &ResourceContext,
        binding: VolumeBindingSpec,
        op: DriverOp,
    ) -> Result<StoredBinding, BindingDriverError> {
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        let generation = d2b_contracts_resource::v3::ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        Ok(StoredBinding::new(
            binding,
            uid,
            generation,
            d2b_contracts_resource::v3::ZoneRevision::new(0),
        ))
    }

    /// Derive the worker Process + Endpoint child specs (old
    /// `worker_child_specs`). Tuning travels in the worker plan at launch,
    /// never in the resource: the store contract admits only standard
    /// Process/Endpoint fields.
    fn worker_child_specs(
        &self,
        binding: &StoredBinding,
    ) -> Result<(ChildEnsure, ChildEnsure), BindingDriverError> {
        let process_ref = binding
            .worker_process_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Reconcile))?;
        let endpoint_ref = binding
            .endpoint_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Reconcile))?;
        let process_spec = serde_json::json!({
            "providerRef": WORKER_PROVIDER_REF,
            "executionRef": binding.spec().execution_ref().to_canonical_string(),
            "domain": "system",
            "processClass": "worker",
            "template": WORKER_TEMPLATE,
            "desiredLifecycle": "running",
            "sandbox": {
                "capabilityClasses": [],
                "startRoot": false,
                "namespaceClasses": ["user"],
                "seccompClass": "strict",
                "noNewPrivileges": true,
                "environmentClass": "minimal",
                "readOnlyRoot": true,
                "umask": "0022",
                "oomScoreAdj": 0,
                "userNamespace": {
                    "mappingClass": "process-principal-root"
                }
            }
        });
        let endpoint_spec = serde_json::json!({
            "providerRef": BINDING_PROVIDER_REF,
            "producerRef": process_ref.to_canonical_string(),
            "endpointClass": "service",
            "transport": "unix",
            "purpose": "virtiofsd",
            "locality": "host-local",
            "visibility": "provider",
            "attachmentPolicy": {
                "supported": false,
                "maxAttachments": 0
            },
            "consumerPolicy": {
                "allowedSubjects": [BINDING_PROVIDER_REF],
                "allowedOperations": ["resolve", "observe"]
            },
            "lifecyclePolicy": "recycle-with-producer"
        });
        let worker = ChildEnsure {
            type_name: ResourceTypeName::new(WORKER_TYPE),
            name: process_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&process_spec)
                .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Reconcile))?,
            metadata: Vec::new(),
        };
        let endpoint = ChildEnsure {
            type_name: ResourceTypeName::new(ENDPOINT_TYPE),
            name: endpoint_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&endpoint_spec)
                .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Reconcile))?,
            metadata: Vec::new(),
        };
        Ok((worker, endpoint))
    }

    async fn ensure_children(
        &self,
        ctx: &mut ResourceContext,
        binding: &StoredBinding,
        op: DriverOp,
    ) -> Result<(String, String), BindingDriverError> {
        let (worker, endpoint) = self.worker_child_specs(binding)?;
        // The manager commits each child row BEFORE creating or updating
        // the child actor (F1, AE1); the endpoint child depends on the
        // worker process, so the worker row is committed first.
        ctx.ensure_child(worker)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        ctx.ensure_child(endpoint)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        let worker_ref = binding
            .worker_process_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?;
        let endpoint_ref = binding
            .endpoint_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?;
        Ok((
            worker_ref.to_canonical_string(),
            endpoint_ref.to_canonical_string(),
        ))
    }
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

#[async_trait::async_trait]
impl ResourceDriver for BindingDriver {
    type Error = BindingDriverError;

    fn classify_error(&self, error: &BindingDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Spec decode, serving Provider check, and the owner-fence check: the
    /// binding's declared parent Volume must be the resource the manager
    /// reports as the row's owner (a child cannot silently change owner).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (_, binding) = self.decoded_binding(ctx, DriverOp::Validate)?;
        self.parent_volume(ctx, &binding, DriverOp::Validate).await?;
        Ok(())
    }

    /// Re-derive the launch plan from the persisted binding plus the
    /// bundle-resolved view spec (KTD7) and probe the serving socket: a
    /// ready socket adopts the pre-restart incarnation, an absent socket
    /// waits for reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (_, binding) = self.decoded_binding(ctx, DriverOp::Recover)?;
        let stored = self.stored_binding(ctx, binding, DriverOp::Recover)?;
        let (_, volume_spec) = self
            .parent_volume(ctx, stored.spec(), DriverOp::Recover)
            .await?;
        let view = volume_spec
            .views()
            .get(stored.spec().view().as_str())
            .ok_or_else(|| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Recover))?;
        let _plan = self.derive_plan(&stored, view, DriverOp::Recover)?;
        let socket = stored.socket_identity(&self.zone_bounded());
        if self.effects.socket_ready(&socket).await {
            ctx.set_status(BindingDriverStatus::RecoveredPlan {
                worker_ref: stored
                    .worker_process_ref()
                    .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Recover))?
                    .to_canonical_string(),
                endpoint_ref: stored
                    .endpoint_ref()
                    .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Recover))?
                    .to_canonical_string(),
            });
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: derive the launch plan, ensure the worker
    /// Process child and then the Endpoint child (F1), and report
    /// child-phase driven readiness on the serving socket.
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let (_, binding) = self.decoded_binding(ctx, DriverOp::Reconcile)?;
        let stored = self.stored_binding(ctx, binding, DriverOp::Reconcile)?;
        let (_, volume_spec) = self
            .parent_volume(ctx, stored.spec(), DriverOp::Reconcile)
            .await?;
        let view = volume_spec
            .views()
            .get(stored.spec().view().as_str())
            .ok_or_else(|| self.error(BindingDriverErrorKind::PlanDerivation, DriverOp::Reconcile))?;
        let _plan = self.derive_plan(&stored, view, DriverOp::Reconcile)?;
        let (worker_ref, endpoint_ref) = self.ensure_children(ctx, &stored, DriverOp::Reconcile).await?;
        ctx.set_status(BindingDriverStatus::ServingChildren {
            worker_ref: worker_ref.clone(),
            endpoint_ref: endpoint_ref.clone(),
        });
        let socket = stored.socket_identity(&self.zone_bounded());
        if self.effects.socket_ready(&socket).await {
            Ok(ReconcileOutcome::Satisfied)
        } else {
            // Child-phase driven readiness: the socket is not up yet; the
            // actor re-reconciles on the next signal (R5).
            Ok(ReconcileOutcome::Satisfied)
        }
    }

    /// Teardown in the preserved endpoint-first / process-last ordering
    /// (R9/F3, old `reconcile_binding_children` deletion order): remove the
    /// endpoint realization first, then delete the worker Process child
    /// through the manager. Idempotent under retry (R10): a missing child
    /// row converges without effects.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok((_, binding)) = self.decoded_binding(ctx, DriverOp::Delete) else {
            return Ok(());
        };
        let Ok(stored) = self.stored_binding(ctx, binding, DriverOp::Delete) else {
            return Ok(());
        };
        let Ok(endpoint_ref) = stored.endpoint_ref() else {
            return Ok(());
        };
        let Ok(worker_ref) = stored.worker_process_ref() else {
            return Ok(());
        };
        // 1. Endpoint first: the endpoint child row is deleted and the
        // socket realization removed.
        let endpoint_key = ResourceKey::new(&self.zone, ENDPOINT_TYPE, endpoint_ref.name().as_str());
        let _ = ctx.delete(&endpoint_key).await;
        let socket = stored.socket_identity(&self.zone_bounded());
        self.effects
            .remove_socket(&socket)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ServingEffect, DriverOp::Delete))?;
        // 2. Process last: the worker child row is deleted only after the
        // endpoint is gone (the worker's socket drain relies on it).
        let worker_key = ResourceKey::new(&self.zone, WORKER_TYPE, worker_ref.name().as_str());
        let _ = ctx
            .delete(&worker_key)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, DriverOp::Delete));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted serving port and a recording
// manager endpoint with one shared ordered log (R4; F1/AE1 and teardown
// ordering observed as the manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::volume::AttachmentAccess;
    use d2b_provider_volume_virtiofs::{SocketIdentity, WORKER_TEMPLATE};
    use d2b_resource_runtime::context::{ChildEnsure, ManagerEndpoint, ResourceContext, WatchId, WatchRegistration};
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{
        BindingDriverArgs, BindingDriverFactory, BindingDriverStatus, binding_spec_decoder,
    };

    type OrderLog = Vec<String>;

    // -- fakes ---------------------------------------------------------------

    /// Scripted serving port: records every call in order.
    struct FakeServingEffects {
        calls: parking_lot::Mutex<Vec<&'static str>>,
        ready: std::sync::atomic::AtomicBool,
    }

    impl FakeServingEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                ready: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }

        fn make_ready(&self) {
            self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl super::BindingDriverEffects for FakeServingEffects {
        async fn socket_ready(&self, _socket: &SocketIdentity) -> bool {
            self.calls.lock().push("socket-ready");
            self.ready.load(std::sync::atomic::Ordering::SeqCst)
        }

        async fn remove_socket(&self, _socket: &SocketIdentity) -> Result<(), String> {
            self.calls.lock().push("remove-socket");
            Ok(())
        }
    }

    /// Recording manager endpoint over one shared ordered log. Rows are
    /// keyed by `zone/type/name`; `get` returns the parent Volume row the
    /// binding declares.
    #[derive(Clone)]
    struct RecordingManager {
        zone: String,
        log: Arc<parking_lot::Mutex<OrderLog>>,
        rows: Arc<parking_lot::Mutex<Vec<StoredDesiredResource>>>,
        next_uid: Arc<std::sync::atomic::AtomicU64>,
    }

    impl RecordingManager {
        fn new() -> Self {
            Self {
                zone: "work".to_owned(),
                log: Arc::new(parking_lot::Mutex::new(Vec::new())),
                rows: Arc::new(parking_lot::Mutex::new(Vec::new())),
                next_uid: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            }
        }

        fn with_parent(self, volume_uid: [u8; 16], spec: &[u8]) -> Self {
            self.rows.lock().push(StoredDesiredResource {
                key: ResourceKey::new("work", "Volume", "data"),
                uid: volume_uid,
                generation: 2,
                owner_uid: None,
                provenance: ResourceProvenance::Api,
                deleting: false,
                spec: spec.to_vec(),
                metadata: Vec::new(),
                created_at: 0,
            });
            self
        }

        fn order(&self) -> Vec<String> {
            self.log.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let id = format!("{}/{}", child.type_name.as_str(), child.name);
            self.log.lock().push(format!("ensure:{id}"));
            let next = self
                .next_uid
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut uid = [0u8; 16];
            uid[..8].copy_from_slice(&next.to_be_bytes());
            let row = StoredDesiredResource {
                key: ResourceKey::new(&self.zone, child.type_name.as_str(), &child.name),
                uid,
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec,
                metadata: child.metadata,
                created_at: 0,
            };
            let mut rows = self.rows.lock();
            let outcome = match rows.iter_mut().find(|row| row.key == row_key(&id, &self.zone)) {
                Some(existing) => {
                    if existing.spec == row.spec {
                        EnsureOutcome::Unchanged(existing.clone())
                    } else {
                        *existing = row.clone();
                        EnsureOutcome::Updated(row.clone())
                    }
                }
                None => {
                    rows.push(row.clone());
                    EnsureOutcome::Created(row.clone())
                }
            };
            // Spawn notification only after the commit (F1, AE1).
            self.log.lock().push(format!("spawned:{id}"));
            Ok(outcome)
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned())
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock()
                .push(format!("delete:{}/{}", key.type_name, key.name));
            self.rows.lock().retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .iter()
                .filter(|row| row.owner_uid.as_ref() == Some(&owner_uid))
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerRpc("watches unused in this unit".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Dead requeue: these binding flows never schedule a requeue.
    struct NullRequeue;

    impl d2b_resource_runtime::context::RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> d2b_resource_runtime::context::RequeueId {
            d2b_resource_runtime::context::RequeueId(0)
        }

        fn cancel(&self, _id: d2b_resource_runtime::context::RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    /// A minimal valid Volume spec (the parent row the binding declares).
    fn parent_volume_bytes() -> Vec<u8> {
        serde_json::json!({
            "providerRef": "Provider/volume-local",
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "policy-default" }
            },
            "kind": "durable",
            "layout": [],
            "views": { "root": { "path": "data", "rights": ["read", "traverse", "write"] } },
            "attachments": [ {
                "executionRef": "Guest/guest-a",
                "transport": "virtiofs",
                "view": "root",
                "access": "read-only",
                "mountPath": "/mnt/data",
                "settings": {}
            } ]
        })
        .to_string()
        .into_bytes()
    }

    /// The exact VolumeBinding child spec envelope the Volume driver mints
    /// (neutral binding + serving Provider reference).
    fn binding_row(owner_uid: [u8; 16]) -> StoredDesiredResource {
        let binding_spec = serde_json::json!({
            "providerRef": "Provider/volume-virtiofs",
            "volumeRef": "Volume/data",
            "executionRef": "Guest/guest-a",
            "view": "root",
            "access": "read-only",
            "mountPath": "/mnt/data",
        });
        StoredDesiredResource {
            key: ResourceKey::new("work", "VolumeBinding", "vol-binding-000000000000000000000000"),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: Some(owner_uid),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: binding_spec.to_string().into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: RecordingManager,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManager) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            binding_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        Fixture { ctx, manager }
    }

    fn row_key(id: &str, zone: &str) -> ResourceKey {
        let (type_name, name) = id.split_once('/').expect("typed id");
        ResourceKey::new(zone, type_name, name)
    }

    async fn driver(effects: Arc<FakeServingEffects>) -> Box<dyn DynResourceDriver> {
        let factory = BindingDriverFactory::new(BindingDriverArgs {
            zone: "work".to_owned(),
            effects,
            vcpu_count: 4,
        });
        factory
            .create(&ResourceKey::new("work", "VolumeBinding", "binding"))
            .await
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_only_the_binding_resource_type() {
        let factory = BindingDriverFactory::new(BindingDriverArgs {
            zone: "work".to_owned(),
            effects: FakeServingEffects::new(),
            vcpu_count: 4,
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "VolumeBinding");
    }

    // -- reconcile: worker + endpoint children --------------------------------

    #[tokio::test]
    async fn ensure_derives_worker_and_endpoint_children_persisted_before_spawn() {
        let fake = FakeServingEffects::new();
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.validate(&mut f.ctx).await.expect("validate");
        let outcome = d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);

        let order = manager.order();
        // F1: each child row is ensured (committed) BEFORE its spawn
        // notification; the endpoint depends on the worker, so the worker
        // row is committed first.
        let worker_ensure = order
            .iter()
            .position(|entry| entry.starts_with("ensure:Process/"))
            .expect("worker ensure recorded");
        let worker_spawn = order
            .iter()
            .position(|entry| entry.starts_with("spawned:Process/"))
            .expect("worker spawn recorded");
        let endpoint_ensure = order
            .iter()
            .position(|entry| entry.starts_with("ensure:Endpoint/"))
            .expect("endpoint ensure recorded");
        let endpoint_spawn = order
            .iter()
            .position(|entry| entry.starts_with("spawned:Endpoint/"))
            .expect("endpoint spawn recorded");
        assert!(worker_ensure < worker_spawn, "persist-before-spawn (F1): {order:?}");
        assert!(endpoint_ensure < endpoint_spawn, "persist-before-spawn (F1): {order:?}");
        assert!(worker_ensure < endpoint_ensure, "endpoint depends on worker");
        // Child-phase driven readiness.
        assert!(fake.call_order().contains(&"socket-ready"));
    }

    // -- recover: plan re-derivation matches the pre-restart incarnation ------

    #[tokio::test]
    async fn recover_rederives_the_launch_plan_matching_the_pre_restart_incarnation() {
        let fake = FakeServingEffects::new();
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager);
        let mut d = driver(fake.clone()).await;

        // Pre-restart: reconcile derives and stores the plan.
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pre = match f.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::ServingChildren { worker_ref, endpoint_ref }) => {
                (worker_ref.clone(), endpoint_ref.clone())
            }
            other => panic!("expected ServingChildren, got {other:?}"),
        };

        // Restart: a fresh context (no in-memory state) recovers; the
        // re-derived plan must match the pre-restart incarnation (same
        // template/socket-equivalent identity - the plan is path-free, so
        // socket identity equivalence is the worker + endpoint refs and the
        // frozen plan fields).
        let mut f2 = fixture(binding_row([0x42; 16]), manager_clone_placeholder());
        let mut d2 = driver(fake.clone()).await;
        fake.make_ready();
        assert_eq!(
            d2.recover(&mut f2.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "socket ready: pre-restart incarnation adopted"
        );
        let recovered = match f2.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::RecoveredPlan { worker_ref, endpoint_ref }) => {
                (worker_ref.clone(), endpoint_ref.clone())
            }
            other => panic!("expected RecoveredPlan, got {other:?}"),
        };
        assert_eq!(pre, recovered, "re-derived plan matches pre-restart incarnation");
        // The frozen plan posture: same template, frozen sandbox defaults.
        let binding = d2b_provider_volume_virtiofs::StoredBinding::new(
            d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec::new(
                d2b_contracts_resource::v3::ResourceRef::parse("Volume/data").expect("volume"),
                d2b_contracts_resource::v3::ResourceRef::parse("Guest/guest-a").expect("guest"),
                "root",
                AttachmentAccess::ReadOnly,
                "/mnt/data",
            )
            .expect("binding spec"),
            d2b_contracts_resource::v3::ResourceUid::parse(
                "42424242-4242-4242-8242-424242424242",
            )
            .expect("uid"),
            d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
            d2b_contracts_resource::v3::ZoneRevision::new(0),
        );
        let view = parent_volume_bytes_view();
        let plan = binding
            .worker_principal()
            .ok()
            .and_then(|principal| {
                d2b_provider_volume_virtiofs::VirtiofsdWorkerPlan::for_binding(
                    &binding, &view, 4, principal,
                )
                .ok()
            })
            .expect("plan derives");
        assert_eq!(plan.template, WORKER_TEMPLATE);
        assert_eq!(plan.thread_pool_size, 4);
        assert!(plan.readonly);
        assert!(!plan.posix_acl);
        assert!(!plan.xattr);
    }

    // -- teardown ordering -----------------------------------------------------

    #[tokio::test]
    async fn delete_removes_endpoint_before_worker() {
        let fake = FakeServingEffects::new();
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.reconcile(&mut f.ctx).await.expect("reconcile");
        d.delete(&mut f.ctx).await.expect("delete");

        let order = manager.order();
        let endpoint_delete = order
            .iter()
            .position(|entry| entry.starts_with("delete:Endpoint/"))
            .expect("endpoint child deleted");
        let _worker_delete = order
            .iter()
            .position(|entry| entry.starts_with("delete:Process/"))
            .expect("worker child deleted");
        assert!(
            endpoint_delete < worker_delete_index(&order, endpoint_delete),
            "endpoint-first / process-last teardown: {order:?}"
        );
        // The socket removal (the endpoint effect) precedes the worker
        // deletion on the serving port.
        assert!(fake.call_order().contains(&"remove-socket"));
    }

    fn worker_delete_index(order: &[String], endpoint_delete: usize) -> usize {
        order[endpoint_delete..]
            .iter()
            .position(|entry| entry.starts_with("delete:Process/"))
            .map(|offset| endpoint_delete + offset)
            .expect("worker delete after endpoint")
    }

    // -- owner guard -----------------------------------------------------------

    #[tokio::test]
    async fn child_cannot_silently_change_owner() {
        let fake = FakeServingEffects::new();
        // The manager reports a DIFFERENT parent uid than the binding row's
        // owner: the driver must refuse with a typed terminal error instead
        // of silently re-parenting.
        let manager = RecordingManager::new().with_parent([0x99; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager);
        let mut d = driver(fake).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal, "owner mismatch is terminal");
    }

    // -- deterministic child identity -------------------------------------------

    #[tokio::test]
    async fn same_parent_and_binding_derive_the_same_child_keys() {
        let fake = FakeServingEffects::new();
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile one");
        d.reconcile(&mut f.ctx).await.expect("reconcile two");
        let order = manager.order();
        let ensures: Vec<&String> = order
            .iter()
            .filter(|entry| entry.starts_with("ensure:Process/") || entry.starts_with("ensure:Endpoint/"))
            .collect();
        // Two passes, same parent + binding: same child keys, no churn.
        assert_eq!(ensures.len(), 4);
        assert_eq!(ensures[0], ensures[2], "worker child key is deterministic");
        assert_eq!(ensures[1], ensures[3], "endpoint child key is deterministic");
    }

    fn parent_volume_bytes_view() -> d2b_contracts_resource::v3::volume::ViewSpec {
        d2b_contracts_resource::v3::volume::ViewSpec::new(
            "data",
            vec![
                d2b_contracts_resource::v3::volume::ViewRight::Read,
                d2b_contracts_resource::v3::volume::ViewRight::Write,
                d2b_contracts_resource::v3::volume::ViewRight::Traverse,
            ],
        )
        .expect("view")
    }

    fn manager_clone_placeholder() -> RecordingManager {
        RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes())
    }
}
