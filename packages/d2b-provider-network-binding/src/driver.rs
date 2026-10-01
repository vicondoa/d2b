//! The `NetworkBinding` resource driver: the row-side conversion of one
//! consumer's membership on a Network's shared fabric.
//!
//! One Network's host fabric - its bridges, routes, ownership markers,
//! NetworkManager policy, and single ownership-scoped firewall projection - is
//! realized once per `(Zone, Network, execution target)` by the Network
//! provider. This driver does not realize it and does not re-admit it: it
//! serves one consumer's membership row and drives the four operations that
//! membership's realization has - observe, join, drain, release - through the
//! driver effect port.
//!
//! # Conversion mapping
//!
//! - `describe` -> [`binding_descriptor`] registration under `NetworkBinding`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - `reconcile` -> [`ResourceDriver::reconcile`].
//! - `drain` -> [`ResourceDriver::pre_drain`].
//! - `finalize_binding` -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only).
//!
//! # What each verb guarantees
//!
//! `validate` names one exact Network and one exact execution target, refuses
//! a row outside this driver's Zone, and refuses a membership that is already
//! realized on a superseded fabric generation: a fabric the Network has since
//! replaced is the Network's own teardown to perform, never this row's to
//! re-derive across.
//!
//! `reconcile` joins the membership rather than assuming it. The pass observes
//! first; a membership the fabric already holds under the current generation is
//! adopted without a second join, and anything else is joined, which the port
//! keeps idempotent so a retry never mints a second interface.
//!
//! `recover` adopts the pre-restart incarnation only when the observation
//! proves the membership is still joined under the current generation with its
//! presented interface realized - it observes and never joins, so a restart
//! cannot duplicate the interface a join already created.
//!
//! `pre_drain` fences new use and drains outstanding use before anything is
//! removed; `delete` then releases the membership, retaining the fabric while
//! another member still uses it, and is idempotent under retry.
//!
//! Everything the driver needs from outside arrives through
//! [`NetworkBindingDriverEffects`]: the composition root supplies the four
//! declared facets and the factory builds the port from them (R2), so the
//! family carries no host state. The Zone's store-assigned identity is factory
//! wiring too: it is a zone-authority input folded by the plane from the Zone
//! authority path, never read from the spec store.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::resource_status::StatusCode;
use d2b_contracts_resource::v3::{
    BindingConsumerKind, IfName, NetworkIfRole, NetworkPresentation, ResourceGeneration,
    ResourceRef, ResourceSpec, ResourceUid, ZoneId, ZoneRevision, derive_network_ifname,
    network_binding::NetworkBindingSpec,
};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, DriverDescriptor, WellKnownType,
};

use crate::effects_service::NetworkBindingEffectsService;

/// The one resource type this factory serves.
pub const NETWORK_BINDING_TYPE_NAME: &str = "NetworkBinding";

/// The serving Provider a `NetworkBinding` row names in `spec.providerRef`.
///
/// The membership is realized on the fabric the Network provider owns, so the
/// row names that Provider; a row naming any other Provider is a relationship
/// this driver does not serve.
pub const NETWORK_BINDING_PROVIDER_REF: &str = "Provider/network-local";

/// The ResourceType of the row this binding names as its source.
const NETWORK_TYPE: &str = "Network";

/// The children this driver mints.
///
/// A membership is realized on the source's own fabric, not on a row of its
/// own: the interface, the routes, and the per-membership firewall entry are
/// host state the Network provider owns. This driver therefore declares no
/// child creation and mints no child row.
pub const NETWORK_BINDING_CREATIONS: &[ChildCreation] = &[];

/// The resource types the driver reads while reconciling.
///
/// The `Network` row supplies the fabric identity - its store uid and its
/// committed generation - and the target row (`Host` or `Guest`) supplies the
/// consumer identity the membership's interface is derived from.
pub const NETWORK_BINDING_READS: &[WellKnownType] =
    &[WellKnownType::NETWORK, WellKnownType::HOST, WellKnownType::GUEST];

/// The execution domains the `NetworkBinding` type can be reconciled in.
///
/// The binding row carries no execution anchor of its own: the placement
/// contract resolves none for it, so the plane reconciles it on its containing
/// Zone's Host. The row's `executionRef` names the target that consumes the
/// membership, never where the membership row itself is reconciled.
const NETWORK_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The re-check cadence while the membership is not yet realized.
///
/// A join returns before the presented interface necessarily exists, so a pass
/// that has not converged re-checks on a timer rather than depending on a watch
/// delivery that may never come.
const NETWORK_BINDING_RESYNC: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// The membership the effect port acts on
// ---------------------------------------------------------------------------

/// What the shared fabric currently holds for one consumer's membership.
///
/// `fabric_generation` is `None` only when the fabric holds no membership for
/// this consumer at all; a membership realized under a generation other than
/// the Network row's committed one is the stale case, and the driver never
/// treats it as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FabricMembershipState {
    /// Whether the fabric currently holds this consumer's membership.
    pub joined: bool,
    /// The Network generation the held membership was realized under.
    pub fabric_generation: Option<ResourceGeneration>,
    /// Whether the presented interface exists on the consumer's side.
    pub interface_ready: bool,
}

impl FabricMembershipState {
    /// The state of a consumer that holds no membership on the fabric.
    pub const fn absent() -> Self {
        Self {
            joined: false,
            fabric_generation: None,
            interface_ready: false,
        }
    }

    /// Whether the fabric holds this membership under exactly `generation`.
    ///
    /// This is the whole convergence predicate: a membership held under an
    /// older generation is not the row's membership on the Network's current
    /// fabric, whatever else the observation reports.
    pub const fn is_held_under(&self, generation: ResourceGeneration) -> bool {
        self.joined && matches!(self.fabric_generation, Some(held) if held.get() == generation.get())
    }

    /// Whether this membership is held under a superseded Network generation.
    pub const fn is_stale_against(&self, generation: ResourceGeneration) -> bool {
        self.joined && !self.is_held_under(generation)
    }
}

/// The outcome of fencing and draining one membership.
///
/// `fenced` says new use is blocked; `drained` says outstanding use has reached
/// the safe state. A pass that fences but has not drained retries: the durable
/// deleting mark and the membership stay in place rather than the teardown
/// cutting through use that is still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FabricDrain {
    /// Whether new use of the membership is now blocked.
    pub fenced: bool,
    /// Whether outstanding use has drained to the safe state.
    pub drained: bool,
}

impl FabricDrain {
    /// Whether the teardown may proceed past this drain.
    pub const fn is_complete(&self) -> bool {
        self.fenced && self.drained
    }
}

/// The outcome of releasing one membership.
///
/// `fabric_retained` is true whenever the shared fabric itself survives the
/// release: releasing one consumer's membership never removes host state a
/// second member still uses (R36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FabricRelease {
    /// How many memberships still sit on the fabric after this release.
    pub remaining_members: usize,
    /// Whether the shared fabric itself was retained.
    pub fabric_retained: bool,
}

impl FabricRelease {
    /// The release of a membership that was the fabric's last member: the
    /// fabric is gone, so nothing is retained.
    pub const fn fabric_released() -> Self {
        Self {
            remaining_members: 0,
            fabric_retained: false,
        }
    }
}

/// Which interface name a membership could not derive from immutable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipDeriveError {
    /// The immutable `(Zone, Network, consumer)` identity does not derive a
    /// valid Linux interface name.
    FabricInterface,
    /// The requested namespace presentation names an interface that is not one.
    PresentedInterface,
}

