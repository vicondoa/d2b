//! The Volume resource driver: the v3 `ResourceDriver` conversion of the
//! shared-Volume path.
//!
//! The driver keeps the shared-volume Volume leg and nothing else: recover
//! discovers existing volume-local layout state on the host target,
//! reconcile runs the preserved volume-local layout effect and then derives
//! one deterministic `VolumeBinding` child per virtiofs attachment through
//! the manager-routed ensure (the child spec is committed BEFORE the child
//! actor exists), and delete removes the Volume's own layout state. Child
//! teardown on parent delete is the manager's reconcile_children diff: the
//! manager marks children deleted and drives bindings -> endpoint ->
//! process-last ordering; this driver's delete covers only the Volume's own
//! effect, with the drain finalizer preserved behind the provider port.
//!
//! Conversion mapping:
//! - `describe` -> [`volume_descriptor`] registration under `Volume`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - layout effect + `volume_children` ensure -> [`ResourceDriver::reconcile`].
//! - volume-local cleanup -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only).
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`VolumeDriverEffects`]): the layout effect over the
//! preserved volume-local controller and the durable probe recover reads.
//! U7: the production implementation is this crate's own
//! [`VolumeEffectsService`], built from the daemon-supplied declared facets
//! ([`crate::facets`]); the daemon holds no volume effect implementation
//! and no externally built port appears at the construction site (R2).
use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::{
    BindingContractError, ResourceName, ResourceRef, ResourceSpec, ZoneId,
    ResourceTypeName as ContractResourceTypeName, ResourceUid,
    volume::VolumeSpec,
};
use crate::effects_service::{VOLUME_EFFECTS_SERVICE, VolumeEffectsService};
use crate::facets::{BindingEvidenceAbsent, VolumeBindingAdmission, VolumeEffectFacets};
use d2b_provider_toolkit::shared_provider::{ContextChildSurface, SharedProviderChildSurface};
use d2b_provider_volume_local::{
    AdmittedVolumeBinding, canonical_binding_row, desired_binding_intents,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::relations::DecodedBindingRequest;
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, ChildCustody, DriverDescriptor,
    WellKnownType,
};

/// The one resource type this factory serves.
pub const VOLUME_TYPE_NAME: &str = "Volume";

/// The fixed Provider this driver serves (old `SharedVolumeResourceKind`).
const VOLUME_PROVIDER_NAME: &str = "volume-local";

/// The serving Provider the derived `VolumeBinding` rows select.
const BINDING_PROVIDER_REF: &str = d2b_provider_volume_virtiofs::PROVIDER_REF;

/// Deterministic `VolumeBinding` child type (old
/// `VOLUME_BINDING_RESOURCE_TYPE`).
const VOLUME_BINDING_TYPE: &str = "VolumeBinding";

/// How soon a Volume re-checks the state it serves.
///
/// A Volume's realization is its layout root plus the binding children
/// derived from it, and both can change under a pass that is not watching the
/// host: a root that goes missing is the volume-local provider's own evidence
/// of loss, and a binding child can be retired by another owner. The re-check
/// is one pass over the child set, so a settled Volume renews it on this
/// cadence rather than waiting for an unrelated trigger.
pub const VOLUME_RESYNC: Duration = Duration::from_secs(30);

/// The children this driver mints: one deterministic `VolumeBinding` per
/// admitted virtiofs attachment, served by the Provider the derived rows
/// select and created by this driver, which also owns their teardown.
pub const VOLUME_CREATIONS: &[ChildCreation] = &[ChildCreation {
    child: WellKnownType::VOLUME_BINDING,
    provider_ref: BINDING_PROVIDER_REF,
    custody: ChildCustody::DriverOwned,
    order: 0,
}];

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeDriverErrorKind {
    /// The durable spec did not decode as the closed Volume contract.
    SpecInvalid,
    /// The spec selected a Provider this driver does not own.
    ProviderUnsupported,
    /// A provider layout effect failed transiently.
    LayoutEffect,
    /// The manager refused a child ensure/delete.
    ChildMutation,
    /// Owned children are still retiring before this Volume may drain.
    DrainPending,
    /// Deterministic child derivation failed (invalid attachment).
    ChildDerivation,
}

impl VolumeDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::LayoutEffect | Self::ChildMutation | Self::DrainPending => {
                FailureClass::Retryable
            }
            Self::SpecInvalid | Self::ProviderUnsupported | Self::ChildDerivation => {
                FailureClass::Terminal
            }
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::VOLUME_SPEC_INVALID,
            Self::ProviderUnsupported => FailureKinds::VOLUME_PROVIDER_UNSUPPORTED,
            Self::LayoutEffect => FailureKinds::VOLUME_LAYOUT_EFFECT_FAILED,
            Self::ChildMutation => FailureKinds::VOLUME_CHILD_MUTATION_FAILED,
            // The shared child-first-teardown kind: draining is not a child
            // mutation.
            Self::DrainPending => FailureKinds::CHILDREN_DRAINING,
            Self::ChildDerivation => FailureKinds::VOLUME_CHILD_DERIVATION_INVALID,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct VolumeDriverError {
    kind: VolumeDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl VolumeDriverError {
    fn new(kind: VolumeDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for VolumeDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            VolumeDriverErrorKind::SpecInvalid => "volume-spec-invalid",
            VolumeDriverErrorKind::ProviderUnsupported => "volume-provider-unsupported",
            VolumeDriverErrorKind::LayoutEffect => "volume-layout-effect-failed",
            VolumeDriverErrorKind::ChildMutation => "volume-child-mutation-failed",
            VolumeDriverErrorKind::DrainPending => "children-draining",
            VolumeDriverErrorKind::ChildDerivation => "volume-child-derivation-invalid",
        })
    }
}

impl std::error::Error for VolumeDriverError {}

/// Typed in-memory status projection (R11: never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumeDriverStatus {
    /// The layout effect is in flight.
    EnsuringLayout,
    /// Binding children derived; readiness aggregates the child rows this
    /// pass converged (R8/R9: child phases, never a parent-side override).
    ServingChildren {
        /// Every `VolumeBinding` row this pass serves: the attachment-shaped
        /// children the spec derives plus the canonical relationships the
        /// admission committed.
        desired: usize,
        /// Whether every attachment-shaped child this pass derived is
        /// present. A canonical row is committed by the pass that derives it,
        /// so it is converged by construction or that pass failed.
        converged: bool,
        /// What the canonical KTD2 path (U14) did on this pass.
        canonical: CanonicalBindingState,
    },
}

/// What the canonical `VolumeBinding` path (U14, KTD2/KTD3) did on one
/// pass.
///
/// These three states are the whole contract of the producing half: a row
/// is committed only out of a real admission, and a pass with no evidence
/// changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanonicalBindingState {
    /// The pass did not reach the admission (recover adopts without
    /// reconciling).
    NotReconciled,
    /// The seam admitted exactly this many relationships, and every one of
    /// them is committed under this row.
    Committed { relationships: usize },
    /// The seam carried no admission evidence: nothing was committed and
    /// nothing was retired, and the refusal names what is missing.
    EvidenceAbsent(BindingEvidenceAbsent),
}

