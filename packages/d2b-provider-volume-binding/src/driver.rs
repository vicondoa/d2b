//! The VolumeBinding resource driver: the v3 `ResourceDriver` conversion of
//! the binding leg of the shared-Volume path.
//!
//! The driver owns the whole binding family: the derived
//! [`VirtiofsdWorkerPlan`] is the serving authority (a binding the frozen
//! plan contract rejects is refused with the stable provider reason, the plan
//! travels in the in-memory status and never in a resource, and the worker
//! Process child is minted argv-free so the Process controller composes its
//! launch from the binding and Volume rows), reconcile derives the
//! binding-owned worker Process and Endpoint children, ensures each through
//! the manager-routed ensure (the child spec is committed BEFORE the child
//! actor exists), retires owned children the derived set no longer names
//! endpoint-first / process-last, registers the dependency watches, and
//! publishes the in-memory status.
//!
//! Delete preserves the drain/finalizer semantics: the guest mount is
//! observed BEFORE anything is deleted, and a present mount blocks the
//! teardown - retryable, durable deleting mark and owned children still in
//! place - instead of force-clearing a served share. Otherwise the worker
//! drains, the Endpoint is removed first and the worker Process child last,
//! and the manager holds the parent row until the last child retires.
//!
//! Recover re-derives the launch plan from the persisted binding plus the
//! bundle-resolved view spec, so the re-derived plan matches the pre-restart
//! incarnation, and adopts the owned-child realization (both child rows
//! current and the serving socket listening).
//!
//! Conversion mapping:
//! - `describe` -> [`binding_descriptor`] registration under `VolumeBinding`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - `binding_children` minting + readiness -> [`ResourceDriver::reconcile`].
//! - `finalize_binding` drain + endpoint-first teardown -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only).
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`BindingDriverEffects`]): the serving socket probe, the
//! socket removal, and the guest-mount observation the daemon realizes. The
//! production implementation lives in the daemon behind that port, so the
//! family carries no host state. The zone-authority inputs (target Guest
//! vcpu count for the worker thread pool) are factory wiring, folded by the
//! plane from the bundle resolver and the Zone authority path, never from the
//! spec store.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceSpec, ResourceUid,
    volume::{VolumeSpec, ViewSpec},
    volume_binding::VolumeBindingSpec,
};
use d2b_provider_volume_virtiofs::{
    StoredBinding, VirtiofsBindingError, VirtiofsdWorkerPlan, WORKER_TEMPLATE,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, ChildCustody, DriverDescriptor,
    WellKnownType,
};

use crate::effects_service::BINDING_EFFECTS_SERVICE;

/// The one resource type this factory serves.
pub const BINDING_TYPE_NAME: &str = "VolumeBinding";

/// The serving Provider this driver owns.
pub(crate) const BINDING_PROVIDER_REF: &str = d2b_provider_volume_virtiofs::PROVIDER_REF;

/// The Process Provider the binding-owned worker runs under (old
/// `worker_child_specs`).
pub(crate) const WORKER_PROVIDER_REF: &str = d2b_provider_process_minijail::PROVIDER_REF;

/// Deterministic owned-child resource types.
const WORKER_TYPE: &str = "Process";
const ENDPOINT_TYPE: &str = "Endpoint";

/// The children this driver mints, in creation rank order: the worker
/// Process first, because the Endpoint names it as producer and a realized
/// producer must exist before the socket that serves it, then that Endpoint.
/// Children retire in descending rank order, which is the preserved
/// endpoint-first / process-last teardown.
pub const BINDING_CREATIONS: &[ChildCreation] = &[
    ChildCreation {
        child: WellKnownType::PROCESS,
        provider_ref: WORKER_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 0,
    },
    ChildCreation {
        child: WellKnownType::ENDPOINT,
        provider_ref: BINDING_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 1,
    },
];

/// Preserved resync cadence while the derived child set is not yet current:
/// the `volume-virtiofs` Runner contract's repair interval (the old runner's
/// resync), not a per-pass timer.
const BINDING_RESYNC: Duration = Duration::from_secs(
    d2b_provider_volume_virtiofs::virtiofs_runner_contract().repair_interval_secs,
);

/// Teardown rank of one owned-child type, read from this family's own child
/// declaration (R9/F3): the Endpoint is removed before the worker Process
/// that produces it, and a type this driver declares no creation for has no
/// rank at all, so it retires last.
fn teardown_rank(type_name: &str) -> Option<u16> {
    BINDING_CREATIONS
        .iter()
        .find(|creation| creation.child.to_resource_type_name().as_str() == type_name)
        .map(|creation| creation.order)
}

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding contract.
    SpecInvalid,
    /// The spec selects a Provider this driver does not own.
    ProviderUnsupported,
    /// The binding's declared parent Volume row is present but its owner uid
    /// differs from this binding's owner: the manager would silently
    /// re-parent. Terminal - the committed rows cannot converge by retrying.
    OwnerMismatch,
    /// The parent Volume row the binding names is not observable yet: the
    /// manager answered `Absent` (the row may simply not be committed yet) or
    /// could not answer at all. Retryable by contract (issue #511): the actor
    /// requeues instead of failing the binding terminal.
    ParentUnavailable,
    /// The parent Volume row is present but not a usable Volume row (its uid
    /// or stored spec does not decode). Terminal: the committed row cannot
    /// converge by retrying.
    ParentSpecInvalid,
    /// The worker plan could not be derived (view rights, zero vcpu).
    PlanDerivation,
    /// Owned children are still retiring before this binding may drain.
    DrainPending,
    /// A provider serving effect failed transiently.
    ServingEffect,
    /// The manager refused a child ensure/delete.
    ChildMutation,
}