/// One consumer's membership on a Network's shared fabric, as the driver
/// derived it from the committed row and the two rows that row names.
///
/// This is the whole value the effect port ever sees. It carries the row
/// identity, the Network's committed identity and generation, the consumer's
/// identity, and the two interface names the membership holds and presents. It
/// deliberately carries no traffic policy: the firewall is the Network's one
/// ownership slot, the per-consumer membership policy is the source provider's
/// admitted state, and a row that restated it would fork that authority.
#[derive(Clone, PartialEq, Eq)]
pub struct FabricMembership {
    /// The `NetworkBinding` row that owns this membership.
    pub row: ResourceKey,
    /// The stable store identity of that row.
    pub row_uid: ResourceUid,
    /// The row's committed spec generation: the identity half of the fence
    /// every readiness report this row publishes carries.
    pub row_generation: ResourceGeneration,
    /// The Zone's store-assigned identity: the fabric namespace.
    pub zone_uid: ResourceUid,
    /// The exact Network row this membership joins.
    pub network_ref: ResourceRef,
    /// The Network's store-assigned identity.
    pub network_uid: ResourceUid,
    /// The Network generation the fabric is realized under.
    pub network_generation: ResourceGeneration,
    /// The exact execution target whose interface this membership holds.
    pub target_ref: ResourceRef,
    /// The target's store-assigned identity.
    pub consumer_uid: ResourceUid,
    /// How the consumer reaches the membership.
    pub presentation: NetworkPresentation,
    /// The interface this membership holds on the shared fabric.
    pub fabric_interface: IfName,
    /// The interface the consumer sees: its namespace name, or the fabric
    /// interface itself for a shared-fabric presentation.
    pub presented_interface: IfName,
}

/// The committed facts one membership is derived from: the binding row's own
/// identity, the two rows it names, and the presentation it asks for.
///
/// Grouping them keeps the derivation one value rather than a long positional
/// call, so no field can be filled from the wrong source.
#[derive(Clone, PartialEq, Eq)]
pub struct MembershipIdentity {
    /// The `NetworkBinding` row.
    pub row: ResourceKey,
    /// The row's store-assigned identity.
    pub row_uid: ResourceUid,
    /// The row's committed spec generation.
    pub row_generation: ResourceGeneration,
    /// The Zone's store-assigned identity: the fabric namespace.
    pub zone_uid: ResourceUid,
    /// The exact Network row.
    pub network_ref: ResourceRef,
    /// The Network's store-assigned identity.
    pub network_uid: ResourceUid,
    /// The Network's committed generation.
    pub network_generation: ResourceGeneration,
    /// The exact execution target.
    pub target_ref: ResourceRef,
    /// The target's store-assigned identity.
    pub consumer_uid: ResourceUid,
    /// The requested presentation.
    pub presentation: NetworkPresentation,
}

impl FabricMembership {
    /// Derive the membership's fabric identity from the two resolved rows.
    ///
    /// The fabric interface is derived from immutable `(Zone, Network,
    /// consumer)` identity exactly as the source side derives it, so a
    /// consumer's interface name is stable across passes, generations, and
    /// restarts: re-deriving can never produce a second name for one
    /// membership.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the immutable identity does not derive a valid Linux
    /// interface name, or when a namespace presentation names an interface
    /// that is not one.
    pub fn derive(identity: MembershipIdentity) -> Result<Self, MembershipDeriveError> {
        let MembershipIdentity {
            row,
            row_uid,
            row_generation,
            zone_uid,
            network_ref,
            network_uid,
            network_generation,
            target_ref,
            consumer_uid,
            presentation,
        } = identity;
        let fabric_interface = derive_network_ifname(
            &zone_uid,
            &network_uid,
            NetworkIfRole::WorkloadGuestTap,
            Some(&consumer_uid),
        )
        .map_err(|_| MembershipDeriveError::FabricInterface)?;
        let presented_interface = match &presentation {
            NetworkPresentation::NamespaceInterface { name } => IfName::parse(name.as_str())
                .map_err(|_| MembershipDeriveError::PresentedInterface)?,
            NetworkPresentation::SharedFabric => fabric_interface.clone(),
        };
        Ok(Self {
            row,
            row_uid,
            row_generation,
            zone_uid,
            network_ref,
            network_uid,
            network_generation,
            target_ref,
            consumer_uid,
            presentation,
            fabric_interface,
            presented_interface,
        })
    }

    /// The status handle for this membership: the exact realization a pass
    /// observed, with no host bytes in it.
    pub fn handle(&self) -> MembershipHandle {
        MembershipHandle {
            network: self.network_ref.to_canonical_string(),
            target: self.target_ref.to_canonical_string(),
            fabric_interface: self.fabric_interface.as_str().to_owned(),
            presented_interface: self.presented_interface.as_str().to_owned(),
            fabric_generation: self.network_generation.get(),
        }
    }
}

impl core::fmt::Debug for FabricMembership {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FabricMembership")
            .field("network", &self.network_ref)
            .field("network_generation", &self.network_generation)
            .field("target", &self.target_ref)
            .field("presentation", &self.presentation)
            .field("presented_interface", &self.presented_interface.as_str())
            .finish_non_exhaustive()
    }
}

/// The in-memory handle the driver reports through status (R11): the exact
/// membership a pass realized or adopted. Tests compare a pre-restart
/// incarnation against the recovered one on this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipHandle {
    /// The Network the membership sits on.
    pub network: String,
    /// The execution target that holds the membership.
    pub target: String,
    /// The interface the membership holds on the shared fabric.
    pub fabric_interface: String,
    /// The interface the consumer sees.
    pub presented_interface: String,
    /// The Network generation the fabric is realized under.
    pub fabric_generation: u64,
}

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkBindingErrorKind {
    /// The durable spec did not decode as the strict neutral binding contract.
    SpecInvalid,
    /// The spec selects a Provider whose shared fabric this driver does not own.
    ProviderUnsupported,
    /// The row names a target that is not an execution target, so no fabric
    /// realizes a membership for it.
    TargetRefused,
    /// A row this driver resolved is present but is not the row the binding
    /// names: it carries another identity, another Zone, or an undecodable
    /// spec. Terminal - the committed rows cannot converge by retrying.
    SourceInvalid,
    /// The declared Network row or target row is not observable yet: the plane
    /// answered `Absent` or could not answer at all. Retryable by contract
    /// (issue #511): the actor requeues instead of failing the row terminal.
    SourceUnavailable,
    /// The declared Network row is owned by another resource.
    OwnerMismatch,
    /// The membership is realized under a superseded Network generation.
    FabricStale,
    /// A fabric realization effect failed.
    ServingEffect,
    /// The membership still has outstanding use after the drain pass.
    DrainPending,
}