impl CanonicalBindingState {
    /// How many canonical relationships this row commits after the pass.
    pub(crate) const fn committed(&self) -> usize {
        match self {
            Self::NotReconciled | Self::EvidenceAbsent(_) => 0,
            Self::Committed { relationships } => *relationships,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one Volume row (KTD2), exactly as persisted.
/// `raw` keeps the exact stored bytes so audits can assert the driver never
/// mutates the durable envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeSpecEnvelope {
    pub(crate) raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for Volume rows.
pub fn volume_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| VolumeSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing layout effect surface the Volume driver needs. The
/// production implementation is this crate's own [`VolumeEffectsService`]
/// (U7) over the daemon-supplied facets; test doubles implement the same
/// seam (R4).
///
/// Object-erased on purpose: the driver holds the port as
/// `Arc<dyn VolumeDriverEffects>` so one factory serves every Volume row.
#[async_trait::async_trait]
pub trait VolumeDriverEffects: Send + Sync + 'static {
    /// Run the preserved volume-local layout reconcile and report whether
    /// the layout phase reached `Ready`.
    async fn ensure_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String>;

    /// Remove the Volume's own layout state (drain finalizer preserved);
    /// idempotent under retry (R10).
    async fn remove_layout(&self, volume_uid: &ResourceUid, spec: &VolumeSpec)
        -> Result<(), String>;

    /// Discover existing volume-local layout state for this exact uid
    /// (recover probe).
    fn has_layout(&self, volume_uid: &ResourceUid) -> bool;

    /// Admit the canonical `VolumeBindingRequest` relationships declared
    /// against this Volume and report the ones the source may commit a row
    /// for (U14, KTD2/KTD3).
    ///
    /// The refusal is a named absence of evidence, never a fallback: the
    /// driver commits no canonical row without it, because a row is a
    /// relationship and an unfenced relationship is not one.
    async fn admit_bindings(
        &self,
        source: &VolumeBindingAdmission<'_>,
    ) -> Result<Vec<AdmittedVolumeBinding>, BindingEvidenceAbsent>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the Volume driver
/// factory for one zone: the declared facet set the effects run over (U7).
pub struct VolumeDriverArgs {
    /// The daemon-supplied facet set the family's effects implementation is
    /// built from. The composition supplies the objects; the driver never
    /// holds a daemon state type and no externally built port appears here
    /// (R2).
    pub facets: VolumeEffectFacets,
}

/// [`ResourceDriverFactory`] for the `Volume` resource type. Construction is
/// infallible by contract (R3).
pub(crate) struct VolumeDriverFactory {
    types: [ResourceTypeName; 1],
    args: VolumeDriverArgs,
}

impl VolumeDriverFactory {
    pub(crate) fn new(args: VolumeDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(VOLUME_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for VolumeDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(VolumeDriver::new(VolumeDriverArgs {
            facets: self.args.facets.clone(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Volume resource's driver.
#[derive(Clone)]
pub(crate) struct VolumeDriver {
    effects: Arc<dyn VolumeDriverEffects>,
    /// In-memory layout phase (spec section 32: nothing persisted; recover
    /// re-probes the host state through [`VolumeDriverEffects::has_layout`]).
    layout_ready: Arc<std::sync::atomic::AtomicBool>,
}

/// One desired `VolumeBinding` child derived from a Volume attachment
/// (old `volume_children`). The child key is deterministic from the parent
/// plus the attachment tuple (volume, execution target, view, mount path) -
/// never from the attachment index - so reordering declared attachments
/// never churns identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredBindingChild {
    pub(crate) name: String,
    /// Exact child spec envelope bytes (neutral binding + serving Provider
    /// reference; no provider extension, KTD1).
    pub(crate) spec: Vec<u8>,
}

/// One canonical `VolumeBinding` child derived from an admitted source
/// relationship (U14).
///
/// The desired bytes are the consumer's own `VolumeBindingRequest`, so the
/// committed row is the declaration the consumer authored rather than a
/// second description translated out of an attachment list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalBindingChild {
    /// Deterministic child name derived from the relationship's identities.
    pub name: String,
    /// Exact canonical request bytes committed as the child's base spec.
    pub spec: Vec<u8>,
}

/// Derive the canonical `VolumeBinding` children the source owns.
///
/// Every admitted relationship becomes exactly one row, named from the
/// KTD3 key rather than from a declaration position, so the same relationship
/// keeps one identity across restarts and two relationships never collide by
/// ordering.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when a derived row name is
/// not a bounded token.
pub fn canonical_binding_children(
    admitted: &[AdmittedVolumeBinding],
) -> Result<Vec<CanonicalBindingChild>, BindingContractError> {
    admitted
        .iter()
        .map(|admitted| {
            let row = canonical_binding_row(admitted)?;
            Ok(CanonicalBindingChild {
                name: row.name().as_str().to_owned(),
                spec: row.spec().to_vec(),
            })
        })
        .collect()
}

impl VolumeDriver {
    pub(crate) fn new(args: VolumeDriverArgs) -> Self {
        Self {
            effects: Arc::new(VolumeEffectsService::new(args.facets)),
            layout_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn error(&self, kind: VolumeDriverErrorKind, op: DriverOp) -> VolumeDriverError {
        VolumeDriverError::new(kind, op)
    }

    /// Decode the stored envelope and the typed spec in one step.
    fn decoded_spec<'a>(
        &self,
        ctx: &'a ResourceContext,
        op: DriverOp,
    ) -> Result<(&'a VolumeSpecEnvelope, VolumeSpec), VolumeDriverError> {
        let envelope = ctx
            .spec::<VolumeSpecEnvelope>()
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<VolumeSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope, spec))
    }

    /// Provider check (old `validate_spec`): the Volume must select the
    /// fixed daemon-owned volume-local Provider.
    fn check_provider(&self, envelope: &VolumeSpecEnvelope, op: DriverOp) -> Result<(), VolumeDriverError> {
        match envelope.provider_ref.as_ref() {
            Some(provider_ref)
                if provider_ref.resource_type().as_str() == "Provider"
                    && provider_ref.name().as_str() == VOLUME_PROVIDER_NAME => {}
            other => {
                return Err(self
                    .error(VolumeDriverErrorKind::ProviderUnsupported, op)
                    .with_detail(
                        FailureDetail::at("spec/provider").comparison(FailureComparison::new(
                            "spec.providerRef",
                            format!("Provider/{VOLUME_PROVIDER_NAME}"),
                            other
                                .map(|reference| reference.to_canonical_string())
                                .unwrap_or_else(|| "absent".to_owned()),
                        )),
                    ))
            }
        }
        Ok(())
    }

    fn volume_ref(&self, ctx: &ResourceContext, op: DriverOp) -> Result<ResourceRef, VolumeDriverError> {
        let resource_type = ContractResourceTypeName::parse(VOLUME_TYPE_NAME.to_owned())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        let name = ResourceName::parse(&ctx.key().name)
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        Ok(ResourceRef::new(resource_type, name))
    }

    /// Derive the deterministic `VolumeBinding` children per virtiofs
    /// attachment (old `volume_children`).
    fn desired_children(
        &self,
        volume_ref: &ResourceRef,
        spec: &VolumeSpec,
        op: DriverOp,
    ) -> Result<Vec<DesiredBindingChild>, VolumeDriverError> {
        let intents = desired_binding_intents(volume_ref, spec, false).map_err(|error| {
            self.error(VolumeDriverErrorKind::ChildDerivation, op)
                .with_detail(derivation_detail(error.code()))
        })?;
        intents
            .into_iter()
            .map(|intent| {
                // Neutral binding payload only (KTD1): access mode and mount
                // intent. The envelope carries no provider extension or
                // attachment settings; the serving posture is the frozen
                // default.
                let binding = d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec::new(
                    intent.volume_ref().clone(),
                    intent.execution_ref().clone(),
                    intent.view().as_str(),
                    intent.access(),
                    intent.mount_path(),
                    d2b_contracts_resource::v3::BindingSourceDecision::new(
                        vec![d2b_contracts_resource::v3::RequestedRights::Consume],
                        d2b_contracts_resource::v3::binding::BindingArbitration::Shared,
                        vec![
                            d2b_contracts_resource::v3::BindingRealizationFacet::FilesystemPresentation,
                        ],
                    )
                    .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?,
                )
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?;
                let mut binding_spec = serde_json::to_value(&binding)
                    .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?
                    .as_object_mut()
                    .ok_or_else(|| self.error(VolumeDriverErrorKind::ChildDerivation, op))?
                    .clone();
                binding_spec.insert(
                    "providerRef".to_owned(),
                    serde_json::Value::String(BINDING_PROVIDER_REF.to_owned()),
                );
                Ok(DesiredBindingChild {
                    name: intent.name().as_str().to_owned(),
                    spec: serde_json::to_vec(&serde_json::Value::Object(binding_spec))
                        .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?,
                })
            })
            .collect()
    }

    /// Ensure every desired binding child and retire any owned child this
    /// pass no longer derives (old `reconcile_owned_children` diff, R8/R9).
    /// Each manager reply fires only after the child row commit (F1/AE1).
    async fn reconcile_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[DesiredBindingChild],
        op: DriverOp,
    ) -> Result<Vec<StoredDesiredResource>, VolumeDriverError> {
        for child in desired {
            let ensure = ChildEnsure {
                type_name: ResourceTypeName::new(VOLUME_BINDING_TYPE),
                name: child.name.clone(),
                spec: child.spec.clone(),
                metadata: Vec::new(),
            };
            ctx.ensure_child(ensure)
                .await
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        let obsolete = owned
            .iter()
            .filter(|row| {
                row.key.type_name == VOLUME_BINDING_TYPE
                    // Ownership boundary (U14): this diff owns the
                    // attachment-shaped rows it derives. A row carrying a
                    // canonical request belongs to the KTD2 admission below,
                    // which retires only what it no longer derives - two
                    // derivations over one resource type would otherwise
                    // retire each other's rows on every pass.
                    && !is_canonical_binding_row(&row.spec)
                    && !desired.iter().any(|child| child.name == row.key.name)
            })
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        for key in obsolete {
            // Obsolete child: the manager retires it and owns its own
            // teardown (endpoint -> process last), R9/F3.
            ctx.delete(&key)
                .await
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        }
        Ok(owned)
    }

    /// The canonical `VolumeBinding` rows this pass commits (U14,
    /// KTD2/KTD3).
    ///
    /// One admitted relationship becomes exactly one row, named from the
    /// KTD3 key by [`canonical_binding_children`], and each is committed
    /// through the manager-routed [`SharedProviderChildSurface`] so the row
    /// is durable before the binding's actor exists (F1/AE1).
    ///
    /// The pass is idempotent by construction: the name is derived from the
    /// relationship's identities, so a second pass over an unchanged parent
    /// re-ensures the same rows and the manager answers `Unchanged`. A
    /// relationship the admitted set no longer names is retired with its
    /// owner, so a shrinking set does not leak rows (the ownership-bounded
    /// reset).
    ///
    /// Two refusals are load-bearing here. A relationship whose KTD3 key does
    /// not name this row as its source is not this parent's relationship and
    /// is never committed under it. And an absent admission admits nothing:
    /// the pass commits nothing and retires nothing, because a missing grant
    /// is not evidence that a relationship ended. Nothing here mints an
    /// authorization or a fence of its own.
    async fn reconcile_canonical_bindings(
        &self,
        ctx: &mut ResourceContext,
        source: &VolumeBindingAdmission<'_>,
        op: DriverOp,
    ) -> Result<CanonicalBindingState, VolumeDriverError> {
        let admitted = match self.effects.admit_bindings(source).await {
            Ok(admitted) => admitted,
            Err(absent) => {
                tracing::warn!(
                    zone = %source.zone().as_str(),
                    volume = %source.volume_ref().to_canonical_string(),
                    reason = %absent,
                    "no canonical volume binding row committed: admission evidence absent"
                );
                return Ok(CanonicalBindingState::EvidenceAbsent(absent));
            }
        };
        if let Some(foreign) = admitted
            .iter()
            .find(|admitted| !names_this_source(admitted, source))
        {
            // The seam answered for a relationship this row is not the
            // source of. Committing it would mint a row under one parent
            // naming another Volume, so the pass refuses instead.
            return Err(self
                .error(VolumeDriverErrorKind::ChildDerivation, op)
                .with_detail(
                    FailureDetail::at("bindings/derive")
                        .comparison(FailureComparison::new(
                            "binding.sourceRef",
                            source.volume_ref().to_canonical_string(),
                            foreign.key().source_ref().to_canonical_string(),
                        ))
                        .with_note("the admitted relationship names another source"),
                ));
        }
        let derived = canonical_binding_children(&admitted).map_err(|error| {
            self.error(VolumeDriverErrorKind::ChildDerivation, op)
                .with_detail(binding_contract_detail(&error))
        })?;
        // The owned set is read once, before the pass mutates it: the rows
        // this pass may retire are the canonical ones its derived set no
        // longer names.
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        let obsolete = owned
            .iter()
            .filter(|row| {
                row.key.type_name == VOLUME_BINDING_TYPE
                    && is_canonical_binding_row(&row.spec)
                    && !derived.iter().any(|child| child.name == row.key.name)
            })
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        {
            let surface = ContextChildSurface::new(ctx);
            for child in &derived {
                surface
                    .ensure(ChildEnsure {
                        type_name: ResourceTypeName::new(VOLUME_BINDING_TYPE),
                        name: child.name.clone(),
                        spec: child.spec.clone(),
                        metadata: Vec::new(),
                    })
                    .await
                    .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
            }
            for key in obsolete {
                // A canonical row this pass no longer derives: the manager
                // marks it deleting and owns its teardown, R9/F3.
                surface
                    .delete(&key)
                    .await
                    .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
            }
        }
        Ok(CanonicalBindingState::Committed { relationships: derived.len() })
    }

    /// Spawn the preserved layout effect as a long effect (R5, KTD12): the
    /// mailbox never blocks on it; completion arrives as
    /// [`d2b_resource_runtime::context::EffectCompleted`].
    ///
    /// A layout report that is not `Ready` (Degraded/Pending) completes as a
    /// retryable failure rather than as a success. `Completed` re-enters the
    /// pass immediately, which would respawn the effect - and, for a Nix
    /// closure source, re-run the broker `StoreSync` the source resolution
    /// performs - at completion rate with no bound. The retryable class is
    /// the actor's documented ownership (R13): it schedules exactly one
    /// requeue after its backoff, so a degraded layout retries at a fixed
    /// interval instead of spinning.
    fn spawn_layout(
        &mut self,
        ctx: &mut ResourceContext,
        uid: ResourceUid,
        spec: VolumeSpec,
        provider: Option<serde_json::Value>,
    ) -> Result<ReconcileOutcome, VolumeDriverError> {
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let layout_ready = Arc::clone(&self.layout_ready);
        tokio::spawn(async move {
            let result = effects.ensure_layout(&uid, &spec, provider.as_ref(), None).await;
            let effect_result = match result {
                Ok(true) => {
                    layout_ready.store(true, std::sync::atomic::Ordering::SeqCst);
                    d2b_resource_runtime::context::EffectResult::Completed
                }
                // Degraded/Pending is a `NotYet`: the actor defers and
                // requeues instead of respawning the effect at completion
                // rate (issue #508).
                Ok(false) => d2b_resource_runtime::context::EffectResult::Failed(
                    DriverFailure::not_yet(
                        DriverOp::Reconcile,
                        FailureKinds::VOLUME_LAYOUT_NOT_READY,
                    )
                    .at("reconcile/layout")
                    .with_comparison(FailureComparison::new(
                        "layout.phase",
                        "Ready",
                        "Degraded/Pending",
                    )),
                ),
                Err(error) => d2b_resource_runtime::context::EffectResult::Failed(
                    DriverFailure::error(
                        DriverOp::Reconcile,
                        FailureKinds::VOLUME_LAYOUT_EFFECT_FAILED,
                        FailureClass::Retryable,
                    )
                    .at("reconcile/layout")
                    .with_comparison(FailureComparison::new(
                        "layout.effect",
                        "completed",
                        "failed",
                    ))
                    .with_note(error),
                ),
            };
            let _ = effect_sender.send(d2b_resource_runtime::context::EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        ctx.set_status(VolumeDriverStatus::EnsuringLayout);
        Ok(ReconcileOutcome::InProgress { operation })
    }
}

/// The comparison naming why deterministic child derivation refused
/// (issue #508): the attachments were not admissible.
fn derivation_detail(code: &str) -> FailureDetail {
    FailureDetail::at("children/derive")
        .comparison(FailureComparison::new(
            "volume.attachments",
            "admissible virtiofs attachments",
            "refused",
        ))
        .with_note(code)
}

/// The comparison naming why a canonical row's derivation refused: the
/// derived row name or its desired bytes are not the closed contract's.
fn binding_contract_detail(code: &BindingContractError) -> FailureDetail {
    FailureDetail::at("bindings/derive")
        .comparison(FailureComparison::new(
            "binding.row",
            "a bounded row name over the canonical request",
            "refused",
        ))
        .with_note(code.to_string())
}

/// Whether one stored child row carries this family's canonical binding
/// request (KTD2) rather than the attachment-shaped row the pre-cutover
/// derivation commits.
///
/// The two are disjoint wire shapes, so the row itself says which
/// derivation owns it: that is the durable ownership boundary between the
/// two diffs, and it holds across a restart, where an in-memory set would
/// not.
fn is_canonical_binding_row(spec: &[u8]) -> bool {
    DecodedBindingRequest::decode(VOLUME_BINDING_TYPE, spec).is_some()
}

/// Whether one admitted relationship's KTD3 key names exactly this Volume
/// as its source.
fn names_this_source(admitted: &AdmittedVolumeBinding, source: &VolumeBindingAdmission<'_>) -> bool {
    admitted.key().source_ref() == source.volume_ref()
        && admitted.key().source_uid() == source.volume_uid()
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    ResourceUid::from_bytes(bytes).map_err(|_| ())
}

#[async_trait::async_trait]
impl ResourceDriver for VolumeDriver {
    type Error = VolumeDriverError;

    fn classify_error(&self, error: &VolumeDriverError) -> DriverFailure {
        let failure = match error.kind {
            VolumeDriverErrorKind::SpecInvalid | VolumeDriverErrorKind::ProviderUnsupported => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            VolumeDriverErrorKind::ChildDerivation => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            VolumeDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            VolumeDriverErrorKind::LayoutEffect | VolumeDriverErrorKind::ChildMutation => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode plus provider reference check (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, _) = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_provider(envelope, DriverOp::Validate)?;
        Ok(())
    }

    /// Discover existing volume-local layout state on the host target
    /// (preserved recover behavior): found layout adopts, absent waits for
    /// reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (envelope, _) = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_provider(envelope, DriverOp::Recover)?;
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, DriverOp::Recover))?;
        if self.effects.has_layout(&uid) {
            // The marker proves a layout was materialized once, not that the
            // entry it names is still the volume's root: a marker that outlives
            // its root is the provider's own evidence of loss. So recovery
            // adopts the row without pinning `layout_ready` - the pass that
            // follows re-runs the layout effect, which re-derives and
            // re-materializes the root, and is idempotent on a root that is
            // still there.
            ctx.set_status(VolumeDriverStatus::ServingChildren {
                desired: 0,
                converged: false,
                canonical: CanonicalBindingState::NotReconciled,
            });
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: pass one spawns the preserved layout effect
    /// (R5); after its completion the actor re-reconciles and this pass
    /// derives the deterministic binding children and ensures each through
    /// the manager (F1), then admits the canonical KTD2 relationships and
    /// commits one row per admitted relationship through the manager-routed
    /// child surface (U14); readiness aggregates the child rows this pass
    /// converged.
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_provider(envelope, DriverOp::Reconcile)?;
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let volume_ref = self.volume_ref(ctx, DriverOp::Reconcile)?;

        if !self.layout_ready.load(std::sync::atomic::Ordering::SeqCst) {
            let provider = envelope.base.get("provider").map(|value| {
                serde_json::to_value(value).expect("canonical JSON values always serialize")
            });
            return self.spawn_layout(ctx, uid, spec, provider);
        }

        let desired = self.desired_children(&volume_ref, &spec, DriverOp::Reconcile)?;
        let owned = self
            .reconcile_children(ctx, &desired, DriverOp::Reconcile)
            .await?;
        let converged = desired.iter().all(|child| {
            owned
                .iter()
                .any(|row| row.key.type_name == VOLUME_BINDING_TYPE && row.key.name == child.name)
        });
        // The canonical pass runs last so the pass's final child state is the
        // one the KTD2 admission produced; the two diffs are fenced apart
        // above, so neither order would collide.
        let zone = ZoneId::parse(&ctx.key().zone)
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let canonical = self
            .reconcile_canonical_bindings(
                ctx,
                &VolumeBindingAdmission::new(zone, volume_ref, uid, &spec),
                DriverOp::Reconcile,
            )
            .await?;
        ctx.set_status(VolumeDriverStatus::ServingChildren {
            desired: desired.len() + canonical.committed(),
            converged,
            canonical,
        });
        // The child set is what this plane serves, and a set that did not
        // converge re-checks on the preserved cadence so a binding that drifts
        // is re-derived instead of sitting unreported. The verdict stays the
        // driver's convergence: this row's binding children chain their own
        // realization through it, and deferring the parent to `Pending` would
        // starve them.
        ctx.requeue_after(VOLUME_RESYNC);
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The call nudges each owned binding child
    /// through its own finalize-before-delete pass and requeues this pass
    /// while any child row is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::DrainPending, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown of the Volume's own layout effect (idempotent under retry,
    /// R10; the durable deleting mark is already committed). Child teardown
    /// (bindings -> endpoint -> process last) is the manager's
    /// reconcile_children diff on parent delete (R9/F3).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok((_, spec)) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing durable to clean up; converged without effects.
            return Ok(());
        };
        let Ok(uid) = resource_uid(ctx.uid()) else {
            return Ok(());
        };
        self.effects
            .remove_layout(&uid, &spec)
            .await
            .map_err(|error| {
                self.error(VolumeDriverErrorKind::LayoutEffect, DriverOp::Delete)
                    .with_detail(
                        FailureDetail::at("delete/layout")
                            .comparison(FailureComparison::new(
                                "layout.state",
                                "removed",
                                "remove failed",
                            ))
                            .with_note(error),
                    )
            })
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The resource verbs the Volume type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
/// The execution domains the Volume type can be reconciled in.
///
/// Derived from the placement contract: `Volume` names no placement anchor
/// (`PlacementAnchor::canonical_for` resolves none), so a Volume row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its containing Zone's Host. A source or attachment reference selects
/// where a share is served, never where the row itself is reconciled.
const VOLUME_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Volume driver reads while reconciling.
///
/// The driver derives every child from its own stored spec plus the layout
/// intents that spec admits, so it reads no other row.
const VOLUME_READS: &[WellKnownType] = &[];

/// The Volume type's driver declaration.
///
/// `Volume` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot serve
/// the converted volume shapes without it, so it must be registered before
/// the plane opens. The type is not exportable: `ResourceExport` admits only
/// qualified `*.d2bus.org.*Service` types, so a volume can never be an
/// export subject. The driver serves no broker operations and contributes no
/// startup steps; the `VolumeBinding` children it mints are declared in
/// [`VOLUME_CREATIONS`].
///
/// U7: the driver's effects are this crate's own implementation
/// ([`VolumeEffectsService`]) built from the daemon-supplied facet set -
/// the construction site holds no externally built port (R2) - and the
/// family's declared effects service ([`VOLUME_EFFECTS_SERVICE`]) rides
/// the declaration, so a zone that cannot host it refuses startup by name
/// (R5).
pub fn volume_descriptor(args: VolumeDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::VOLUME,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: VOLUME_EXECUTION_DOMAINS,
        exportable: false,
        reads: VOLUME_READS,
        operations: &[],
        creations: VOLUME_CREATIONS,
        startup: &[],
        services: &[VOLUME_EFFECTS_SERVICE],
        decoder: volume_spec_decoder(),
        factory: Arc::new(VolumeDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted layout port and a recording
// manager endpoint (ordering observed as the manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_provider_toolkit::testing::fakes::RecordingManagerEndpoint;
    use d2b_resource_runtime::context::{RequeueId, RequeueScheduler, ResourceContext};
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{DriverOp, FailureClass};
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, StoredDesiredResource,
    };
    use super::{VolumeDriverArgs, VolumeDriverFactory, volume_spec_decoder};
    use crate::test_support::{RecordingRuntime, recording_facets};

    // -- fakes ---------------------------------------------------------------
    /// Records nothing: the Volume flows schedule their re-checks on
    /// [`VOLUME_RESYNC`], and the tests assert the outcome that carries them.
    struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    /// A minimal valid Volume spec: one localPath source, one write-capable
    /// view, one virtiofs attachment at `mount_path`.
    fn volume_spec_json(mount_path: &str, extra_attachment: bool) -> serde_json::Value {
        let mut attachments = vec![serde_json::json!({
            "executionRef": "Guest/guest-a",
            "transport": "virtiofs",
            "view": "root",
            "access": "read-only",
            "mountPath": mount_path,
            "settings": {},
        })];
        if extra_attachment {
            attachments.push(serde_json::json!({
                "executionRef": "Guest/guest-b",
                "transport": "virtiofs",
                "view": "root",
                "access": "read-only",
                "mountPath": "/mnt/second",
                "settings": {},
            }));
        }
        serde_json::json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "policy-default" },
            },
            "kind": "durable",
            "layout": [],
            "views": { "root": { "path": "data", "rights": ["read", "traverse", "write"] } },
            "attachments": attachments,
        })
    }

    fn spec_bytes(mount_path: &str, extra_attachment: bool) -> Vec<u8> {
        let mut spec = volume_spec_json(mount_path, extra_attachment);
        let object = spec.as_object_mut().expect("spec object");
        object.insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-local".to_owned()),
        );
        serde_json::to_vec(&spec).expect("canonical volume spec")
    }

    fn test_row(spec: &[u8]) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Volume", "data"),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec.to_vec(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        effects: tokio::sync::mpsc::UnboundedReceiver<
            d2b_resource_runtime::context::EffectCompleted,
        >,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
        let (effects_tx, effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            volume_spec_decoder(),
            Arc::new(manager),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        Fixture { ctx, effects: effects_rx }
    }

    async fn driver(runtime: Arc<RecordingRuntime>) -> Box<dyn DynResourceDriver> {
        let factory = VolumeDriverFactory::new(VolumeDriverArgs {
            facets: recording_facets(runtime),
        });
        factory
            .create(&ResourceKey::new("work", "Volume", "data"))
            .await
    }

    async fn reconcile_to_children(
        d: &mut Box<dyn DynResourceDriver>,
        f: &mut Fixture,
    ) -> ReconcileOutcome {
        // Pass one spawns the layout effect; the mailbox never blocks (R5).
        let first = d.reconcile(&mut f.ctx).await.expect("reconcile one");
        assert!(matches!(first, ReconcileOutcome::InProgress { .. }), "{first:?}");
        let completed = f.effects.recv().await.expect("typed completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));
        // The actor re-reconciles on the effect completion.
        d.reconcile(&mut f.ctx).await.expect("reconcile two")
    }

    // -- ensure: layout effect, then children ---------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn ensure_creates_binding_children_after_the_layout_effect() {
        let fake = RecordingRuntime::new();
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.validate(&mut f.ctx).await.expect("validate");
        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "no layout yet: nothing to adopt"
        );

        let outcome = reconcile_to_children(&mut d, &mut f).await;
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let order = manager.call_order();
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "ensure-layout"],
            "recover probe then exactly one layout effect"
        );
        assert!(
            order.iter().any(|entry| entry.starts_with("ensure:VolumeBinding/")),
            "binding child ensured through ctx, order: {order:?}"
        );
        // F1: every child ensure is recorded (committed) BEFORE its spawn
        // notification.
        let binding_ensure = order
            .iter()
            .position(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .expect("ensure recorded");
        let _binding_spawn = order
            .iter()
            .position(|entry| entry.starts_with("spawned:VolumeBinding/"))
            .expect("spawn notification recorded");
        assert!(binding_ensure < binding_spawn_index(&order, binding_ensure));
        let status = f
            .ctx
            .status::<super::VolumeDriverStatus>()
            .expect("status");
        assert_eq!(
            *status,
            super::VolumeDriverStatus::ServingChildren {
                desired: 1,
                converged: true,
                canonical: super::CanonicalBindingState::EvidenceAbsent(
                    BindingEvidenceAbsent::new(vec![
                        BindingAdmissionEvidence::Authorization,
                        BindingAdmissionEvidence::FreshnessFence,
                    ]),
                ),
            }
        );
    }

    fn binding_spawn_index(order: &[String], ensure_index: usize) -> usize {
        order[ensure_index..]
            .iter()
            .position(|entry| entry.starts_with("spawned:"))
            .map(|offset| ensure_index + offset)
            .expect("spawn notification after ensure")
    }

    // -- adoption: an existing layout is adopted, then re-validated ------------

    /// §36 recovery (`existing volume/mount is adopted` + `missing desired
    /// resource is recreated`): a fresh driver - the post-restart in-memory
    /// state - adopts the layout the host already holds, and the pass that
    /// follows re-validates it through the same idempotent `ensure-layout`
    /// effect instead of trusting the marker alone: a marker that outlives its
    /// root is the provider's own evidence of loss, and a pass that never
    /// re-reads the layout could not tell the two apart. Adoption still never
    /// mints a duplicate: the effect ensures the one farm, and the
    /// deterministic binding child is re-attached rather than re-created.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_the_existing_layout_and_revalidates_it() {
        let fake = RecordingRuntime::new();
        let manager = RecordingManagerEndpoint::new();
        let mut first = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(fake.clone()).await;
        // The pre-restart lifetime realizes the layout and its binding child.
        let outcome = reconcile_to_children(&mut d, &mut first).await;
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        assert!(fake.ready.load(std::sync::atomic::Ordering::SeqCst), "the host holds the layout");

        // Restart: a fresh driver over the same host layout state.
        let mut restarted = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut adopted = driver(fake.clone()).await;
        assert_eq!(
            adopted.recover(&mut restarted.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the existing layout is adopted, not recreated"
        );
        let outcome = reconcile_to_children(&mut adopted, &mut restarted).await;
        assert_eq!(outcome, ReconcileOutcome::Satisfied);

        let layout_effects = fake
            .call_order()
            .iter()
            .filter(|call| **call == "ensure-layout")
            .count();
        assert_eq!(
            layout_effects, 2,
            "the adoption pass re-validates the layout through the idempotent ensure"
        );
        let binding_ensures = manager
            .call_order()
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        assert_eq!(binding_ensures, 2, "the adoption pass re-attaches the same binding child");
        assert_eq!(
            manager.rows().len(),
            1,
            "re-attaching the deterministic child never mints a duplicate row"
        );
    }

    // -- degraded layout: one effect per pass, retry owned by the actor --------

    /// A Degraded/Pending layout report (`Ok(false)`) must not complete as a
    /// success: `Completed` re-enters the pass immediately, which respawns the
    /// effect - and, for a Nix closure source, the broker `StoreSync` the
    /// source resolution performs - at completion rate with no bound. The
    /// report is a retryable failure, so the actor's one backoff requeue owns
    /// the retry and the effect runs at most once per pass.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn degraded_layout_reports_one_retryable_failure_per_pass() {
        let fake = RecordingRuntime::degraded();
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(fake.clone()).await;

        let outcome = d.reconcile(&mut f.ctx).await.expect("reconcile spawns the effect");
        assert!(matches!(outcome, ReconcileOutcome::InProgress { .. }), "{outcome:?}");
        let completed = f.effects.recv().await.expect("typed completion");
        let failure = match completed.result {
            d2b_resource_runtime::context::EffectResult::Failed(failure) => failure,
            other => panic!("a degraded layout must not complete as success: {other:?}"),
        };
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert_eq!(
            fake.call_order(),
            vec!["ensure-layout"],
            "exactly one layout effect per pass; the driver never re-spawns on its own"
        );
        assert!(
            !manager
                .call_order()
                .iter()
                .any(|entry| entry.starts_with("ensure:VolumeBinding/")),
            "a degraded layout derives no binding children"
        );

        // The actor's requeue delivers the next pass; exactly one more effect.
        let outcome = d.reconcile(&mut f.ctx).await.expect("requeued pass");
        assert!(matches!(outcome, ReconcileOutcome::InProgress { .. }), "{outcome:?}");
        let _ = f.effects.recv().await.expect("typed completion");
        assert_eq!(
            fake.call_order(),
            vec!["ensure-layout", "ensure-layout"],
            "one layout effect per reconcile pass"
        );

        // Once the layout is Ready the same port converges without another
        // effect on the pass that observes it.
        fake.degraded.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = d.reconcile(&mut f.ctx).await.expect("reconcile after ready");
        let _ = f.effects.recv().await.expect("ready completion");
        assert_eq!(d.reconcile(&mut f.ctx).await.expect("children pass"), ReconcileOutcome::Satisfied);
    }

    // -- deterministic child identity -----------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn same_parent_and_attachment_derive_the_same_child_key() {
        let manager = RecordingManagerEndpoint::new();
        {
            let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
            let mut d = driver(RecordingRuntime::new()).await;
            reconcile_to_children(&mut d, &mut f).await;
        }
        let first = manager.call_order();
        // Same parent + attachment -> exactly one child key, ensured again
        // as Unchanged (no duplicate identity, no churn).
        {
            let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
            let mut d = driver(RecordingRuntime::new()).await;
            d.recover(&mut f.ctx).await.expect("recover");
            reconcile_to_children(&mut d, &mut f).await;
        }
        let second = manager.call_order();
        let ensure_count = first
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        let unchanged_count = second
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        assert_eq!(ensure_count, 1);
        assert_eq!(unchanged_count, 2, "same attachment -> same child key, no churn");
        let child_name = first
            .iter()
            .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/"))
            .expect("binding name recorded")
            .to_owned();
        assert_eq!(
            second
                .iter()
                .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/")),
            Some(child_name.as_str()),
            "child identity is deterministic"
        );
    }

    // -- parent spec change: retire obsolete, retain matching ------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn parent_spec_change_retires_obsolete_children_and_retains_matching() {
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(RecordingRuntime::new()).await;
        d.recover(&mut f.ctx).await.expect("recover");
        reconcile_to_children(&mut d, &mut f).await;
        let first_child = manager
            .call_order()
            .iter()
            .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/"))
            .expect("first binding name")
            .to_owned();

        // Grow the spec: a second attachment. The first child must be
        // retained, a second child created, nothing deleted.
        let grown = test_row(&spec_bytes("/mnt/data", true));
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut ctx2 = ResourceContext::new(
            grown,
            volume_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        d.reconcile(&mut ctx2).await.expect("reconcile grown");
        let ensured = manager
            .call_order()
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        assert_eq!(ensured, 3, "first retained + two passes over two children");
        assert!(
            !manager
                .call_order()
                .iter()
                .any(|entry| entry.starts_with("delete:")),
            "matching child retained, no delete on growth"
        );

        // Shrink the spec back: the second child is retired through the
        // manager; the matching child stays.
        let shrunk = test_row(&spec_bytes("/mnt/data", false));
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut ctx3 = ResourceContext::new(
            shrunk,
            volume_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        d.reconcile(&mut ctx3).await.expect("reconcile shrunk");
        let deletes = manager
            .call_order()
            .iter()
            .filter(|entry| entry.starts_with("delete:VolumeBinding/"))
            .cloned()
            .collect::<Vec<String>>();
        assert_eq!(deletes.len(), 1, "exactly the obsolete child retired");
        assert!(
            !deletes[0].contains(&first_child),
            "matching child never retired, order: {:?}",
            manager.call_order()
        );
    }

    // -- finalize: owned children retire before the layout teardown (F3) -----

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_layout_teardown() {
        let manager = RecordingManagerEndpoint::new();
        manager.seed_owned(ResourceKey::new("work", "VolumeBinding", "vol-binding-0"));
        let fake = RecordingRuntime::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(fake.clone()).await;

        // A live owned child: the pass requeues and the Volume's own layout
        // teardown does not run.
        let failure = d.finalize(&mut f.ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            manager.call_order(),
            vec!["delete:VolumeBinding/vol-binding-0".to_owned()],
            "the owned child is nudged through its own finalize-before-delete pass"
        );
        assert!(fake.call_order().is_empty(), "the layout teardown has not run");

        // The manager removed the retired child row: the same pass converges.
        d.finalize(&mut f.ctx).await.expect("converged once the child retired");
        assert!(fake.call_order().is_empty(), "finalize runs no layout effect");
    }

    // -- delete: the Volume's own layout effect -------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_removes_the_volume_layout_exactly() {
        let fake = RecordingRuntime::new();
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager);
        let mut d = driver(fake.clone()).await;
        d.recover(&mut f.ctx).await.expect("recover");
        d.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "remove-layout"],
            "recover probe then drain-finalizer-gated cleanup"
        );
        // Retry is idempotent (R10).
        d.delete(&mut f.ctx).await.expect("delete retry");
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "remove-layout", "remove-layout"]
        );
    }

    // -- spec guards -----------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn wrong_provider_is_rejected_at_validate() {
        let mut spec = volume_spec_json("/mnt/data", false);
        spec.as_object_mut().expect("spec object").insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-virtiofs".to_owned()),
        );
        let bytes = serde_json::to_vec(&spec).expect("spec");
        let mut f = fixture(test_row(&bytes), RecordingManagerEndpoint::new());
        let mut d = driver(RecordingRuntime::new()).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal, "provider mismatch is terminal");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn malformed_spec_decodes_to_a_terminal_failure() {
        let bytes = serde_json::json!({
            "providerRef": "Provider/volume-local",
            "nonsense": true,
        })
        .to_string()
        .into_bytes();
        let mut f = fixture(test_row(&bytes), RecordingManagerEndpoint::new());
        let mut d = driver(RecordingRuntime::new()).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    // -- canonical binding children (U14) ------------------------------------

    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::volume::{AttachmentAccess, VolumeSpec};
    use d2b_contracts_resource::v3::{
        BindingAuthorization, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
        DesiredDigest, DesiredRevision, FreshnessTuple, ResourceUid, StoreIncarnation,
        VolumeBindingRequest, VolumePresentation, ZoneId, canonical_json_bytes,
    };
    use d2b_provider_volume_local::{
        AdmittedVolumeBinding, VolumeAdmissionGrant, VolumeAdmissionSource, VolumeConsumerRequest,
        admit_consumer_requests,
    };
    use d2b_resource_runtime::relations::DecodedBindingRequest;

    use super::{
        CanonicalBindingState, VOLUME_BINDING_TYPE, VolumeDriverStatus, canonical_binding_children,
        canonical_binding_row,
    };
    use crate::facets::{BindingAdmissionEvidence, BindingEvidenceAbsent};

    const VOLUME_UID_VALUE: &str = "6f9619ff-8b86-4d01-b42d-00cf4fc964ff";
    const PROCESS_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
    const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";

    /// The durable uid the reconciled test row carries: the identity its
    /// relationships are admitted against, because a KTD3 key names the
    /// source's store identity and not its reference alone.
    const ROW_UID: [u8; 16] = [0x42; 16];

    /// A conformant graph-era Volume: declared views, no attachment list.
    /// Its relationships arrive through the admission seam instead.
    fn canonical_graph_volume_value() -> serde_json::Value {
        serde_json::json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
            },
            "kind": "durable",
            "layout": [],
            "views": {
                "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            },
        })
    }

    fn canonical_graph_volume() -> VolumeSpec {
        serde_json::from_value(canonical_graph_volume_value())
            .expect("conformant Volume spec")
    }

    /// The same spec as the row the driver reconciles: the stored envelope
    /// carries the Provider reference the driver checks.
    fn canonical_row_bytes() -> Vec<u8> {
        let mut spec = canonical_graph_volume_value();
        spec.as_object_mut()
            .expect("spec object")
            .insert(
                "providerRef".to_owned(),
                serde_json::Value::String("Provider/volume-local".to_owned()),
            );
        serde_json::to_vec(&spec).expect("canonical volume spec")
    }

    /// The reconciled row's own store identity, as the authority spells it.
    fn row_uid() -> ResourceUid {
        ResourceUid::from_bytes(&ROW_UID).expect("canonical row uid")
    }

    /// One admitted relationship per consumer kind, admitted through the one
    /// source-side path against exactly the identity the source carries.
    fn admitted_source(
        source_ref: &str,
        source_uid: &str,
        spec: &VolumeSpec,
    ) -> Vec<AdmittedVolumeBinding> {
        let zone = ZoneId::parse("work").expect("zone");
        let volume_ref =
            d2b_contracts_resource::v3::ResourceRef::parse(source_ref).expect("volume");
        let volume_uid = ResourceUid::parse(source_uid).expect("uid");
        let support = BindingRealizationSupport::new(vec![
            BindingRealizationFacet::FilesystemPresentation,
            BindingRealizationFacet::ConsumerDeviceSlot,
        ])
        .expect("support set");
        let authorization = BindingAuthorization::granted();
        let fence: Vec<FreshnessTuple> = [
            (source_ref, source_uid),
            ("Process/worker", PROCESS_UID),
            ("Guest/work-vm", GUEST_UID),
        ]
        .into_iter()
        .map(|(name, identity)| {
            FreshnessTuple::new(
                zone.clone(),
                StoreIncarnation::parse("store-one").expect("incarnation"),
                d2b_contracts_resource::v3::ResourceRef::parse(name).expect("reference"),
                ResourceUid::parse(identity).expect("uid"),
                DesiredRevision::INITIAL,
                DesiredDigest::of(name.as_bytes()),
            )
        })
        .collect();
        let grant = VolumeAdmissionGrant::new(&support, &authorization, &fence);
        let source =
            VolumeAdmissionSource::new(&zone, &volume_ref, &volume_uid, spec, false, &grant);
        let requests = [
            VolumeConsumerRequest::new(
                ResourceUid::parse(PROCESS_UID).expect("uid"),
                VolumeBindingRequest::new(
                    volume_ref.clone(),
                    d2b_contracts_resource::v3::ResourceRef::parse("Process/worker")
                        .expect("consumer"),
                    BindingSlot::parse("work").expect("slot"),
                    BoundedToken::parse("controller").expect("view"),
                    AttachmentAccess::ReadWrite,
                    VolumePresentation::filesystem("/srv/work").expect("destination"),
                )
                .expect("canonical request"),
            ),
            VolumeConsumerRequest::new(
                ResourceUid::parse(GUEST_UID).expect("uid"),
                VolumeBindingRequest::new(
                    volume_ref.clone(),
                    d2b_contracts_resource::v3::ResourceRef::parse("Guest/work-vm")
                        .expect("consumer"),
                    BindingSlot::parse("state").expect("slot"),
                    BoundedToken::parse("controller").expect("view"),
                    AttachmentAccess::ReadOnly,
                    VolumePresentation::block_device(1).expect("device slot"),
                )
                .expect("canonical request"),
            ),
        ];
        admit_consumer_requests(&source, &requests).expect("admitted")
    }

    /// The admitted set for the source identity the derivation tests pin.
    fn admitted_set() -> Vec<AdmittedVolumeBinding> {
        admitted_source("Volume/state", VOLUME_UID_VALUE, &canonical_graph_volume())
    }

    /// The admitted set the seam answers with for the row the driver
    /// reconciles: the same two consumer kinds, against that row's own
    /// identity.
    fn admitted_for_row() -> Vec<AdmittedVolumeBinding> {
        admitted_source("Volume/data", row_uid().as_str(), &canonical_graph_volume())
    }

    /// The manager key one derived canonical row is committed under.
    fn binding_key(name: &str) -> ResourceKey {
        ResourceKey::new("work", VOLUME_BINDING_TYPE, name)
    }

    /// The committed `VolumeBinding` rows, ordered by name.
    fn committed_bindings(manager: &RecordingManagerEndpoint) -> Vec<StoredDesiredResource> {
        let mut rows: Vec<StoredDesiredResource> = manager
            .rows()
            .into_iter()
            .filter(|row| row.key.type_name == VOLUME_BINDING_TYPE)
            .collect();
        rows.sort_by(|left, right| left.key.name.cmp(&right.key.name));
        rows
    }

    /// The reconciliation facts the assertions read: (name, generation).
    fn generations(rows: &[StoredDesiredResource]) -> Vec<(String, u64)> {
        rows.iter().map(|row| (row.key.name.clone(), row.generation)).collect()
    }

    // The producing half (U14): the pass a real verb owns commits what the
    // seam admitted, a second pass over an unchanged parent commits nothing
    // new, and what the admitted set no longer names is retired.

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_admitted_relationship_commits_one_row_a_second_pass_does_not_duplicate() {
        let fake = RecordingRuntime::new();
        fake.set_admitted(admitted_for_row()).await;
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&canonical_row_bytes()), manager.clone());
        let mut d = driver(fake.clone()).await;

        assert_eq!(reconcile_to_children(&mut d, &mut f).await, ReconcileOutcome::Satisfied);

        let derived = canonical_binding_children(&admitted_for_row()).expect("derived rows");
        let committed = committed_bindings(&manager);
        assert_eq!(committed.len(), derived.len(), "one row per admitted relationship");
        for (child, relationship) in derived.iter().zip(&admitted_for_row()) {
            let row = manager
                .row(&binding_key(&child.name))
                .unwrap_or_else(|| panic!("{} is committed under its KTD3 name", child.name));
            assert_eq!(row.spec, child.spec, "the committed row is the consumer's request");
            assert_eq!(row.owner_uid, Some(ROW_UID), "the deriving Volume owns the row");
            // The committed row is what the graph reads back: the manager's
            // own relation-index decoder resolves it to this exact
            // relationship, with the consumer slot and rights the admission
            // decided.
            let decoded = DecodedBindingRequest::decode(VOLUME_BINDING_TYPE, &row.spec)
                .unwrap_or_else(|| panic!("{} is not an indexable relationship", child.name));
            assert_eq!(decoded.source_ref(), relationship.request().source_ref());
            assert_eq!(decoded.consumer_ref(), relationship.request().consumer_ref());
            assert_eq!(decoded.slot(), relationship.request().slot());
            assert_eq!(decoded.rights(), relationship.request().requested_rights());
            assert_eq!(decoded.fingerprint(), &relationship.request().fingerprint());
        }
        assert_eq!(
            *f.ctx.status::<VolumeDriverStatus>().expect("status"),
            VolumeDriverStatus::ServingChildren {
                desired: derived.len(),
                converged: true,
                canonical: CanonicalBindingState::Committed { relationships: derived.len() },
            }
        );

        // A second pass over an unchanged parent re-ensures the same rows, so
        // the manager answers `Unchanged`: no row is rewritten and no second
        // row is minted.
        let before = generations(&committed);
        let passes = fake.admission_passes();
        let calls = manager.call_order().len();
        assert_eq!(d.reconcile(&mut f.ctx).await.expect("second pass"), ReconcileOutcome::Satisfied);
        assert_eq!(fake.admission_passes(), passes + 1, "each pass asks the seam again");
        let after = committed_bindings(&manager);
        assert_eq!(generations(&after), before, "an unchanged parent rewrites no row");
        assert_eq!(after.len(), derived.len(), "no second row is minted");
        assert_eq!(
            manager.call_order()[calls..]
                .iter()
                .filter(|call| call.starts_with("delete:"))
                .count(),
            0,
            "an unchanged parent retires nothing: the attachment-shaped diff leaves the \
             canonical rows it does not own alone"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_relationship_the_admitted_set_no_longer_names_is_retired_with_its_owner() {
        let fake = RecordingRuntime::new();
        fake.set_admitted(admitted_for_row()).await;
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&canonical_row_bytes()), manager.clone());
        let mut d = driver(fake.clone()).await;
        assert_eq!(reconcile_to_children(&mut d, &mut f).await, ReconcileOutcome::Satisfied);

        // The Guest withdraws its block presentation: one relationship still
        // admits, one no longer does.
        let admitted = admitted_for_row();
        let kept = admitted
            .iter()
            .find(|admitted| admitted.request().consumer_ref().name().as_str() == "worker")
            .expect("the filesystem relationship")
            .clone();
        let dropped = admitted
            .iter()
            .find(|admitted| admitted.request().consumer_ref().name().as_str() == "work-vm")
            .expect("the device relationship");
        let dropped_name = canonical_binding_row(dropped).expect("row").name().as_str().to_owned();
        let kept_name =
            canonical_binding_row(&kept).expect("row").name().as_str().to_owned();
        let kept_before = manager
            .row(&binding_key(&kept_name))
            .expect("the kept row is committed")
            .generation;
        fake.set_admitted(vec![kept]).await;

        let calls = manager.call_order().len();
        assert_eq!(d.reconcile(&mut f.ctx).await.expect("shrunk pass"), ReconcileOutcome::Satisfied);
        assert!(
            manager.row(&binding_key(&dropped_name)).is_none(),
            "a row the admitted set no longer names is retired with its owner"
        );
        assert_eq!(
            manager.call_order()[calls..]
                .iter()
                .filter(|call| call.starts_with("delete:"))
                .cloned()
                .collect::<Vec<String>>(),
            vec![format!("delete:VolumeBinding/{dropped_name}")],
            "exactly the row the admission dropped is retired"
        );
        let kept_row = manager
            .row(&binding_key(&kept_name))
            .expect("the relationship that still admits keeps its row");
        assert_eq!(kept_row.generation, kept_before, "the surviving row is not rewritten");
        assert_eq!(
            *f.ctx.status::<VolumeDriverStatus>().expect("status"),
            VolumeDriverStatus::ServingChildren {
                desired: 1,
                converged: true,
                canonical: CanonicalBindingState::Committed { relationships: 1 },
            }
        );
    }

    // The negative cases: no evidence commits no row, and no evidence
    // retires none.

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn without_admission_evidence_no_row_is_committed_and_a_committed_one_survives() {
        let fake = RecordingRuntime::new();
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&canonical_row_bytes()), manager.clone());
        let mut d = driver(fake.clone()).await;

        assert_eq!(reconcile_to_children(&mut d, &mut f).await, ReconcileOutcome::Satisfied);
        assert!(committed_bindings(&manager).is_empty(), "no evidence, no committed row");
        assert!(
            !manager.call_order().iter().any(|call| call.starts_with("ensure:VolumeBinding/")),
            "the pass never reaches the child surface without evidence"
        );
        assert_eq!(
            *f.ctx.status::<VolumeDriverStatus>().expect("status"),
            VolumeDriverStatus::ServingChildren {
                desired: 0,
                converged: true,
                canonical: CanonicalBindingState::EvidenceAbsent(BindingEvidenceAbsent::new(vec![
                    BindingAdmissionEvidence::Authorization,
                    BindingAdmissionEvidence::FreshnessFence,
                ])),
            }
        );

        // With evidence on the seam, the relationships commit.
        fake.set_admitted(admitted_for_row()).await;
        assert_eq!(d.reconcile(&mut f.ctx).await.expect("evidenced pass"), ReconcileOutcome::Satisfied);
        let committed = generations(&committed_bindings(&manager));
        assert_eq!(committed.len(), 2);

        // Losing the evidence is not evidence that a relationship ended: the
        // pass commits nothing new and retires nothing.
        fake.withdraw_evidence().await;
        assert_eq!(d.reconcile(&mut f.ctx).await.expect("unevidenced pass"), ReconcileOutcome::Satisfied);
        assert_eq!(
            generations(&committed_bindings(&manager)),
            committed,
            "an absent grant is not evidence that a relationship ended"
        );
        assert!(
            !manager.call_order().iter().any(|call| call.starts_with("delete:VolumeBinding/")),
            "no owned row is retired without an admission that dropped it"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_relationship_naming_another_source_is_refused_instead_of_committed_here() {
        let fake = RecordingRuntime::new();
        // The seam answers for a relationship this row is not the source of.
        fake.set_admitted(admitted_set()).await;
        let manager = RecordingManagerEndpoint::new();
        let mut f = fixture(test_row(&canonical_row_bytes()), manager.clone());
        let mut d = driver(fake.clone()).await;

        let first = d.reconcile(&mut f.ctx).await.expect("reconcile spawns the effect");
        assert!(matches!(first, ReconcileOutcome::InProgress { .. }), "{first:?}");
        let _ = f.effects.recv().await.expect("typed completion");
        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("a relationship for another source is refused");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(
            committed_bindings(&manager).is_empty(),
            "a row naming another source is never minted under this parent"
        );
    }

    #[test]
    fn one_admitted_relationship_commits_exactly_one_canonical_row() {
        let admitted = admitted_set();
        let children = canonical_binding_children(&admitted).expect("derived rows");
        assert_eq!(children.len(), admitted.len());
        for (child, relationship) in children.iter().zip(&admitted) {
            // The committed bytes ARE the consumer's request, so the row the
            // graph reads back is the declaration that was admitted.
            let decoded = DecodedBindingRequest::decode("VolumeBinding", &child.spec)
                .expect("the committed row is a canonical request");
            assert_eq!(decoded.consumer_ref(), relationship.request().consumer_ref());
            assert_eq!(decoded.slot(), relationship.request().slot());
            assert_eq!(
                decoded.fingerprint(),
                &relationship.request().fingerprint()
            );
            let rendered: serde_json::Value =
                serde_json::from_slice(&canonical_json_bytes(relationship.request()).expect("bytes"))
                    .expect("canonical request value");
            let committed: serde_json::Value =
                serde_json::from_slice(&child.spec).expect("committed row value");
            // Only the presentation differs in shape; the rest is identical.
            assert_eq!(committed["sourceRef"], rendered["sourceRef"]);
            assert_eq!(committed["consumerRef"], rendered["consumerRef"]);
            assert_eq!(committed["slot"], rendered["slot"]);
            assert_eq!(committed["view"], rendered["view"]);
            assert_eq!(committed["access"], rendered["access"]);
            assert_eq!(committed["presentation"], rendered["presentation"]);
        }
    }

    #[test]
    fn a_relationship_keeps_one_row_name_across_passes_and_consumers() {
        let admitted = admitted_set();
        let first = canonical_binding_children(&admitted).expect("derived rows");
        // Deriving again from the same admitted set changes nothing, so a
        // restart re-ensures the same rows instead of churning identities.
        let second = canonical_binding_children(&admitted).expect("derived rows");
        assert_eq!(first, second);
        // Distinct relationships - here two different consumer kinds - never
        // collide on one row name.
        let names: std::collections::BTreeSet<&str> = first
            .iter()
            .map(|child| child.name.as_str())
            .collect();
        assert_eq!(names.len(), first.len());
    }

}