impl BindingDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ServingEffect
            | Self::ChildMutation
            | Self::ParentUnavailable
            | Self::DrainPending => FailureClass::Retryable,
            Self::SpecInvalid
            | Self::ProviderUnsupported
            | Self::OwnerMismatch
            | Self::ParentSpecInvalid
            | Self::PlanDerivation => FailureClass::Terminal,
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::BINDING_SPEC_INVALID,
            Self::ProviderUnsupported => FailureKinds::BINDING_PROVIDER_UNSUPPORTED,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::ParentUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::ParentSpecInvalid => FailureKinds::BINDING_PARENT_SPEC_INVALID,
            Self::PlanDerivation => FailureKinds::BINDING_PLAN_DERIVATION_INVALID,
            // The shared child-first-teardown kind: draining is not a child
            // mutation.
            Self::DrainPending => FailureKinds::CHILDREN_DRAINING,
            Self::ServingEffect => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
            Self::ChildMutation => FailureKinds::BINDING_CHILD_MUTATION_FAILED,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct BindingDriverError {
    kind: BindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl BindingDriverError {
    fn new(kind: BindingDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for BindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            BindingDriverErrorKind::SpecInvalid => "binding-spec-invalid",
            BindingDriverErrorKind::ProviderUnsupported => "binding-provider-unsupported",
            BindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            BindingDriverErrorKind::ParentUnavailable => "binding-parent-unavailable",
            BindingDriverErrorKind::ParentSpecInvalid => "binding-parent-spec-invalid",
            BindingDriverErrorKind::PlanDerivation => "binding-plan-derivation-invalid",
            BindingDriverErrorKind::DrainPending => "children-draining",
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
    /// Worker + Endpoint children derived and ensured; readiness as observed
    /// this pass (worker socket + guest mount, both fail-closed).
    ServingChildren {
        plan: DerivedPlan,
        /// The derived child set was already current (no ensure/create or
        /// obsolete retirement mutated it); the manager requeues otherwise.
        converged: bool,
        socket_ready: bool,
        mount_ready: bool,
    },
    /// The exact pre-restart plan was re-derived on recover and the owned
    /// children plus the serving socket are current.
    RecoveredPlan {
        plan: DerivedPlan,
        socket_ready: bool,
    },
    /// A terminal admission rejection (old `failed_binding_result`): the
    /// stable provider reason stays visible in memory while the actor
    /// publishes the Failed phase (KTD5).
    Rejected { reason: &'static str },
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
pub fn binding_spec_decoder() -> Arc<dyn SpecDecoder> {
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
pub trait BindingDriverEffects: Send + Sync + 'static {
    /// Whether the worker's private socket is ready (child-phase evidence).
    async fn socket_ready(&self, socket: &d2b_provider_volume_virtiofs::SocketIdentity)
        -> bool;

    /// Remove the endpoint realization (socket) - endpoint-first teardown.
    async fn remove_socket(
        &self,
        socket: &d2b_provider_volume_virtiofs::SocketIdentity,
    ) -> Result<(), String>;

    /// Guest mount observation (the KTD6 drain gate): whether the target
    /// Guest currently observes the mount.
    ///
    /// The evidence is the target layer's, never a second channel (U13): the
    /// owning row's assignment in the Zone target directory is asked through
    /// the live authenticated ComponentSession, and only a target-local
    /// realization the Guest reports `ready` for that source answers `true`.
    /// A target the directory cannot reach, a loose row, and a source the
    /// Guest holds no realization for all answer `false` - the same
    /// fail-closed answer the old plane gave while the Endpoint child had no
    /// published state. A present mount therefore keeps the durable deleting
    /// mark and the owned children (the drain never force-clears a serve that
    /// is still mounted).
    async fn guest_mount_ready(
        &self,
        _key: &ResourceKey,
        _binding: &StoredBinding,
    ) -> Result<bool, String> {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the binding driver
/// factory for one zone: the serving effects plus the zone-authority inputs
/// (the target Guest vcpu count resolved by the bundle resolver, never read
/// from the spec store).
pub struct BindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: String,
    /// The daemon-supplied facet set the family's effects are built from
    /// (R2): the serving-socket probe, the socket removal, and the
    /// guest-mount observation. The family never receives a daemon-built
    /// effect port.
    pub facets: crate::facets::BindingEffectFacets,
    /// Target Guest vcpu count; the worker thread-pool size.
    pub vcpu_count: u32,
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
        Box::new(BindingDriver::new(
            self.args.zone.clone(),
            // The driver builds its effects from the declared facets; no
            // externally built port appears at this construction site (R2).
            Arc::new(crate::effects_service::BindingEffectsService::new(
                self.args.facets.clone(),
            )),
            self.args.vcpu_count,
        ))
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
    /// Targets this driver already registered a dependency watch on
    /// (R12/R17). Runtime-only (R6/R11): one registration per target keeps
    /// the dependency edge that wakes the actor on dependency death or
    /// readiness without accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl BindingDriver {
    pub(crate) fn new(
        zone: String,
        effects: Arc<dyn BindingDriverEffects>,
        vcpu_count: u32,
    ) -> Self {
        Self {
            zone,
            effects,
            vcpu_count,
            watched: Vec::new(),
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
            Some(provider_ref) if provider_ref.to_canonical_string() == BINDING_PROVIDER_REF => {}
            other => {
                return Err(self
                    .error(BindingDriverErrorKind::ProviderUnsupported, op)
                    .with_detail(
                        FailureDetail::at("spec/provider").comparison(FailureComparison::new(
                            "spec.providerRef",
                            BINDING_PROVIDER_REF,
                            other
                                .map(|reference| reference.to_canonical_string())
                                .unwrap_or_else(|| "absent".to_owned()),
                        )),
                    ))
            }
        }
        let binding = serde_json::from_slice::<VolumeBindingSpec>(
            &envelope.base.to_canonical_bytes(),
        )
        .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope.clone(), binding))
    }

    /// The key of the parent Volume this binding declares.
    fn parent_volume_key(&self, binding: &VolumeBindingSpec) -> ResourceKey {
        ResourceKey::new(&self.zone, "Volume", binding.volume_ref().name().as_str())
    }

    /// The parent Volume row through the manager (R2: the driver never
    /// touches the spec store). The binding's declared Volume must be the
    /// row the manager reports as this resource's owner - a child cannot
    /// silently change owner.
    ///
    /// Classified per issue #511: a row that is not observable yet (`Absent`,
    /// `Unavailable`, or an `Error` read the manager could not answer with a
    /// usable row) defers retryably, so a parent that simply has not been
    /// committed yet never fails the binding terminal; `OwnerMismatch`
    /// applies only to a present row whose owner uid actually differs.
    async fn parent_volume(
        &self,
        ctx: &mut ResourceContext,
        binding: &VolumeBindingSpec,
        op: DriverOp,
    ) -> Result<(ResourceUid, VolumeSpec), BindingDriverError> {
        let key = self.parent_volume_key(binding);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                // A non-present read defers: the row may not be committed yet
                // and an unusable payload is not terminal by itself (#511).
                let mut detail = FailureDetail::at("parent/lookup");
                if let Some(comparison) = lookup.failure_comparison("parent.volume", "present") {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                if let RowLookup::Error { plane, detail: error_detail } = &lookup {
                    tracing::warn!(
                        plane = ?plane,
                        key = %key,
                        detail = %error_detail,
                        "binding parent row read answered with an unreadable row",
                    );
                }
                return Err(self
                    .error(BindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(detail));
            }
        };
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            // The row's declared owner uid does not match the parent the
            // spec names: refuse rather than silently re-parent (R8).
            return Err(self
                .error(BindingDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("parent/owner").comparison(
                    FailureComparison::new("parent.ownerUid", uid_hex(owner), uid_hex(&row.uid)),
                )));
        }
        let uid = resource_uid(&row.uid).map_err(|_| {
            self.parent_spec_invalid(op)
                .with_detail(Self::parent_row_detail("parent.uid"))
        })?;
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec).map_err(|_| {
            self.parent_spec_invalid(op)
                .with_detail(Self::parent_row_detail("parent.spec"))
        })?;
        let volume_spec = serde_json::from_slice::<VolumeSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| {
                self.parent_spec_invalid(op)
                    .with_detail(Self::parent_row_detail("parent.spec"))
            })?;
        Ok((uid, volume_spec))
    }

    /// The terminal classification for a present parent row whose stored
    /// identity or spec does not decode (issue #508: this is not an ownership
    /// mismatch).
    fn parent_spec_invalid(&self, op: DriverOp) -> BindingDriverError {
        self.error(BindingDriverErrorKind::ParentSpecInvalid, op)
    }

    /// The comparison naming which part of the parent row was unusable.
    fn parent_row_detail(field: &'static str) -> FailureDetail {
        FailureDetail::at("parent/decode")
            .comparison(FailureComparison::new(field, "a canonical Volume row", "decode failed"))
    }

    /// Record one terminal admission rejection in this pass's in-memory
    /// status (old `failed_binding_result`) and return the typed failure: the
    /// actor publishes the Failed phase, and the stable provider reason stays
    /// visible instead of collapsing into a generic error (KTD5).
    fn rejected(
        &self,
        ctx: &mut ResourceContext,
        reason: &'static str,
        op: DriverOp,
    ) -> BindingDriverError {
        ctx.set_status(BindingDriverStatus::Rejected { reason });
        self.error(BindingDriverErrorKind::PlanDerivation, op)
            .with_detail(FailureDetail::at("plan/derive").with_note(reason))
    }

    /// Re-derive the path-free launch plan from the persisted binding plus
    /// the bundle-resolved view spec (KTD7). Tuning travels in the plan,
    /// never in the resource (KTD1): the serving posture is the frozen
    /// default declared by `VirtiofsdWorkerPlan::for_binding`. A plan the
    /// frozen contract rejects is terminal, and its reason is preserved.
    fn derive_plan(
        &self,
        ctx: &mut ResourceContext,
        binding: &StoredBinding,
        view: &ViewSpec,
        op: DriverOp,
    ) -> Result<VirtiofsdWorkerPlan, BindingDriverError> {
        let principal = binding
            .worker_principal()
            .map_err(|reason| self.rejected(ctx, reason.code(), op))?;
        VirtiofsdWorkerPlan::for_binding(binding, view, self.vcpu_count, principal)
            .map_err(|reason| self.rejected(ctx, reason.code(), op))
    }

    /// The status handle carrying the exact re-derived plan (KTD7).
    fn derived_plan(
        &self,
        stored: &StoredBinding,
        plan: VirtiofsdWorkerPlan,
        op: DriverOp,
    ) -> Result<DerivedPlan, BindingDriverError> {
        Ok(DerivedPlan {
            worker_ref: stored
                .worker_process_ref()
                .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?
                .to_canonical_string(),
            endpoint_ref: stored
                .endpoint_ref()
                .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?
                .to_canonical_string(),
            plan,
        })
    }

    /// The desired owned-child keys: worker Process first, then its Endpoint.
    fn desired_child_keys(
        &self,
        stored: &StoredBinding,
        op: DriverOp,
    ) -> Result<[ResourceKey; 2], BindingDriverError> {
        let worker = stored
            .worker_process_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?;
        let endpoint = stored
            .endpoint_ref()
            .map_err(|_| self.error(BindingDriverErrorKind::PlanDerivation, op))?;
        Ok([
            ResourceKey::new(&self.zone, WORKER_TYPE, worker.name().as_str()),
            ResourceKey::new(&self.zone, ENDPOINT_TYPE, endpoint.name().as_str()),
        ])
    }

    /// Register one dependency watch (R12/R17) exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet (`register_watch` refuses it), and the
    /// resync requeue re-evaluates those rows until they are served.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// Retire the owned children the desired set no longer names, in the
    /// preserved teardown order (old `reconcile_owned_children` diff, R8/R9:
    /// Endpoint before its producer Process). Rows already deleting are left
    /// to the manager's retirement; reports whether anything was retired.
    async fn retire_obsolete_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ResourceKey],
        op: DriverOp,
    ) -> Result<bool, BindingDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        let mut obsolete = owned
            .iter()
            .filter(|row| !row.deleting && !desired.contains(&row.key))
            .collect::<Vec<_>>();
        obsolete.sort_by_key(|row| {
            (
                core::cmp::Reverse(teardown_rank(&row.key.type_name)),
                row.key.name.clone(),
            )
        });
        let retired = !obsolete.is_empty();
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        }
        Ok(retired)
    }

    /// The stored binding for one row: strict neutral spec plus the
    /// identity the fence is pinned to (uid from the durable row; the new
    /// store carries no Zone revision, so the fence's observation revision
    /// is the row revision the manager plane publishes - the row
    /// generation, which the manager maps onto the wire revision, KTD8).
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
            // The fence revision names the row revision the projection was
            // authored at. The manager has no separate Zone revision: its
            // wire revision *is* the row generation (KTD8), so a projection
            // pinned to any other ordinal would either claim an observation
            // the row never held or fall behind the row it describes. The
            // read side keeps the guard - a fence ahead of the row's own
            // revision, under another uid, or under another generation is
            // never current.
            d2b_contracts_resource::v3::ZoneRevision::new(generation.get()),
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
        // The worker executes on the host, exactly as the runtime this
        // driver replaces minted it (old `worker_child_specs` hardcodes
        // Host/host-system) and as the signed `virtiofsd-worker` template
        // the launch ticket resolves through binds it: the template's
        // execution_ref is the volume-virtiofs Provider's
        // config.controllerExecutionRef. The attachment's Guest stays the
        // ticket's target ref (KTD7), re-derived from the owning VolumeBinding
        // row by the Process driver's identity path.
        let process_spec = serde_json::json!({
            "providerRef": WORKER_PROVIDER_REF,
            "executionRef": "Host/host-system",
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

    /// Ensure the desired worker Process and Endpoint children (old
    /// `binding_children` minting). The manager commits each child row BEFORE
    /// creating or updating the child actor (F1, AE1); the Endpoint child
    /// names the worker as its producer, so the worker row is committed
    /// first. Reports whether either ensure mutated the durable child set.
    async fn ensure_children(
        &self,
        ctx: &mut ResourceContext,
        binding: &StoredBinding,
        op: DriverOp,
    ) -> Result<bool, BindingDriverError> {
        let (worker, endpoint) = self.worker_child_specs(binding)?;
        let mut mutated = false;
        for child in [worker, endpoint] {
            match ctx.ensure_child(child).await {
                Ok(EnsureOutcome::Created(_) | EnsureOutcome::Updated(_)) => mutated = true,
                Ok(EnsureOutcome::Unchanged(_)) => {}
                Err(_) => return Err(self.error(BindingDriverErrorKind::ChildMutation, op)),
            }
        }
        Ok(mutated)
    }
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    ResourceUid::from_bytes(bytes).map_err(|_| ())
}

/// The hex spelling one compared uid renders as (issue #508).
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[async_trait::async_trait]
impl ResourceDriver for BindingDriver {
    type Error = BindingDriverError;

    fn classify_error(&self, error: &BindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            BindingDriverErrorKind::SpecInvalid
            | BindingDriverErrorKind::ProviderUnsupported
            | BindingDriverErrorKind::OwnerMismatch
            | BindingDriverErrorKind::ParentSpecInvalid
            | BindingDriverErrorKind::PlanDerivation => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::ParentUnavailable | BindingDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::ServingEffect | BindingDriverErrorKind::ChildMutation => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode, serving Provider check, and the owner-fence check: the
    /// binding's declared parent Volume must be the resource the manager
    /// reports as the row's owner (a child cannot silently change owner).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (_, binding) = self.decoded_binding(ctx, DriverOp::Validate)?;
        self.parent_volume(ctx, &binding, DriverOp::Validate).await?;
        Ok(())
    }

    /// Owned-child adoption (F2): the pre-restart incarnation is adopted
    /// only when the derived plan re-derives exactly, every desired child
    /// row is present and current, and the serving socket is listening.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let (_, binding) = self.decoded_binding(ctx, op)?;
        let stored = self.stored_binding(ctx, binding, op)?;
        let (_, volume_spec) = self.parent_volume(ctx, stored.spec(), op).await?;
        let view = volume_spec
            .views()
            .get(stored.spec().view().as_str())
            .ok_or_else(|| self.rejected(ctx, VirtiofsBindingError::ViewNotFound.code(), op))?;
        let plan = self.derive_plan(ctx, &stored, view, op)?;
        let derived = self.derived_plan(&stored, plan, op)?;
        let desired = self.desired_child_keys(&stored, op)?;
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        let children_current = desired
            .iter()
            .all(|key| owned.iter().any(|row| row.key == *key && !row.deleting));
        let socket = stored.socket_identity(&self.zone_bounded());
        let socket_ready = self.effects.socket_ready(&socket).await;
        if children_current && socket_ready {
            ctx.set_status(BindingDriverStatus::RecoveredPlan {
                plan: derived,
                socket_ready,
            });
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`):
    /// derive the launch plan, ensure the worker Process child and then the
    /// Endpoint child (F1), retire owned children the derived set no longer
    /// names, register the dependency watches (R12/R17), and publish the
    /// in-memory status with the observed serving readiness (R11).
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let (_, binding) = self.decoded_binding(ctx, op)?;
        let stored = self.stored_binding(ctx, binding, op)?;
        let (_, volume_spec) = self.parent_volume(ctx, stored.spec(), op).await?;
        // Dependency edge (R12/R17): a Volume change wakes this actor.
        self.watch_once(ctx, self.parent_volume_key(stored.spec())).await;
        let view = volume_spec
            .views()
            .get(stored.spec().view().as_str())
            .ok_or_else(|| self.rejected(ctx, VirtiofsBindingError::ViewNotFound.code(), op))?;
        let plan = self.derive_plan(ctx, &stored, view, op)?;
        let derived = self.derived_plan(&stored, plan, op)?;

        let mut mutated = self.ensure_children(ctx, &stored, op).await?;
        let desired = self.desired_child_keys(&stored, op)?;
        // Old owner diff (R8/R9): an owned child the derived set no longer
        // names is retired endpoint-first / process-last.
        mutated |= self.retire_obsolete_children(ctx, &desired, op).await?;
        for key in desired {
            self.watch_once(ctx, key).await;
        }

        // Readiness is child-phase driven: the worker socket is the serving
        // evidence and the guest mount the consumer-side one; both
        // fail closed when the port cannot observe them.
        let socket = stored.socket_identity(&self.zone_bounded());
        let socket_ready = self.effects.socket_ready(&socket).await;
        let mount_ready = self
            .effects
            .guest_mount_ready(ctx.key(), &stored)
            .await
            .unwrap_or(false);
        ctx.set_status(BindingDriverStatus::ServingChildren {
            plan: derived,
            converged: !mutated,
            socket_ready,
            mount_ready,
        });
        // KTD3: the fenced projection is the wire-visible readiness, and the
        // row's actor republishes it on every concluding pass - a pass that
        // published none would clear the manager's projection layer. `ready`
        // is the serving evidence this plane can observe: the worker's
        // private socket listening. The guest-mount half of the controller's
        // Ready phase is a *drain* gate (U13 wires it to the target layer's
        // own observation), not a readiness term here: no converted type has
        // Guest-side effect code yet, so a projection gated on a target-local
        // realization would pin readiness false forever while the share is
        // actually being served. `mount_ready` stays in the typed status for
        // consumers that need it.
        let reason = (!socket_ready).then_some(VirtiofsBindingError::BindingNotReady);
        ctx.set_status_projection(
            serde_json::to_value(stored.status_projection(socket_ready, reason))
                .expect("the fenced binding projection is always serializable"),
        );
        if mutated || !socket_ready {
            // The child rows were (re)committed this pass, or the socket is
            // not serving yet: re-check on the preserved resync cadence (the
            // Runner contract's repair interval) - the same shape the Guest
            // driver uses while its Provider phase is not Ready, so a
            // readiness the port cannot observe yet converges without
            // depending on a watch delivery.
            ctx.requeue_after(BINDING_RESYNC);
        }
        // `Satisfied` here is the *driver's* convergence - the derived child
        // set is committed and this pass did its work - not the serving
        // readiness, which is the fenced projection set above (`ready` is
        // false with `BindingNotReady` until the worker's socket listens).
        // The two must not be conflated: the binding's worker Process is a
        // child of this row, and deferring this row to `RetryScheduled` while
        // its socket is absent publishes `Pending` for the parent the worker's
        // own launch chains through - a cycle in which neither the socket nor
        // the parent ever converges. The projection is what tells a Guest the
        // share is not being served yet.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The call nudges each owned child - the
    /// endpoint and worker Process rows - through its own
    /// finalize-before-delete pass and requeues this pass while any child row
    /// is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::DrainPending, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown with the preserved drain semantics (R9/F3, old
    /// `finalize_binding`): the guest mount is observed BEFORE anything is
    /// deleted (KTD6) - a present mount keeps the durable deleting mark and
    /// the owned children, and the pass retries instead of force-clearing a
    /// serve that is still mounted. Otherwise the Endpoint realization is
    /// removed first and the worker Process child last, and the manager holds
    /// the parent row until the last child retires. Idempotent under retry
    /// (R10): missing child rows converge without effects.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        // A row whose spec no longer decodes still drains by owner key below.
        if let Ok((_, binding)) = self.decoded_binding(ctx, op)
            && let Ok(stored) = self.stored_binding(ctx, binding, op)
        {
            if self
                .effects
                .guest_mount_ready(ctx.key(), &stored)
                .await
                .unwrap_or(false)
            {
                // Old `VirtiofsBindingController::drain` returned
                // `DrainIncomplete`: the finalizer (now the durable deleting
                // mark) and the owned children stay, and the actor retries.
                return Err(self
                    .error(BindingDriverErrorKind::ServingEffect, op)
                    .with_detail(
                        FailureDetail::at("delete/mount").comparison(FailureComparison::new(
                            "guest.mount",
                            "released",
                            "still mounted",
                        )),
                    ));
            }
            let [worker, endpoint] = self.desired_child_keys(&stored, op)?;
            // Endpoint-first: the endpoint child row is retired and the
            // socket realization removed before the worker goes away (the
            // worker's socket drain relies on it).
            ctx.delete(&endpoint)
                .await
                .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
            let socket = stored.socket_identity(&self.zone_bounded());
            self.effects
                .remove_socket(&socket)
                .await
                .map_err(|error| {
                    self.error(BindingDriverErrorKind::ServingEffect, op)
                        .with_detail(
                            FailureDetail::at("delete/socket")
                                .comparison(FailureComparison::new(
                                    "binding.socket",
                                    "removed",
                                    "remove failed",
                                ))
                                .with_note(error),
                        )
                })?;
            // Process-last.
            ctx.delete(&worker)
                .await
                .map_err(|_| self.error(BindingDriverErrorKind::ChildMutation, op))?;
        }
        // Retire anything else this binding owns behind the same role order;
        // already-deleting rows are left to the manager. The manager keeps
        // the parent row until the last child retires (F3) - the guarantee
        // the old finalizer existed for.
        let _ = self.retire_obsolete_children(ctx, &[], op).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The resource verbs the VolumeBinding type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
/// The execution domains the VolumeBinding type can be reconciled in.
///
/// Derived from the placement contract: `VolumeBinding` names no placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a binding row
/// never carries the canonical `spec.executionRef` and the plane reconciles
/// it on its containing Zone's Host. The attachment's `executionRef` selects
/// the Guest that consumes the share, never where the binding row itself is
/// reconciled.
const BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the binding driver reads while reconciling.
///
/// The driver resolves the owning Volume row through the manager for its view
/// spec and its owner fence; the worker plan it derives from that row is what
/// the owned children are minted from.
const BINDING_READS: &[WellKnownType] = &[WellKnownType::VOLUME];

/// The VolumeBinding type's driver declaration.
///
/// `VolumeBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot
/// serve the converted binding shapes without it, so it must be registered
/// before the plane opens. The type is not exportable: `ResourceExport`
/// admits only qualified `*.d2bus.org.*Service` types, so a binding can never
/// be an export subject. The driver serves no broker operations and
/// contributes no startup steps; the worker Process and Endpoint children it
/// mints are declared in [`BINDING_CREATIONS`], and the declaration carries
/// the family's declared effects service
/// ([`crate::effects_service::BINDING_EFFECTS_SERVICE`]), which the daemon
/// hosts per zone from the family's registered factory.
pub fn binding_descriptor(args: BindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::VOLUME_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: BINDING_READS,
        operations: &[],
        creations: BINDING_CREATIONS,
        startup: &[],
        services: &[BINDING_EFFECTS_SERVICE],
        decoder: binding_spec_decoder(),
        factory: Arc::new(BindingDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted serving port and a recording
// manager endpoint with one shared ordered log (teardown and commit-before-
// spawn ordering observed as the manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{
        ResourceGeneration, ResourceUid, ZoneRevision,
        resource_status::StatusCode,
        volume_binding::{VolumeBindingReadinessFence, VolumeBindingStatusResource},
    };
    use crate::test_support::FakeServingEffects;
    use d2b_provider_volume_virtiofs::WORKER_TEMPLATE;
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

    /// Recording manager endpoint over one shared ordered log. Rows are
    /// keyed by `zone/type/name`; `get` returns the parent Volume row the
    /// binding declares.
    #[derive(Clone)]
    struct RecordingManager {
        zone: String,
        log: Arc<parking_lot::Mutex<OrderLog>>,
        rows: Arc<parking_lot::Mutex<Vec<StoredDesiredResource>>>,
        watch_targets: Arc<parking_lot::Mutex<Vec<ResourceKey>>>,
        next_uid: Arc<std::sync::atomic::AtomicU64>,
        fail_reads: Arc<std::sync::atomic::AtomicBool>,
    }

    impl RecordingManager {
        fn new() -> Self {
            Self {
                zone: "work".to_owned(),
                log: Arc::new(parking_lot::Mutex::new(Vec::new())),
                rows: Arc::new(parking_lot::Mutex::new(Vec::new())),
                watch_targets: Arc::new(parking_lot::Mutex::new(Vec::new())),
                next_uid: Arc::new(std::sync::atomic::AtomicU64::new(1)),
                fail_reads: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }

        /// Make `get` answer `ManagerRpc` (the unanswerable plane).
        fn set_fail_reads(&self, fail: bool) {
            self.fail_reads.store(fail, std::sync::atomic::Ordering::SeqCst);
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

        /// Seed one owned child row (drift the driver must retire).
        fn seed_owned(&self, key: ResourceKey) {
            self.rows.lock().push(StoredDesiredResource {
                key,
                uid: [0x77; 16],
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            });
        }

        fn log(&self) -> Arc<parking_lot::Mutex<OrderLog>> {
            Arc::clone(&self.log)
        }

        fn order(&self) -> Vec<String> {
            self.log.lock().clone()
        }

        fn rows(&self) -> Vec<StoredDesiredResource> {
            self.rows.lock().clone()
        }

        fn watch_targets(&self) -> Vec<ResourceKey> {
            self.watch_targets.lock().clone()
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
            self.log.lock().push(format!("ensure:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            let mut rows = self.rows.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            self.log.lock().push(format!("spawned:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(outcome)
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned()) // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            // Desired rows only: this fixture publishes no runtime status, so
            // it serves no observed state.
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .push(format!("delete:{}/{}", key.type_name, key.name));
            self.rows.lock().retain(|row| row.key != *key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            // Manager rows always exist here; the actor-side handler is the
            // runtime's, so the fake only records the registration.
            let mut targets = self.watch_targets.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            targets.push(registration.target.clone());
            Ok(WatchId(targets.len() as u64))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Recording requeue: the driver's resync schedules are observable.
    #[derive(Clone)]
    struct RecordingRequeue {
        scheduled: Arc<parking_lot::Mutex<Vec<ResourceKey>>>,
    }

    impl RecordingRequeue {
        fn new() -> Self {
            Self {
                scheduled: Arc::new(parking_lot::Mutex::new(Vec::new())),
            }
        }

        fn scheduled(&self) -> Vec<ResourceKey> {
            self.scheduled.lock().clone()
        }
    }

    impl d2b_resource_runtime::context::RequeueScheduler for RecordingRequeue {
        fn schedule(
            &self,
            key: ResourceKey,
            _after: std::time::Duration,
        ) -> d2b_resource_runtime::context::RequeueId {
            let mut scheduled = self.scheduled.lock();
            scheduled.push(key);
            d2b_resource_runtime::context::RequeueId(scheduled.len() as u64)
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
        requeue: RecordingRequeue,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManager) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::new();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            binding_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            manager,
            requeue,
        }
    }

    fn row_key(id: &str, zone: &str) -> ResourceKey {
        let (type_name, name) = id.split_once('/').expect("typed id");
        ResourceKey::new(zone, type_name, name)
    }

    async fn driver(effects: Arc<FakeServingEffects>) -> Box<dyn DynResourceDriver> {
        let factory = BindingDriverFactory::new(BindingDriverArgs {
            zone: "work".to_owned(),
            facets: effects.facet_set(),
            vcpu_count: 4,
        });
        factory
            .create(&ResourceKey::new("work", "VolumeBinding", "binding"))
            .await
    }

    // -- factory -------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn factory_registers_only_the_binding_resource_type() {
        let factory = BindingDriverFactory::new(BindingDriverArgs {
            zone: "work".to_owned(),
            facets: FakeServingEffects::new().facet_set(),
            vcpu_count: 4,
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "VolumeBinding");
    }

    // -- reconcile: worker + endpoint children --------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn ensure_derives_worker_and_endpoint_children_persisted_before_spawn() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.validate(&mut f.ctx).await.expect("validate");
        let outcome = d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome::Satisfied,
            "the pass converged its own work; the serving half rides the fenced projection, which \
             reports `ready: false` with `BindingNotReady` until the worker's socket listens"
        );

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
        // Child-phase driven readiness is observed this pass.
        assert!(order.contains(&"socket-ready".to_owned()));
        assert!(order.contains(&"guest-mount".to_owned()));
        // The derived plan is the status handle (never a resource field).
        match f.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::ServingChildren {
                plan,
                converged,
                socket_ready,
                mount_ready,
            }) => {
                assert!(!converged, "the first pass committed the child rows");
                assert!(!socket_ready, "the scripted socket is not ready yet");
                assert!(!mount_ready);
                assert_eq!(plan.plan.template, WORKER_TEMPLATE);
                assert_eq!(plan.plan.thread_pool_size, 4);
                assert!(plan.plan.readonly);
            }
            other => panic!("expected ServingChildren, got {other:?}"),
        }
        // The not-yet-current child set re-checks on the preserved resync.
        assert_eq!(f.requeue.scheduled().len(), 1);

        // Second pass: the same children are current, no churn; the socket
        // is still not serving, so the repair cadence keeps re-checking.
        d.reconcile(&mut f.ctx).await.expect("reconcile again");
        let ensures = f
            .manager
            .order()
            .iter()
            .filter(|entry| entry.starts_with("ensure:"))
            .count();
        assert_eq!(ensures, 4, "two passes, two ensures each");
        assert_eq!(f.requeue.scheduled().len(), 2);
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::ServingChildren {
                converged: true,
                ..
            })
        ));

        // Once the socket serves, the binding converges and stops requeueing.
        fake.make_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile third");
        assert_eq!(f.requeue.scheduled().len(), 2);
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::ServingChildren {
                converged: true,
                socket_ready: true,
                ..
            })
        ));
    }

    // -- fenced status projection (KTD3) ---------------------------------------

    /// The concluding pass publishes the fenced projection as the frozen
    /// typed contract: the fence names the row's own uid, generation, and
    /// revision, so the frozen reader accepts it for exactly this row and for
    /// no unfenced revision.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_concluding_pass_publishes_the_fenced_status_projection() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let row = binding_row([0x42; 16]);
        let mut f = fixture(row.clone(), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.validate(&mut f.ctx).await.expect("validate");
        fake.make_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let projection = f
            .ctx
            .take_status_projection()
            .expect("the concluding pass publishes the fenced projection");
        let typed = serde_json::from_value::<VolumeBindingStatusResource>(projection.clone())
            .expect("the projection is the typed fenced contract");
        assert!(typed.ready, "the worker socket is serving");
        assert!(typed.reason.is_none());

        // The fence names this row's own identity at the row revision the
        // manager publishes (the row generation), so the frozen reader accepts
        // the layer for exactly this row and for no other.
        let fence = typed.fence.clone();
        assert_eq!(
            fence.uid,
            super::resource_uid(f.ctx.uid()).expect("the row uid is canonical"),
        );
        assert_eq!(
            fence.generation,
            ResourceGeneration::new(f.ctx.generation()).expect("the row generation is canonical"),
        );
        assert_eq!(fence.revision.get(), f.ctx.generation());
        assert!(fence.revision.get() > 0);
        assert!(typed.readiness_is_current(&fence.uid, fence.generation, fence.revision));
        assert!(
            !typed.readiness_is_current(&fence.uid, fence.generation, ZoneRevision::new(0)),
            "an unfenced revision never reports the row ready"
        );
    }

    /// The fence cannot be satisfied vacuously: a pass that cannot observe
    /// the serving socket publishes `ready: false` under the frozen reason,
    /// and a projection authored under an older identity (or ahead of the
    /// stored revision) never reports the row ready.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn stale_or_not_serving_fences_never_report_ready() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let row = binding_row([0x42; 16]);
        let mut f = fixture(row.clone(), manager.clone());
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pending = serde_json::from_value::<VolumeBindingStatusResource>(
            f.ctx.take_status_projection().expect("projection"),
        )
        .expect("typed projection");
        assert!(!pending.ready, "no socket was observed serving");
        assert_eq!(
            pending.reason.as_ref().map(StatusCode::as_str),
            Some("binding-not-ready"),
            "the not-ready projection carries the frozen provider reason"
        );

        // The row moves to generation 2 (a spec change): the next pass pins
        // the projection to the new identity, and the old one can never
        // report the newer row ready.
        let mut advanced = row.clone();
        advanced.generation = 2;
        let manager2 = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake2 = FakeServingEffects::shared(manager2.log());
        let mut f2 = fixture(advanced, manager2);
        let mut d2 = driver(fake2.clone()).await;
        fake2.make_ready();
        d2.reconcile(&mut f2.ctx).await.expect("reconcile");
        let current = serde_json::from_value::<VolumeBindingStatusResource>(
            f2.ctx.take_status_projection().expect("projection"),
        )
        .expect("typed projection");
        assert!(current.ready);
        assert_eq!(current.fence.generation.get(), 2);

        let uid = current.fence.uid.clone();
        let generation = current.fence.generation;
        let revision = current.fence.revision;
        assert!(
            !pending.readiness_is_current(&uid, generation, revision),
            "a fence from the older row generation never reports the new row ready"
        );
        assert!(current.readiness_is_current(&uid, generation, revision));

        // A fence ahead of the stored revision is not current: the revision
        // is a currency bound, not a free field.
        let ahead = VolumeBindingStatusResource {
            ready: true,
            fence: VolumeBindingReadinessFence {
                uid: uid.clone(),
                generation,
                revision: ZoneRevision::new(revision.get() + 1),
            },
            reason: None,
        };
        assert!(!ahead.readiness_is_current(&uid, generation, revision));

        // And a fence under a foreign identity is never current.
        let foreign =
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("foreign uid");
        assert!(!current.readiness_is_current(&foreign, generation, revision));
    }

    // -- launch authority: plan + typed, argv-free worker child ---------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn worker_child_carries_the_signed_template_and_no_argv() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let rows = manager.rows();
        let worker = rows
            .iter()
            .find(|row| row.key.type_name == "Process")
            .expect("worker Process child row");
        let endpoint = rows
            .iter()
            .find(|row| row.key.type_name == "Endpoint")
            .expect("Endpoint child row");
        let mut process_spec: serde_json::Value =
            serde_json::from_slice(&worker.spec).expect("worker spec json");
        // The serving posture is the frozen default; the template is the
        // signed worker template the launch path resolves (KTD13/U17).
        assert_eq!(process_spec["template"], WORKER_TEMPLATE);
        assert_eq!(process_spec["executionRef"], "Host/host-system");
        assert_eq!(process_spec["processClass"], "worker");
        // No argv/launch parameter may travel in the resource (KTD1): the
        // Process controller composes the launch from the binding/Volume rows.
        let rendered = process_spec.to_string();
        for forbidden in ["argv", "args", "command", "cmd", "exec"] {
            assert!(
                !rendered.contains(&format!("\"{forbidden}\"")),
                "worker child must stay argv-free, found {forbidden:?}: {rendered}"
            );
        }
        // The store validates the base layer; providerRef lives in the
        // envelope layer (mirroring validate_standard_base_bytes).
        process_spec
            .as_object_mut()
            .expect("process spec object")
            .remove("providerRef");
        let typed: d2b_contracts_resource::v3::process::ProcessSpec =
            serde_json::from_value(process_spec).expect("ProcessSpec parses");
        assert_eq!(typed.execution().template().as_str(), WORKER_TEMPLATE);
        let endpoint_spec: serde_json::Value =
            serde_json::from_slice(&endpoint.spec).expect("endpoint spec json");
        let _typed: d2b_provider_endpoint::endpoint::EndpointSpec =
            serde_json::from_value(endpoint_spec).expect("EndpointSpec parses");
    }

    // -- recover: plan re-derivation matches the pre-restart incarnation ------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_rederives_the_launch_plan_matching_the_pre_restart_incarnation() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        // Pre-restart: reconcile derives the plan and commits the child rows.
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pre = match f.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::ServingChildren { plan, .. }) => plan.clone(),
            other => panic!("expected ServingChildren, got {other:?}"),
        };

        // Restart: a fresh context over the same durable rows (the manager
        // holds them across the restart) recovers; the re-derived plan must
        // equal the pre-restart incarnation (the plan is path-free, so
        // equality covers the frozen template/sandbox posture).
        let mut f2 = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d2 = driver(fake.clone()).await;
        fake.make_ready();
        assert_eq!(
            d2.recover(&mut f2.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "owned children current and socket ready: pre-restart incarnation adopted"
        );
        match f2.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::RecoveredPlan { plan, socket_ready }) => {
                assert!(socket_ready);
                assert_eq!(pre, plan.clone(), "re-derived plan matches pre-restart");
                assert_eq!(plan.plan.template, WORKER_TEMPLATE);
                assert_eq!(plan.plan.thread_pool_size, 4);
                assert!(plan.plan.readonly);
                assert!(!plan.plan.posix_acl);
                assert!(!plan.plan.xattr);
            }
            other => panic!("expected RecoveredPlan, got {other:?}"),
        }

        // A restart whose children are gone adopts nothing.
        let mut f3 = fixture(binding_row([0x42; 16]), manager_clone_placeholder());
        let mut d3 = driver(fake).await;
        assert_eq!(
            d3.recover(&mut f3.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "no owned child rows: the realization is missing"
        );
    }

    // -- finalize: owned children retire before the binding (F3) -------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_binding_teardown() {
        let manager = RecordingManager::new();
        manager.seed_owned(ResourceKey::new("work", "Process", "worker-0"));
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake).await;

        // A live owned child: the pass requeues and the binding's own
        // teardown (mount gate, socket removal) does not run.
        let failure = d.finalize(&mut f.ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            manager.order(),
            vec!["delete:Process/worker-0".to_owned()],
            "the owned child is nudged through its own finalize-before-delete pass"
        );

        // The manager removed the retired child row: the same pass converges.
        d.finalize(&mut f.ctx).await.expect("converged once the child retired");
    }

    // -- teardown ordering -----------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_drains_the_endpoint_before_the_worker_and_behind_the_mount_gate() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let before = manager.order().len();
        d.delete(&mut f.ctx).await.expect("delete");

        let order: Vec<String> = manager.order().into_iter().skip(before).collect();
        let position = |needle: &str| {
            order
                .iter()
                .position(|entry| entry.starts_with(needle))
                .unwrap_or_else(|| panic!("{needle} recorded in {order:?}"))
        };
        // KTD6: the guest mount is observed before anything is deleted, and
        // the preserved ordering holds: endpoint row, socket realization,
        // worker Process row last.
        assert!(position("guest-mount") < position("delete:Endpoint/"));
        assert!(position("delete:Endpoint/") < position("remove-socket"));
        assert!(position("remove-socket") < position("delete:Process/"));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn mounted_share_blocks_the_drain_before_any_child_is_removed() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let children_before = manager.rows().len();
        fake.make_mounted();

        // KTD6: a present mount keeps the durable deleting mark and the
        // owned children; the pass retries instead of force-clearing a
        // served share (old `VirtiofsBindingError::DrainIncomplete`).
        let failure = d.delete(&mut f.ctx).await.expect_err("drain must block");
        assert_eq!(failure.class(), FailureClass::Retryable);
        let teardown: Vec<String> = manager
            .order()
            .into_iter()
            .filter(|entry| entry.starts_with("delete:") || entry == "remove-socket")
            .collect();
        assert!(teardown.is_empty(), "nothing may be torn down: {teardown:?}");
        assert_eq!(manager.rows().len(), children_before, "children stay committed");
    }

    // -- owned-child drift -----------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn obsolete_owned_child_is_retired_endpoint_first() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        // A stale Endpoint child the derived set no longer names (a binding
        // whose endpoint identity changed), plus a stale Process sibling.
        manager.seed_owned(ResourceKey::new("work", "Endpoint", "stale-endpoint"));
        manager.seed_owned(ResourceKey::new("work", "Process", "stale-worker"));
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake).await;

        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let order = manager.order();
        let stale_endpoint = order
            .iter()
            .position(|entry| entry == "delete:Endpoint/stale-endpoint")
            .expect("stale endpoint retired");
        let stale_worker = order
            .iter()
            .position(|entry| entry == "delete:Process/stale-worker")
            .expect("stale worker retired");
        assert!(
            stale_endpoint < stale_worker,
            "endpoint-first / process-last retirement: {order:?}"
        );
    }

    // -- dependency edges ------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile one");
        d.reconcile(&mut f.ctx).await.expect("reconcile two");

        let mut targets = manager
            .watch_targets()
            .into_iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        let registered = targets.len();
        targets.sort();
        targets.dedup();
        assert_eq!(
            registered,
            targets.len(),
            "no dependency watch is registered twice"
        );
        // The parent Volume plus both derived children (R12/R17).
        assert!(targets.contains(&"Volume/data".to_owned()));
        let child_keys = manager
            .rows()
            .iter()
            .filter(|row| matches!(row.key.type_name.as_str(), "Process" | "Endpoint"))
            .map(|row| format!("{}/{}", row.key.type_name, row.key.name))
            .collect::<Vec<_>>();
        assert_eq!(child_keys.len(), 2, "worker + endpoint child rows");
        for key in child_keys {
            assert!(targets.contains(&key), "the dependency edge watches {key}");
        }
    }

    // -- terminal rejection visibility -----------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn rejected_view_keeps_its_stable_reason_in_status() {
        let manager = RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes());
        let fake = FakeServingEffects::shared(manager.log());
        let mut row = binding_row([0x42; 16]);
        let mut spec: serde_json::Value = serde_json::from_slice(&row.spec).expect("binding spec");
        spec["view"] = serde_json::json!("absent");
        row.spec = serde_json::to_vec(&spec).expect("binding spec bytes");
        let mut f = fixture(row, manager.clone());
        let mut d = driver(fake).await;

        let failure = d.reconcile(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
        // Old `failed_binding_result`: the stable provider code stays visible.
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::Rejected {
                reason: "view-not-found"
            })
        ));
        assert!(
            manager.order().iter().all(|entry| !entry.starts_with("ensure:")),
            "a rejected binding mints no children"
        );
    }

    // -- owner guard -----------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    /// Issue #511 at the parent-row read (`BindingDriver::parent_volume`): a
    /// parent Volume row that is not observable yet defers retryably - the row
    /// may simply not be committed yet - while a present row with a different
    /// owner uid stays terminal (the owner guard above is unchanged).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn absent_parent_row_defers_retryably_while_owner_mismatch_stays_terminal() {
        let fake = FakeServingEffects::new();

        // Not committed yet: the manager holds no parent Volume row.
        let manager = RecordingManager::new();
        let mut f = fixture(binding_row([0x42; 16]), manager.clone());
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("absent parent row");
        assert_eq!(
            failure.class(),
            FailureClass::Retryable,
            "a parent row that simply does not exist yet must defer, not fail terminal"
        );
        assert!(
            manager.order().iter().all(|entry| !entry.starts_with("ensure:")),
            "no child is minted before the parent row is observable"
        );

        // The manager cannot answer: the same retryable defer.
        manager.set_fail_reads(true);
        let failure = d.reconcile(&mut f.ctx).await.expect_err("unanswerable manager");
        assert_eq!(
            failure.class(),
            FailureClass::Retryable,
            "an unanswerable manager defers as well and is never reported as absence"
        );
        manager.set_fail_reads(false);

        // Present but a different owner uid: still terminal.
        let manager = RecordingManager::new().with_parent([0x99; 16], &parent_volume_bytes());
        let mut f = fixture(binding_row([0x42; 16]), manager);
        let mut d = driver(fake).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("owner mismatch");
        assert_eq!(
            failure.class(),
            FailureClass::Terminal,
            "a present row whose owner actually differs stays terminal"
        );
    }

    // -- deterministic child identity -------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    fn manager_clone_placeholder() -> RecordingManager {
        RecordingManager::new().with_parent([0x42; 16], &parent_volume_bytes())
    }
}