impl NetworkBindingErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SourceUnavailable | Self::ServingEffect | Self::DrainPending => {
                FailureClass::Retryable
            }
            Self::SpecInvalid
            | Self::ProviderUnsupported
            | Self::TargetRefused
            | Self::SourceInvalid
            | Self::OwnerMismatch
            | Self::FabricStale => FailureClass::Terminal,
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::BINDING_SPEC_INVALID,
            Self::ProviderUnsupported => FailureKinds::BINDING_PROVIDER_UNSUPPORTED,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::SourceUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::SourceInvalid => FailureKinds::BINDING_PARENT_SPEC_INVALID,
            // A target this fabric cannot realize and a membership sitting on a
            // superseded fabric are both derivations the row cannot produce.
            Self::TargetRefused | Self::FabricStale => {
                FailureKinds::BINDING_PLAN_DERIVATION_INVALID
            }
            // The shared drain kind: draining is not a serving effect.
            Self::DrainPending => FailureKinds::CHILDREN_DRAINING,
            Self::ServingEffect => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct NetworkBindingDriverError {
    kind: NetworkBindingErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl NetworkBindingDriverError {
    fn new(kind: NetworkBindingErrorKind, op: DriverOp) -> Self {
        Self {
            kind,
            op,
            detail: FailureDetail::new(),
        }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for NetworkBindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            NetworkBindingErrorKind::SpecInvalid => "binding-spec-invalid",
            NetworkBindingErrorKind::ProviderUnsupported => "binding-provider-unsupported",
            NetworkBindingErrorKind::TargetRefused => "binding-target-refused",
            NetworkBindingErrorKind::SourceInvalid => "binding-source-invalid",
            NetworkBindingErrorKind::SourceUnavailable => "binding-source-unavailable",
            NetworkBindingErrorKind::OwnerMismatch => "binding-owner-mismatch",
            NetworkBindingErrorKind::FabricStale => "binding-fabric-generation-stale",
            NetworkBindingErrorKind::ServingEffect => "binding-serving-effect-failed",
            NetworkBindingErrorKind::DrainPending => "binding-drain-pending",
        })
    }
}

impl std::error::Error for NetworkBindingDriverError {}

/// Typed in-memory status projection (R11: never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkBindingStatus {
    /// The membership was joined this pass or found already held; readiness is
    /// what the fabric reported this pass.
    Joined {
        membership: MembershipHandle,
        /// The fabric already held the membership under the committed Network
        /// generation, so this pass issued no join.
        converged: bool,
        ready: bool,
    },
    /// The pre-restart incarnation was adopted exactly: the membership is
    /// still joined under the current generation with its interface realized.
    Recovered { membership: MembershipHandle },
    /// The drain pass fenced new use and reported whether outstanding use has
    /// reached the safe state.
    Draining {
        fenced: bool,
        drained: bool,
    },
    /// The membership was released; the fabric survives while another member
    /// still uses it.
    Released {
        remaining_members: usize,
        fabric_retained: bool,
    },
    /// A terminal admission rejection (KTD5): the stable provider reason stays
    /// visible in memory while the actor publishes the Failed phase.
    Rejected { reason: &'static str },
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one `NetworkBinding` row, exactly as persisted.
///
/// The envelope carries the Provider selection and the canonical base bytes;
/// the typed row itself is read out of those bytes by the driver's decode, so
/// an undecodable row fails without touching any host state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingSpecEnvelope {
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for `NetworkBinding` rows.
pub fn binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| BindingSpecEnvelope {
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing fabric effect surface the binding driver needs.
///
/// The production implementation is built by this crate from the daemon-
/// supplied declared facets (see [`crate::effects_service`]); test doubles
/// implement the same seam over the caller's ordered log (R4).
#[async_trait::async_trait]
pub trait NetworkBindingDriverEffects: Send + Sync + 'static {
    /// What the shared fabric currently holds for this membership.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the observation cannot complete; the pass defers
    /// rather than reporting absence.
    async fn observe_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String>;

    /// Join the membership on the provider-owned shared fabric, or place the
    /// interface under the consumer's own namespace, and report the resulting
    /// state.
    ///
    /// Idempotent under retry: joining a membership the fabric already holds
    /// under the same Network generation keeps its one interface.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the join could not be realized; the pass defers and
    /// re-joins.
    async fn join_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String>;

    /// Block new use of the membership and drive outstanding use to the safe
    /// state.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the drain could not run; the teardown retries.
    async fn drain_membership(&self, membership: &FabricMembership)
    -> Result<FabricDrain, String>;

    /// Remove this consumer's membership from the shared fabric.
    ///
    /// Idempotent under retry: a membership that was never joined, or was
    /// already released, answers `Ok` with the fabric retained.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the release could not run; the teardown retries.
    async fn leave_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricRelease, String>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the binding driver
/// factory for one zone: the declared facets plus the zone's store-assigned
/// identity, which is authority wiring folded by the plane rather than a field
/// any spec could carry.
pub struct NetworkBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
    /// The zone's store-assigned identity: the fabric namespace every derived
    /// interface name folds in.
    pub zone_uid: ResourceUid,
    /// The daemon-supplied facet set the family's effects are built from (R2).
    pub facets: crate::facets::NetworkBindingEffectFacets,
}

/// [`ResourceDriverFactory`] for the `NetworkBinding` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct NetworkBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: NetworkBindingDriverArgs,
}

impl NetworkBindingDriverFactory {
    pub(crate) fn new(args: NetworkBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(NETWORK_BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for NetworkBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(NetworkBindingDriver::new(
            self.args.zone.clone(),
            self.args.zone_uid.clone(),
            // The driver builds its effects from the declared facets; no
            // externally built port appears at this construction site (R2).
            Arc::new(NetworkBindingEffectsService::new(self.args.facets.clone())),
        ))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One `NetworkBinding` resource's driver.
#[derive(Clone)]
pub(crate) struct NetworkBindingDriver {
    zone: ZoneId,
    zone_uid: ResourceUid,
    effects: Arc<dyn NetworkBindingDriverEffects>,
    /// Targets this driver already registered a dependency watch on (R12/R17).
    /// Runtime-only (R6/R11): one registration per target keeps the dependency
    /// edge that wakes the actor without accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl NetworkBindingDriver {
    pub(crate) fn new(
        zone: ZoneId,
        zone_uid: ResourceUid,
        effects: Arc<dyn NetworkBindingDriverEffects>,
    ) -> Self {
        Self {
            zone,
            zone_uid,
            effects,
            watched: Vec::new(),
        }
    }

    fn error(&self, kind: NetworkBindingErrorKind, op: DriverOp) -> NetworkBindingDriverError {
        NetworkBindingDriverError::new(kind, op)
    }

    /// Record one terminal admission rejection in this pass's in-memory status
    /// and return the typed failure: the stable provider reason stays visible
    /// instead of collapsing into a generic error (KTD5).
    fn rejected(
        &self,
        ctx: &mut ResourceContext,
        kind: NetworkBindingErrorKind,
        reason: &'static str,
        detail: FailureDetail,
        op: DriverOp,
    ) -> NetworkBindingDriverError {
        ctx.set_status(NetworkBindingStatus::Rejected { reason });
        self.error(kind, op).with_detail(detail.with_note(reason))
    }

    /// The key of the Network row this binding declares.
    fn network_key(&self, network_ref: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            NETWORK_TYPE,
            network_ref.name().as_str(),
        )
    }

    /// The key of the target row this binding declares.
    ///
    /// A membership is realized on a fabric per execution target, so the row
    /// must name one: a consumer that is not an execution parent has no fabric
    /// to join, and the driver refuses it rather than approximating.
    fn target_key(
        &self,
        ctx: &mut ResourceContext,
        target_ref: &ResourceRef,
        op: DriverOp,
    ) -> Result<ResourceKey, NetworkBindingDriverError> {
        let execution_parent =
            BindingConsumerKind::from_resource_type(target_ref.resource_type().as_str())
                .is_some_and(BindingConsumerKind::is_execution_parent);
        if !execution_parent {
            return Err(self.rejected(
                ctx,
                NetworkBindingErrorKind::TargetRefused,
                "network-binding-target-not-an-execution-target",
                FailureDetail::at("binding/executionRef").comparison(
                    FailureComparison::new(
                        "binding.executionRef",
                        "an execution target",
                        target_ref.to_canonical_string(),
                    ),
                ),
                op,
            ));
        }
        Ok(ResourceKey::new(
            self.zone.as_str(),
            target_ref.resource_type().as_str(),
            target_ref.name().as_str(),
        ))
    }

    /// Resolve one declared row through the manager (R2: the driver never
    /// touches the spec store).
    ///
    /// A row this Zone's plane cannot answer with yet defers retryably: the
    /// row may simply not be committed yet, and issue #511 makes absence and an
    /// unanswerable plane deferrals rather than terminal verdicts. A row that
    /// comes back under another Zone, another identity, or with an identity
    /// that does not decode is terminal: the committed rows cannot converge by
    /// retrying.
    async fn resolved_row(
        &self,
        ctx: &mut ResourceContext,
        key: &ResourceKey,
        field: &'static str,
        op: DriverOp,
    ) -> Result<d2b_resource_runtime::spec_store::StoredDesiredResource, NetworkBindingDriverError>
    {
        let lookup = ctx.lookup(key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                let mut detail = FailureDetail::at("source/lookup");
                if let Some(comparison) = lookup.failure_comparison(field, "present") {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                return Err(self
                    .error(NetworkBindingErrorKind::SourceUnavailable, op)
                    .with_detail(detail));
            }
        };
        // The plane answered with a row this binding did not name: a foreign
        // Zone, type, or name is a cross-Zone or otherwise misdirected
        // membership, never a membership to join from here.
        if row.key != *key {
            return Err(self
                .error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(FailureDetail::at("source/identity").comparison(
                    FailureComparison::new(
                        field,
                        format!("{}/{}/{}", key.zone, key.type_name, key.name),
                        format!("{}/{}/{}", row.key.zone, row.key.type_name, row.key.name),
                    ),
                )));
        }
        // A membership is a Zone-local realization, and this row belongs to
        // its own Zone's plane: no other Zone's row is ever joined from here.
        if row.key.zone != ctx.key().zone {
            return Err(self
                .error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(FailureDetail::at("source/zone").comparison(
                    FailureComparison::new("row.zone", ctx.key().zone.clone(), row.key.zone),
                )));
        }
        Ok(row)
    }

    /// The store identity of one resolved row.
    fn row_uid(
        &self,
        bytes: &[u8; 16],
        field: &'static str,
        op: DriverOp,
    ) -> Result<ResourceUid, NetworkBindingDriverError> {
        resource_uid(bytes).map_err(|_| {
            self.error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(Self::row_detail(field))
        })
    }

    /// Resolve the Network row and prove it is a usable Network row.
    ///
    /// The owner fence is the one every converted binding carries: a binding
    /// whose owner uid is set must name the row that is its owner, or adopting
    /// it would silently re-parent the relationship.
    async fn resolve_network(
        &self,
        ctx: &mut ResourceContext,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<(ResourceUid, ResourceGeneration), NetworkBindingDriverError> {
        const FIELD: &str = "binding.networkRef";
        let row = self.resolved_row(ctx, &self.network_key(binding.network_ref()), FIELD, op).await?;
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            return Err(self
                .error(NetworkBindingErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("source/owner").comparison(
                    FailureComparison::new(
                        "source.ownerUid",
                        uid_hex(owner),
                        uid_hex(&row.uid),
                    ),
                )));
        }
        let uid = self.row_uid(&row.uid, FIELD, op)?;
        let generation = ResourceGeneration::new(row.generation)
            .map_err(|_| self.error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(Self::row_detail(FIELD)))?;
        // The row must actually be a Network row: a row whose stored spec does
        // not decode is terminal, never approximated.
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(Self::row_detail(FIELD)))?;
        serde_json::from_slice::<d2b_contracts_resource::v3::NetworkSpec>(
            &envelope.base().to_canonical_bytes(),
        )
        .map_err(|_| {
            self.error(NetworkBindingErrorKind::SourceInvalid, op)
                .with_detail(Self::row_detail(FIELD))
        })?;
        Ok((uid, generation))
    }

    /// Resolve the target row whose identity the membership's interface is
    /// derived from.
    async fn resolve_target(
        &self,
        ctx: &mut ResourceContext,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<ResourceUid, NetworkBindingDriverError> {
        const FIELD: &str = "binding.executionRef";
        let key = self.target_key(ctx, binding.execution_ref(), op)?;
        let row = self.resolved_row(ctx, &key, FIELD, op).await?;
        self.row_uid(&row.uid, FIELD, op)
    }

    /// The comparison naming which part of a resolved row was unusable.
    fn row_detail(field: &'static str) -> FailureDetail {
        FailureDetail::at("source/decode")
            .comparison(FailureComparison::new(field, "a canonical row", "decode failed"))
    }

    /// Decode the stored envelope into the strict neutral binding contract.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<NetworkBindingSpec, NetworkBindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(NetworkBindingErrorKind::SpecInvalid, op))?;
        match envelope.provider_ref.as_ref() {
            Some(provider_ref)
                if provider_ref.to_canonical_string() == NETWORK_BINDING_PROVIDER_REF => {}
            other => {
                return Err(self
                    .error(NetworkBindingErrorKind::ProviderUnsupported, op)
                    .with_detail(FailureDetail::at("spec/provider").comparison(
                        FailureComparison::new(
                            "spec.providerRef",
                            NETWORK_BINDING_PROVIDER_REF,
                            other
                                .map(|reference| reference.to_canonical_string())
                                .unwrap_or_else(|| "absent".to_owned()),
                        ),
                    )));
            }
        }
        serde_json::from_slice::<NetworkBindingSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(NetworkBindingErrorKind::SpecInvalid, op))
    }

    /// Register one dependency watch (R12/R17) exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the resync requeue re-evaluates those
    /// rows until they are served.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// Derive this row's membership from its committed bytes and the two rows
    /// it names.  Every verb starts here, so no verb can act on a relationship
    /// the row does not name or on a target that is not observable yet.
    async fn membership(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<FabricMembership, NetworkBindingDriverError> {
        let binding = self.decoded_binding(ctx, op)?;
        let (network_uid, network_generation) = self.resolve_network(ctx, &binding, op).await?;
        let consumer_uid = self.resolve_target(ctx, &binding, op).await?;
        let row_uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(NetworkBindingErrorKind::SpecInvalid, op))?;
        let row_generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(NetworkBindingErrorKind::SpecInvalid, op))?;
        FabricMembership::derive(MembershipIdentity {
            row: ctx.key().clone(),
            row_uid,
            row_generation,
            zone_uid: self.zone_uid.clone(),
            network_ref: binding.network_ref().clone(),
            network_uid,
            network_generation,
            target_ref: binding.execution_ref().clone(),
            consumer_uid,
            presentation: binding.presentation().clone(),
        })
        .map_err(|_| {
            self.error(NetworkBindingErrorKind::SourceInvalid, op).with_detail(
                FailureDetail::at("membership/derive").comparison(
                    FailureComparison::new(
                        "binding.presentation",
                        "a realizable interface name",
                        "derive failed",
                    ),
                ),
            )
        })
    }

    /// Observe the shared fabric for one membership.
    async fn observe(
        &self,
        membership: &FabricMembership,
        op: DriverOp,
    ) -> Result<FabricMembershipState, NetworkBindingDriverError> {
        self.effects
            .observe_membership(membership)
            .await
            .map_err(|error| {
                self.error(NetworkBindingErrorKind::ServingEffect, op)
                    .with_detail(FailureDetail::at("fabric/observe").with_note(error))
            })
    }

    /// Join one membership on the shared fabric.
    async fn join(
        &self,
        membership: &FabricMembership,
        op: DriverOp,
    ) -> Result<FabricMembershipState, NetworkBindingDriverError> {
        self.effects.join_membership(membership).await.map_err(|error| {
            self.error(NetworkBindingErrorKind::ServingEffect, op).with_detail(
                FailureDetail::at("fabric/join").comparison(FailureComparison::new(
                    "membership.state",
                    "joined",
                    "join failed",
                ))
                .with_note(error),
            )
        })
    }

    /// The fenced readiness projection this pass publishes (KTD3).
    ///
    /// The fence names the row's own identity at the row revision the manager
    /// publishes, so a reader accepts the projection for exactly this row and
    /// for no other identity or spec generation.
    fn status_projection(
        &self,
        membership: &FabricMembership,
        ready: bool,
        reason: Option<&str>,
    ) -> serde_json::Value {
        let projection = NetworkBindingStatusResource {
            ready,
            fence: NetworkBindingReadinessFence {
                uid: membership.row_uid.clone(),
                generation: membership.row_generation,
                revision: ZoneRevision::new(membership.row_generation.get()),
            },
            reason: reason.map(|code| {
                StatusCode::parse(code).expect("a driver reason is a status code")
            }),
            fabric_generation: Some(membership.network_generation),
        };
        serde_json::to_value(projection)
            .expect("the fenced membership projection is always serializable")
    }

    /// The key of the target row one derived membership belongs to, so the
    /// dependency edge watches the row the interface is placed on.
    fn membership_target_key(&self, membership: &FabricMembership) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            membership.target_ref.resource_type().as_str(),
            membership.target_ref.name().as_str(),
        )
    }
}

// ---------------------------------------------------------------------------
// Wire-visible status projection
// ---------------------------------------------------------------------------

/// The UID / generation / revision fence one readiness report was observed
/// under.
///
/// A report is current only when the UID and generation match the binding row's
/// own identity and the fence revision is at most the stored revision: every
/// store mutation, including the status write carrying the report itself,
/// advances the stored revision past the observed one, so an exact-revision
/// rule could never latch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkBindingReadinessFence {
    /// The binding row the evidence was observed under.
    pub uid: ResourceUid,
    /// The binding spec generation the evidence was observed under.
    pub generation: ResourceGeneration,
    /// The Zone-store revision the evidence was observed under.
    pub revision: ZoneRevision,
}

impl NetworkBindingReadinessFence {
    /// Whether this fence still matches the row's current identity.
    pub fn matches(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// The public `NetworkBinding` status resource projection.
///
/// It exposes only fenced readiness, the Network generation the membership is
/// realized under, and a stable safe failure code. Interface names, uids, and
/// every host byte stay provider-private and never appear here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkBindingStatusResource {
    /// Whether the fabric reports the membership realized.
    pub ready: bool,
    /// The fence the readiness evidence was observed under.
    pub fence: NetworkBindingReadinessFence,
    /// The stable safe failure code, when not ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<StatusCode>,
    /// The Network generation the membership is realized under, once the
    /// fabric has reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabric_generation: Option<ResourceGeneration>,
}

impl NetworkBindingStatusResource {
    /// Whether this projection reports ready under the current fence.
    ///
    /// Readiness without a matching UID and generation is never current
    /// (fail-closed).
    pub fn readiness_is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.ready && self.fence.matches(uid, generation, revision)
    }
}

#[async_trait::async_trait]
impl ResourceDriver for NetworkBindingDriver {
    type Error = NetworkBindingDriverError;

    fn classify_error(&self, error: &NetworkBindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            NetworkBindingErrorKind::SpecInvalid
            | NetworkBindingErrorKind::ProviderUnsupported
            | NetworkBindingErrorKind::TargetRefused
            | NetworkBindingErrorKind::SourceInvalid
            | NetworkBindingErrorKind::OwnerMismatch
            | NetworkBindingErrorKind::FabricStale => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            NetworkBindingErrorKind::SourceUnavailable | NetworkBindingErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            NetworkBindingErrorKind::ServingEffect => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode, the serving-Provider check, the exact Network and target
    /// rows, and the fabric-generation fence.
    ///
    /// A membership that is already realized on a fabric the Network has since
    /// superseded is refused rather than admitted: re-deriving a membership
    /// across a fabric teardown is the Network's own decision, and admitting the
    /// row here would let two generations of one relationship overlap.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let binding = self.decoded_binding(ctx, op)?;
        self.require_admitted_facet(ctx, &binding, op)?;
        let membership = self.membership(ctx, op).await?;
        let observed = self.observe(&membership, op).await?;
        if observed.is_stale_against(membership.network_generation) {
            return Err(self.rejected(
                ctx,
                NetworkBindingErrorKind::FabricStale,
                "network-binding-fabric-generation-stale",
                Self::stale_detail(membership.network_generation, observed),
                op,
            ));
        }
        Ok(())
    }

    /// Adoption of the pre-restart incarnation (F2): the membership is adopted
    /// only when the fabric still holds it under the Network's committed
    /// generation with its presented interface realized. Recovery observes and
    /// never joins, so a restart cannot mint a second interface.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let membership = self.membership(ctx, op).await?;
        let observed = self.observe(&membership, op).await?;
        let handle = membership.handle();
        if observed.is_held_under(membership.network_generation) && observed.interface_ready {
            ctx.set_status(NetworkBindingStatus::Recovered {
                membership: handle,
            });
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: observe the shared fabric, join the membership when
    /// it is not already held under the committed generation, register the
    /// dependency watches (R12/R17), and publish the fenced readiness
    /// projection with the observed state (R11).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let membership = self.membership(ctx, op).await?;
        self.watch_once(ctx, self.network_key(&membership.network_ref))
            .await;
        self.watch_once(ctx, self.membership_target_key(&membership))
            .await;

        let observed = self.observe(&membership, op).await?;
        // A membership the fabric already holds under the committed generation
        // is adopted as it stands; anything else is joined. The join is the
        // port's idempotent one, so a requeued pass never mints a second
        // interface for this consumer.
        let converged = observed.is_held_under(membership.network_generation);
        let state = if converged {
            observed
        } else {
            self.join(&membership, op).await?
        };
        let ready = state.is_held_under(membership.network_generation) && state.interface_ready;
        ctx.set_status(NetworkBindingStatus::Joined {
            membership: membership.handle(),
            converged,
            ready,
        });
        ctx.set_status_projection(self.status_projection(
            &membership,
            ready,
            (!ready).then_some("network-binding-membership-not-realized"),
        ));
        if !ready {
            // The join returns before the presented interface necessarily
            // exists, so an unrealized membership re-checks on a timer rather
            // than depending on a watch delivery that may never come.
            ctx.requeue_after(NETWORK_BINDING_RESYNC);
        }
        // `Satisfied` is the driver's own convergence - this pass did its work.
        // The serving half rides the fenced projection, which reports not ready
        // until the fabric holds the membership with its interface realized.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain (KTD10, R6): block new use of the membership and drive
    /// outstanding use to the safe state before anything is removed.
    ///
    /// A membership the fabric does not hold has nothing to drain and converges
    /// here; a membership that is fenced but not yet drained keeps the durable
    /// deleting mark in place and requeues rather than cutting through use that
    /// is still open. Idempotent under retry.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let membership = match self.membership(ctx, op).await {
            Ok(membership) => membership,
            Err(error) => {
                // A row whose Network or target is no longer resolvable holds no
                // membership this driver admitted, so there is nothing to keep
                // open; the teardown converges without waiting.
                tracing::debug!(
                    key = %ctx.key(),
                    error = %error,
                    "network binding has no resolvable membership to drain",
                );
                return Ok(());
            }
        };
        let observed = self.observe(&membership, op).await?;
        if !observed.joined {
            return Ok(());
        }
        let drain = self
            .effects
            .drain_membership(&membership)
            .await
            .map_err(|error| {
                self.error(NetworkBindingErrorKind::ServingEffect, op)
                    .with_detail(FailureDetail::at("fabric/drain").with_note(error))
            })?;
        ctx.set_status(NetworkBindingStatus::Draining {
            fenced: drain.fenced,
            drained: drain.drained,
        });
        if !drain.is_complete() {
            return Err(self
                .error(NetworkBindingErrorKind::DrainPending, op)
                .with_detail(FailureDetail::at("delete/drain").comparison(
                    FailureComparison::new(
                        "membership.drain",
                        "fenced and drained",
                        if drain.fenced {
                            "fenced, use outstanding"
                        } else {
                            "not fenced"
                        },
                    ),
                )));
        }
        Ok(())
    }

    /// Teardown (R10, R36): the membership is released, and the shared fabric
    /// survives while another member still uses it.
    ///
    /// Idempotent under retry: a membership the fabric no longer holds releases
    /// successfully with the fabric retained, so a second delete converges.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let membership = match self.membership(ctx, op).await {
            Ok(membership) => membership,
            Err(error) => {
                tracing::debug!(
                    key = %ctx.key(),
                    error = %error,
                    "network binding has no resolvable membership to release",
                );
                return Ok(());
            }
        };
        let release = self
            .effects
            .leave_membership(&membership)
            .await
            .map_err(|error| {
                self.error(NetworkBindingErrorKind::ServingEffect, op).with_detail(
                    FailureDetail::at("fabric/release")
                        .comparison(FailureComparison::new(
                            "membership.state",
                            "released",
                            "release failed",
                        ))
                        .with_note(error),
                )
            })?;
        ctx.set_status(NetworkBindingStatus::Released {
            remaining_members: release.remaining_members,
            fabric_retained: release.fabric_retained,
        });
        Ok(())
    }
}

impl NetworkBindingDriver {
    /// The row may only realize a presentation the source's own committed
    /// decision admits.  The row is minted from that decision, so a row whose
    /// realized facets do not cover its presentation asks for a realization
    /// the source never granted, and it is refused rather than approximated.
    fn require_admitted_facet(
        &self,
        ctx: &mut ResourceContext,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<(), NetworkBindingDriverError> {
        let admitted = binding.source().realized_facets();
        let missing = binding
            .presentation()
            .required_facets()
            .iter()
            .find(|facet| !admitted.contains(facet));
        match missing {
            None => Ok(()),
            Some(facet) => Err(self.rejected(
                ctx,
                NetworkBindingErrorKind::TargetRefused,
                "network-binding-facet-not-admitted",
                FailureDetail::at("spec/source").comparison(FailureComparison::new(
                    "spec.source.realizedFacets",
                    "the presentation's required facet",
                    format!("{facet:?}"),
                )),
                op,
            )),
        }
    }

    /// The comparison a membership realized under a superseded generation
    /// produces.
    fn stale_detail(
        committed: ResourceGeneration,
        observed: FabricMembershipState,
    ) -> FailureDetail {
        FailureDetail::at("fabric/generation").comparison(FailureComparison::new(
            "membership.fabricGeneration",
            committed.get().to_string(),
            observed
                .fabric_generation
                .map_or_else(|| "absent".to_owned(), |held| held.get().to_string()),
        ))
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The `NetworkBinding` type's driver declaration.
///
/// `NetworkBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot
/// serve the converted binding shapes without it, so it must be registered
/// before the plane opens. The type is not exportable: `ResourceExport` admits
/// only qualified `*.d2bus.org.*Service` types, so a binding can never be an
/// export subject. The driver serves no broker operations, contributes no
/// startup steps, and mints no child rows: a membership is realized on the
/// source's own fabric, so [`NETWORK_BINDING_CREATIONS`] is empty.
pub fn binding_descriptor(args: NetworkBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::NETWORK_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: NETWORK_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: NETWORK_BINDING_READS,
        operations: &[],
        creations: NETWORK_BINDING_CREATIONS,
        startup: &[],
        services: &[],
        decoder: binding_spec_decoder(),
        factory: Arc::new(NetworkBindingDriverFactory::new(args)),
    }
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    ResourceUid::from_bytes(bytes).map_err(|_| ())
}

/// The hex spelling one compared uid renders as (issue #508).
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted fabric and a recording manager
// endpoint sharing one ordered log (ordering across manager reads and fabric
// effects observed as the manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{BindingRealizationFacet, ZoneRevision};
    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::context::ResourceContext;
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_resource_runtime::error::FailureClass;
    use d2b_resource_runtime::identity::{ResourceProvenance, StoredDesiredResource};

    use super::*;
    use crate::test_support::FakeFabricEffects;

    // -- fixtures ------------------------------------------------------------

    /// The Network row the binding names: a usable Network row whose committed
    /// generation is the fabric's generation.
    fn network_row(generation: u64) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", NETWORK_TYPE, "lan"),
            uid: [0x11; 16],
            generation,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: serde_json::json!({
                "providerRef": NETWORK_BINDING_PROVIDER_REF,
                "lanCidr": "10.20.0.0/24",
                "uplinkCidr": "10.20.1.0/30",
                "netVmSystemArtifactId": "net-vm"
            })
            .to_string()
            .into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The execution target row whose identity the membership's interface is
    /// derived from.
    fn target_row() -> StoredDesiredResource {
        execution_row("Guest", "guest-a")
    }

    /// The Zone's own Host row: a Host consumer is an execution target too, and
    /// its membership sits on the same shared fabric.
    fn host_row() -> StoredDesiredResource {
        execution_row("Host", "host-system")
    }

    fn execution_row(type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: [0x22; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"{}".to_vec(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The canonical NetworkBinding row: one Network, one execution target, one
    /// presentation, and the source decision that admitted it.
    fn binding_row(execution_ref: &str, presentation: serde_json::Value) -> StoredDesiredResource {
        let source = if presentation["presentation"] == "shared-fabric" {
            BindingRealizationFacet::SharedFabric
        } else {
            BindingRealizationFacet::NamespaceInterface
        };
        StoredDesiredResource {
            key: ResourceKey::new("work", NETWORK_BINDING_TYPE_NAME, "lan-guest-a"),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::json!({
                "providerRef": NETWORK_BINDING_PROVIDER_REF,
                "networkRef": "Network/lan",
                "executionRef": execution_ref,
                "presentation": presentation,
                "source": {
                    "admittedRights": ["consume"],
                    "arbitration": "shared",
                    "realizedFacets": [source]
                }
            })
            .to_string()
            .into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn namespace_presentation() -> serde_json::Value {
        serde_json::json!({"presentation": "namespace-interface", "name": "eth0"})
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: RecordingManagerEndpoint,
        requeue: RecordingRequeue,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::default();
        let ctx = ResourceContext::new(
            row,
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

    async fn driver(effects: Arc<FakeFabricEffects>) -> Box<dyn DynResourceDriver> {
        NetworkBindingDriverFactory::new(NetworkBindingDriverArgs {
            zone: ZoneId::parse("work").expect("zone"),
            zone_uid: ResourceUid::from_bytes(&[0x33; 16]).expect("zone uid"),
            facets: effects.facet_set(),
        })
        .create(&ResourceKey::new("work", NETWORK_BINDING_TYPE_NAME, "lan-guest-a"))
        .await
    }

    /// A manager holding the Network and target rows the binding names.
    fn sourced_manager() -> RecordingManagerEndpoint {
        RecordingManagerEndpoint::new().with_rows(vec![network_row(4), target_row()])
    }

    fn row_key() -> ResourceKey {
        ResourceKey::new("work", NETWORK_BINDING_TYPE_NAME, "lan-guest-a")
    }

    // -- validate ------------------------------------------------------------

    /// An unknown Network defers: the row may simply not be committed yet, and
    /// issue #511 makes absence a deferral, never a terminal verdict.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_defers_an_unknown_network_target() {
        let fake = FakeFabricEffects::new();
        let manager = RecordingManagerEndpoint::new().with_rows(vec![target_row()]);
        let mut f = fixture(binding_row("Guest/guest-a", namespace_presentation()), manager);
        let mut d = driver(fake.clone()).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("no Network row");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(
            fake.call_order().is_empty(),
            "nothing is realized against a Network the plane cannot answer with"
        );

        // An unanswerable plane defers exactly the same way and is never
        // reported as absence.
        f.manager.set_fail_reads(true);
        let failure = d.validate(&mut f.ctx).await.expect_err("unanswerable plane");
        assert_eq!(failure.class(), FailureClass::Retryable);
    }

    /// A target that is not an execution target has no fabric to join, so the
    /// row is refused terminally rather than approximated.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_target_that_is_not_an_execution_target() {
        let fake = FakeFabricEffects::new();
        let manager = sourced_manager();
        let mut f = fixture(
            binding_row("Process/worker", namespace_presentation()),
            manager,
        );
        let mut d = driver(fake.clone()).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("target refused");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(
            matches!(
                f.ctx.status::<NetworkBindingStatus>(),
                Some(NetworkBindingStatus::Rejected {
                    reason: "network-binding-target-not-an-execution-target"
                })
            ),
            "the stable provider reason stays visible"
        );
        assert!(fake.call_order().is_empty(), "nothing is realized");
    }

    /// A membership realized on a fabric the Network has since superseded is
    /// refused at admission: re-deriving a relationship across a fabric
    /// teardown is the Network's own decision.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_stale_fabric_generation() {
        let fake = FakeFabricEffects::new();
        let manager = sourced_manager();
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();

        // The membership joins the fabric of Network generation 4.
        d.reconcile(&mut f.ctx).await.expect("join");
        d.validate(&mut f.ctx)
            .await
            .expect("the membership is current for this Network");

        // The Network advances: the fabric this membership sits on is superseded.
        f.manager.drop_row(&ResourceKey::new("work", NETWORK_TYPE, "lan"));
        f.manager.seed(network_row(5));

        let failure = d.validate(&mut f.ctx).await.expect_err("stale fabric");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(
            matches!(
                f.ctx.status::<NetworkBindingStatus>(),
                Some(NetworkBindingStatus::Rejected {
                    reason: "network-binding-fabric-generation-stale"
                })
            ),
            "the stable provider reason stays visible"
        );
    }

    /// A row whose source decision does not admit the facet its presentation
    /// realizes asks for a grant the source never made.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_presentation_the_source_did_not_admit() {
        let fake = FakeFabricEffects::new();
        let manager = sourced_manager();
        let mut row = binding_row("Guest/guest-a", namespace_presentation());
        let mut spec: serde_json::Value = serde_json::from_slice(&row.spec).expect("row spec");
        spec["source"]["realizedFacets"] = serde_json::json!(["shared-fabric"]);
        row.spec = serde_json::to_vec(&spec).expect("row spec bytes");
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("facet refused");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(fake.call_order().is_empty(), "nothing is realized");
    }

    // -- reconcile -----------------------------------------------------------

    /// Reconcile joins the membership rather than assuming it, and a second pass
    /// over a membership the fabric already holds issues no second join.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_joins_the_membership_and_then_converges_without_a_second_join() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;

        let outcome = d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome::Satisfied,
            "the pass did its own work; readiness rides the fenced projection"
        );
        assert_eq!(
            fake.call_order()
                .iter()
                .filter(|entry| entry.starts_with("fabric-join:"))
                .count(),
            1,
            "the membership was joined, not assumed: {:?}",
            fake.call_order()
        );
        assert_eq!(fake.membership_count().await, 1, "one membership");
        // The interface is not realized yet, so the pass is not ready and
        // re-checks on the preserved cadence.
        assert_eq!(f.requeue.scheduled().len(), 1);
        match f.ctx.status::<NetworkBindingStatus>() {
            Some(NetworkBindingStatus::Joined {
                converged,
                ready,
                membership,
            }) => {
                assert!(!converged, "the first pass had to join");
                assert!(!ready, "the presented interface does not exist yet");
                assert_eq!(membership.presented_interface, "eth0");
                assert_eq!(membership.network, "Network/lan");
                assert_eq!(membership.target, "Guest/guest-a");
                assert_eq!(membership.fabric_generation, 4);
            }
            other => panic!("expected Joined, got {other:?}"),
        }

        // Once the interface exists, the next pass converges without churn.
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile again");
        assert_eq!(
            fake.call_order()
                .iter()
                .filter(|entry| entry.starts_with("fabric-join:"))
                .count(),
            1,
            "a membership the fabric already holds is adopted, not re-joined"
        );
        assert_eq!(f.requeue.scheduled().len(), 1, "a converged pass stops requeueing");
        assert!(matches!(
            f.ctx.status::<NetworkBindingStatus>(),
            Some(NetworkBindingStatus::Joined {
                converged: true,
                ready: true,
                ..
            })
        ));
    }

    /// The fenced projection the concluding pass publishes names this row's own
    /// identity at the row revision, and names the fabric generation the
    /// membership is realized under.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_concluding_pass_publishes_the_fenced_status_projection() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager,
        );
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let projection = f
            .ctx
            .take_status_projection()
            .expect("the concluding pass publishes the fenced projection");
        let typed =
            serde_json::from_value::<NetworkBindingStatusResource>(projection).expect("typed");
        assert!(typed.ready, "the fabric holds the membership with its interface");
        assert!(typed.reason.is_none());
        assert_eq!(typed.fabric_generation.map(|value| value.get()), Some(4));
        assert_eq!(
            typed.fence.uid,
            resource_uid(f.ctx.uid()).expect("the row uid is canonical"),
        );
        assert_eq!(
            typed.fence.generation.get(),
            f.ctx.generation(),
            "the fence names this row's own spec generation"
        );
        assert!(typed.readiness_is_current(&typed.fence.uid, typed.fence.generation, typed.fence.revision));
        assert!(
            !typed.readiness_is_current(&typed.fence.uid, typed.fence.generation, ZoneRevision::new(0)),
            "an unfenced revision never reports the row ready"
        );
    }

    /// A membership the interface has not reached yet publishes not-ready with
    /// the stable provider reason, so readiness is never vacuous.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_unrealized_membership_never_reports_ready() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager,
        );
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let projection = f
            .ctx
            .take_status_projection()
            .expect("projection");
        let typed =
            serde_json::from_value::<NetworkBindingStatusResource>(projection).expect("typed");
        assert!(!typed.ready);
        assert_eq!(
            typed.reason.as_ref().map(StatusCode::as_str),
            Some("network-binding-membership-not-realized"),
        );
    }

    // -- recover -------------------------------------------------------------

    /// A restart adopts the joined membership exactly and never joins again,
    /// so the interface a join already created is not duplicated.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_a_joined_membership_without_duplicating_the_interface() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pre = match f.ctx.status::<NetworkBindingStatus>() {
            Some(NetworkBindingStatus::Joined { membership, .. }) => membership.clone(),
            other => panic!("expected Joined, got {other:?}"),
        };

        // Restart: a fresh context over the same durable rows.
        let mut f2 = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d2 = driver(fake.clone()).await;
        assert_eq!(
            d2.recover(&mut f2.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the membership is still joined under the committed generation"
        );
        match f2.ctx.status::<NetworkBindingStatus>() {
            Some(NetworkBindingStatus::Recovered { membership }) => {
                assert_eq!(pre, *membership, "the recovered incarnation matches");
            }
            other => panic!("expected Recovered, got {other:?}"),
        }

        // The post-restart reconcile observes the held membership and joins
        // nothing: exactly one interface was ever joined, for one membership.
        d2.reconcile(&mut f2.ctx).await.expect("reconcile");
        assert_eq!(
            fake.call_order()
                .iter()
                .filter(|entry| entry.starts_with("fabric-join:"))
                .count(),
            1
        );
        assert_eq!(fake.membership_count().await, 1, "one membership");
        assert_eq!(fake.held_interfaces().await, vec!["eth0".to_owned()]);

        // A restart whose membership is gone adopts nothing.
        let mut f3 = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            RecordingManagerEndpoint::new().with_rows(vec![network_row(4), target_row()]),
        );
        let mut d3 = driver(FakeFabricEffects::new()).await;
        assert_eq!(
            d3.recover(&mut f3.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
        );
    }

    // -- drain and release ---------------------------------------------------

    /// The drain fences new use and blocks the teardown until outstanding use
    /// reaches the safe state.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn pre_drain_fences_the_membership_and_blocks_until_use_has_drained() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let failure = d.pre_drain(&mut f.ctx).await.expect_err("use outstanding");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(fake.call_order().contains(&"fabric-drain".to_owned()));
        assert!(matches!(
            f.ctx.status::<NetworkBindingStatus>(),
            Some(NetworkBindingStatus::Draining {
                fenced: true,
                drained: false
            })
        ));
        assert_eq!(
            fake.membership_count().await,
            1,
            "a blocked drain keeps the membership in place"
        );

        fake.make_drained();
        d.pre_drain(&mut f.ctx).await.expect("drained");
    }

    /// Delete removes this consumer's membership and leaves the shared fabric
    /// standing; a second delete converges.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_removes_the_membership_and_a_second_delete_is_idempotent() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(fake.membership_count().await, 1);

        d.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(fake.membership_count().await, 0, "the membership is gone");
        assert!(
            matches!(
                f.ctx.status::<NetworkBindingStatus>(),
                Some(NetworkBindingStatus::Released {
                    fabric_retained: true,
                    ..
                })
            ),
            "one member's release never removes the fabric another still uses"
        );

        d.delete(&mut f.ctx).await.expect("delete again");
        assert_eq!(
            fake.call_order()
                .iter()
                .filter(|entry| entry.as_str() == "fabric-release")
                .count(),
            2,
            "the retried teardown converges on the same release"
        );
        assert_eq!(fake.membership_count().await, 0);
    }

    // -- guards --------------------------------------------------------------

    /// The owner fence holds: a binding whose owner uid names another resource
    /// is terminal, while a Network row that is simply not committed yet is a
    /// deferral.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_owner_mismatch_stays_terminal_while_absence_defers() {
        let fake = FakeFabricEffects::new();
        let mut row = binding_row("Guest/guest-a", namespace_presentation());
        row.owner_uid = Some([0x99; 16]);
        let manager = sourced_manager();
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;

        let failure = d.reconcile(&mut f.ctx).await.expect_err("owner mismatch");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(fake.call_order().is_empty(), "nothing is realized");
    }

    /// The decode seam refuses committed bytes that are not a canonical
    /// NetworkBinding row, and a row naming a Provider whose fabric this driver
    /// does not own.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_decode_refuses_an_undecodable_spec_and_an_unsupported_provider() {
        let fake = FakeFabricEffects::new();
        let mut row = binding_row("Guest/guest-a", namespace_presentation());
        row.spec = b"not a binding envelope".to_vec();
        let mut f = fixture(row, sourced_manager());
        let mut d = driver(fake.clone()).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("undecodable");
        assert_eq!(failure.class(), FailureClass::Terminal);

        let mut row = binding_row("Guest/guest-a", namespace_presentation());
        let mut spec: serde_json::Value = serde_json::from_slice(&row.spec).expect("row spec");
        spec["providerRef"] = serde_json::json!("Provider/network-other");
        row.spec = serde_json::to_vec(&spec).expect("row spec bytes");
        let mut f = fixture(row, sourced_manager());
        let mut d = driver(fake).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("wrong provider");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    /// A fabric effect that fails defers retryably: the committed rows converge
    /// on a retry and nothing is reported as absent.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_join_or_release_defers_retryably() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        fake.set_fail_observe(true);
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
        let mut d = driver(fake.clone()).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("observe failed");
        assert_eq!(failure.class(), FailureClass::Retryable);

        fake.set_fail_observe(false);
        fake.set_fail_join(true);
        let failure = d.reconcile(&mut f.ctx).await.expect_err("join failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(fake.membership_count().await, 0);

        fake.set_fail_join(false);
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        fake.set_fail_release(true);
        let failure = d.delete(&mut f.ctx).await.expect_err("release failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            fake.membership_count().await,
            1,
            "a failed release keeps the membership in place"
        );
    }

    /// The dependency edges are registered once per target, so a repeated pass
    /// does not accumulate manager watch entries.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let mut f = fixture(
            binding_row("Guest/guest-a", namespace_presentation()),
            manager.clone(),
        );
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
        assert_eq!(registered, targets.len(), "no watch is registered twice");
        assert!(targets.contains(&"Network/lan".to_owned()));
        assert!(targets.contains(&"Guest/guest-a".to_owned()));
    }

    /// A shared-fabric presentation presents the fabric interface itself, and
    /// the interface name is derived from immutable identity, so two passes and
    /// a restart agree on it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_shared_fabric_presentation_presents_the_derived_fabric_interface() {
        let manager = sourced_manager();
        let fake = FakeFabricEffects::shared(manager.log_handle());
        let row = binding_row("Host/host-system", serde_json::json!({"presentation": "shared-fabric"}));
        let mut f = fixture(row, manager.clone());
        f.manager.seed(host_row());
        let mut d = driver(fake.clone()).await;
        fake.make_interface_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        let handle = match f.ctx.status::<NetworkBindingStatus>() {
            Some(NetworkBindingStatus::Joined { membership, .. }) => membership.clone(),
            other => panic!("expected Joined, got {other:?}"),
        };
        assert_eq!(handle.target, "Host/host-system");
        assert_eq!(
            handle.presented_interface, handle.fabric_interface,
            "the shared fabric presents the membership's own fabric interface"
        );
        assert_eq!(
            fake.held_interfaces().await,
            vec![handle.presented_interface],
            "a Host membership is served on the same fabric as any other"
        );
        let _ = row_key();
    }
}
