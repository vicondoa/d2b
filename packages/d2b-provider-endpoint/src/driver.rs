//! The Endpoint resource driver: the v3 `ResourceDriver` conversion of the
//! daemon-owned Endpoint realization path.
//!
//! The driver covers the endpoint shapes the v3 plane realizes, per
//! preserved behavior:
//!
//! - the local Unix socket for purpose `virtiofsd` (the binding-owned
//!   virtiofsd socket): recover probes the socket on the host target,
//!   reconcile realizes the socket through the provider port as a long
//!   effect, and delete participates in the preserved endpoint-first teardown
//!   ordering - the endpoint is removed BEFORE the worker Process child (the
//!   binding driver deletes its own endpoint child first, then the worker;
//!   the drain finalizer and recycle-with-producer semantics are preserved:
//!   an endpoint with `recycle-with-producer` lifecycle goes away with its
//!   producer and nothing outlives it);
//! - the guest-runtime control endpoints the Cloud Hypervisor provider's
//!   fixed child roles declare (`ch-api` on the guest's VMM Process,
//!   `guest-control` on the Guest): the nested VMM carries both private
//!   rendezvous, so the same recover/reconcile/delete verbs run against the
//!   guest's committed VMM Process row instead of a socket this daemon
//!   creates (the row's actor owns its status; the old publication stage
//!   wrote both rows from exactly that evidence);
//! - the Device TPM Provider's worker-socket endpoints (`swtpm-tpm-socket`
//!   on the Device-class row, `swtpm-control-socket` on the Control-class
//!   one): one swtpm launch composes both sockets, so the producer Process
//!   row's `Ready` status is the evidence - the same rule as the control
//!   family, and the daemon creates and removes nothing for this shape.
//!
//! Conversion mapping:
//! - `describe` -> [`EndpointDriverFactory`] registration under `Endpoint`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - socket realization -> [`ResourceDriver::reconcile`].
//! - socket removal -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only).
//!
//! Which purposes a declaring provider commits, and on which producer, is
//! this crate's own derivation ([`crate::effects_service`]): the closed
//! admission set is read from the declaring providers' own vocabularies.
//!
//! A declaring Provider that realizes a shape this crate knows nothing
//! about - the display session's three endpoint shapes, for instance -
//! commits it through [`EndpointPurposeVocabulary::committed_endpoint_shape`]
//! and owns every constant and every field of the match itself (KTD5). This
//! crate never names such a Provider: it asks the question, the Provider
//! answers with one exact verdict, and the composition root wires the
//! answer in. Nothing is admitted until a Provider commits it, so the
//! closed set above is unchanged by an absent answer.

use std::sync::Arc;

use d2b_contracts_resource::v3::{
    BoundedText, CanonicalJsonObject, ChildSupportCeiling, ResourceRef, ResourceSpec, ResourceUid,
    ZoneId,
};
use crate::binding::{ ENDPOINT_BINDING_TYPE_NAME, EndpointBindingError };
use crate::endpoint::{ EndpointClass, EndpointLifecyclePolicy, EndpointLocality, EndpointSpec, EndpointTransport,
        EndpointVisibility, };
use d2b_resource_runtime::context::WatchCondition;
use d2b_resource_runtime::context::{ChildEnsure, ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::ResourceStatus;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};

use crate::effects_service::ENDPOINT_EFFECTS_SERVICE;

/// The frozen purpose of the binding-owned virtiofsd socket.
///
/// The purpose is this family's own vocabulary: the shape check classifies
/// against it, and the daemon's host socket facet refuses any purpose it
/// does not realize through the same constant (the refusal is preserved
/// from the pre-move port rather than weakened).
pub const VIRTIOFSD_PURPOSE: &str = "virtiofsd";

/// The producer one guest-runtime control purpose is declared with, exactly
/// as the Cloud Hypervisor provider's fixed child roles commit it: `ch-api`
/// is created on the guest's VMM Process, `guest-control` on the Guest
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlProducer {
    /// The guest's VMM Process child (`Process/<guest>-vmm`).
    VmmProcess,
    /// The Guest itself.
    Guest,
}

impl GuestControlProducer {
    /// The producer ResourceType the provider's role vocabulary declares.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::VmmProcess => "Process",
            Self::Guest => "Guest",
        }
    }

    /// The locality the provider materializes for this producer:
    /// `cross-domain` exactly for the Guest-produced endpoint, `host-local`
    /// for the VMM-Process-produced one, whose API socket lives on the host
    /// beside the VMM.
    pub const fn locality(self) -> EndpointLocality {
        match self {
            Self::VmmProcess => EndpointLocality::HostLocal,
            Self::Guest => EndpointLocality::CrossDomain,
        }
    }
}

/// The provider-neutral vocabulary the Endpoint family answers with.
///
/// The family admits exactly the endpoint shapes its declaring Providers
/// commit. For the families this crate realizes itself it derives that
/// commitment from the declaring providers' own vocabularies - the Cloud
/// Hypervisor provider's child roles and the Device TPM Provider's
/// declared purposes - so the closed admission set cannot drift from the
/// children a guest's provider controller commits. Every method has a
/// closed default, so an implementor answers only for the vocabularies it
/// actually owns: a Provider that commits no purpose of this crate's own
/// families, and no shape at all, is a complete answer rather than a
/// missing one (KTD5).
pub trait EndpointPurposeVocabulary: Send + Sync {
    /// The producer one guest-runtime control purpose is committed on, or
    /// `None` for any purpose outside that family.
    fn guest_control_producer(&self, _purpose: &str) -> Option<GuestControlProducer> {
        None
    }

    /// The Endpoint class a declaring provider declares for one of its
    /// device-worker purposes, or `None` for any purpose outside that
    /// family.
    fn device_worker_endpoint_class(&self, _purpose: &str) -> Option<EndpointClass> {
        None
    }

    /// The exact shape ONE declaring Provider commits for `spec`, with the
    /// reconnect evidence that shape is bound to, or `None` for any spec
    /// this Provider does not commit (KTD5).
    ///
    /// This is the whole provider seam. The Provider owns its own
    /// vocabulary - the provider reference, the producer, the purpose, the
    /// fingerprint, the consumer policy, and every structural field it
    /// publishes - and it matches them exactly, field by field, against
    /// the shape it committed. A look-alike on ANY axis is `None`, which is
    /// a terminal refusal: this crate never repairs a near miss into an
    /// admission and never admits one on a warning.
    ///
    /// The answer carries the reconnect generation the committed shape is
    /// bound to, because that is the one fact about a Provider's own
    /// evidence this crate cannot re-derive: it fences the shape's service
    /// fingerprint to the generation the Provider currently authenticates,
    /// so a fingerprint minted for an earlier generation is not a shape
    /// this Provider commits any more (R15).
    ///
    /// The default admits nothing, which is what a composition that
    /// injects no Provider vocabulary gets.
    fn committed_endpoint_shape(&self, _spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        None
    }
}

/// One shape a declaring Provider commits, with the reconnect evidence it
/// is bound to.
///
/// The pair travels together because they are one commitment: the shape is
/// admitted only while the fingerprint this Provider matched is the one its
/// currently authenticated reconnect generation mints, and a derivation
/// that bounded the incarnation needs the generation it was bounded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommittedEndpointShape {
    realization: EndpointRealization,
    reconnect_generation: u64,
}

impl CommittedEndpointShape {
    /// Bind one committed shape to the reconnect generation it is bounded
    /// to.
    pub const fn new(realization: EndpointRealization, reconnect_generation: u64) -> Self {
        Self { realization, reconnect_generation }
    }

    /// The realization this committed shape is served by.
    pub const fn realization(self) -> EndpointRealization {
        self.realization
    }

    /// The reconnect generation this shape's fingerprint is bound to.
    pub const fn reconnect_generation(self) -> u64 {
        self.reconnect_generation
    }
}

/// The realization the v3 plane owns for one admitted Endpoint spec. Anything
/// outside this closed set is refused at validate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRealization {
    /// The binding-owned virtiofsd socket: transport unix, purpose
    /// `virtiofsd`, realized by the serving worker Process child on the host.
    VirtiofsdSocket,
    /// One guest-runtime control endpoint: a provider-visible control
    /// endpoint the Cloud Hypervisor provider's fixed child roles declare
    /// (`ch-api` on the guest's VMM Process, `guest-control` on the Guest).
    /// The nested VMM carries both private rendezvous - the Cloud Hypervisor
    /// API socket and the authenticated guest-control session - so the
    /// realization is the evidence row's committed VMM Process being live,
    /// the evidence the old daemon publication stage read.
    GuestControl,
    /// One Device-owning worker's declared socket: the Device TPM Provider's
    /// swtpm worker rows (`swtpm-tpm-socket` on the Device-class row the
    /// Guest's VMM consumes, `swtpm-control-socket` on the Control-class one
    /// the pre-start flush connects to). One launch composes both sockets, so
    /// the realization is the producer Process row's own `Ready` status - the
    /// same rule the guest-runtime control family follows; the daemon creates
    /// and removes nothing for this shape.
    DeviceWorkerSocket,
    /// One Provider-committed host socket: a transport the declaring
    /// Provider resolves privately on the host target behind the session
    /// that owns it. The realization is the exact socket the daemon minted
    /// a handle for, and nothing about that socket is readable (KTD5,
    /// KTD8).
    HostSocketTransport,
    /// One Provider-committed worker attachment: a data carriage a worker
    /// the Provider launched realizes privately, consumed by exactly one
    /// subject. The realization is that worker's own live row.
    WorkerDataAttachment,
    /// One Provider-committed worker transport: a cross-domain carriage a
    /// worker the Provider launched realizes privately. The realization is
    /// that worker's own live row, exactly as for the attachment shape.
    WorkerCrossDomainTransport,
}

/// The evidence one Provider-committed shape is realized behind.
///
/// The two are different questions with different sources, and neither is
/// this crate's to answer: a host socket realization is private daemon
/// state (the private host observation facet), while a worker realization
/// is a row this manager serves (the generation-fenced producer view). A
/// shape that names neither is not Provider-committed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRealizationEvidence {
    /// The daemon privately observed the exact socket behind this endpoint.
    HostSocket,
    /// The producer row this endpoint declares is the realization, and it
    /// must be live at its current generation.
    ProducerRow,
}

impl EndpointRealization {
    /// The evidence a Provider-committed shape is realized behind, or
    /// `None` for a shape this crate realizes itself.
    pub const fn provider_evidence(self) -> Option<ProviderRealizationEvidence> {
        match self {
            Self::HostSocketTransport => Some(ProviderRealizationEvidence::HostSocket),
            Self::WorkerDataAttachment | Self::WorkerCrossDomainTransport => {
                Some(ProviderRealizationEvidence::ProducerRow)
            }
            Self::VirtiofsdSocket | Self::GuestControl | Self::DeviceWorkerSocket => None,
        }
    }
}

/// The shape ONE declaring Provider commits for `spec`, or `None`.
///
/// This is the admission question a Provider answers for itself. The answer
/// is the shape it committed together with the reconnect generation that
/// shape's fingerprint is bound to; anything else - including a spec that
/// differs from the committed shape on one structural field alone - is
/// `None`, and a `None` answer is a terminal refusal at validate (KTD5,
/// R14).
pub fn provider_committed_endpoint_shape(
    spec: &EndpointSpec,
    vocabulary: &dyn EndpointPurposeVocabulary,
) -> Option<CommittedEndpointShape> {
    vocabulary.committed_endpoint_shape(spec)
}

/// Classify one Endpoint spec onto the realization the plane owns.
pub fn endpoint_realization(
    spec: &EndpointSpec,
    vocabulary: &dyn EndpointPurposeVocabulary,
) -> Option<EndpointRealization> {
    if spec.endpoint_class() == EndpointClass::Service
        && spec.transport() == EndpointTransport::Unix
        && spec.purpose().as_str() == VIRTIOFSD_PURPOSE
        && spec.lifecycle_policy() == EndpointLifecyclePolicy::RecycleWithProducer
    {
        return Some(EndpointRealization::VirtiofsdSocket);
    }
    // The control family is exactly the shape the declaring provider's own
    // child roles commit, per purpose: the producer role's ResourceType and
    // the locality the provider materializes for that producer. `ch-api` is
    // produced by the VMM Process (host-local: the API socket lives on the
    // host beside the VMM), `guest-control` by the Guest (cross-domain). No
    // other producer/purpose pairing is admitted, and nothing else about the
    // Endpoint family is relaxed.
    if spec.endpoint_class() == EndpointClass::Control
        && spec.transport() == EndpointTransport::OpaqueCarriage
        && spec.visibility() == EndpointVisibility::Provider
        && spec.lifecycle_policy() == EndpointLifecyclePolicy::RecycleWithProducer
        && let Some(producer) = vocabulary.guest_control_producer(spec.purpose().as_str())
        && spec.producer_ref().resource_type().as_str() == producer.resource_type()
        && spec.locality() == producer.locality()
    {
        return Some(EndpointRealization::GuestControl);
    }
    // The device-worker family is exactly the Device TPM Provider's declared
    // worker-socket rows: the purpose names the class, the producer is the
    // swtpm worker Process that owns both sockets, and the posture is the
    // owner-scoped host-local one the provider's projection declares.
    if spec.transport() == EndpointTransport::OpaqueCarriage
        && spec.visibility() == EndpointVisibility::Owner
        && spec.locality() == EndpointLocality::HostLocal
        && spec.lifecycle_policy() == EndpointLifecyclePolicy::RecycleWithProducer
        && spec.producer_ref().resource_type().as_str() == "Process"
        && vocabulary.device_worker_endpoint_class(spec.purpose().as_str())
            == Some(spec.endpoint_class())
    {
        return Some(EndpointRealization::DeviceWorkerSocket);
    }
    // A Provider that commits a shape of its own answers last, so the three
    // families this crate realizes itself are matched exactly as before and
    // a Provider shape can never be admitted by a near miss in one of them.
    // The three shapes are disjoint from those families by construction (a
    // Provider shape is a transport this crate does not realize), and the
    // Provider's own exact match is the only thing that admits one.
    provider_committed_endpoint_shape(spec, vocabulary).map(|shape| shape.realization)
}

// ---------------------------------------------------------------------------
// Child target-support ceiling
// ---------------------------------------------------------------------------

/// The child target-support ceiling one admitted Endpoint spec offers.
///
/// An `EndpointBinding` is admitted against the endpoint's OWN declaration,
/// so the driver owns the conversion from a committed `Endpoint` row to the
/// ceiling its children may request against it (R16, U18). A ceiling bounds
/// admission and creates no binding, no reservation, and no access: the
/// realization behind the endpoint is the binding's, and the locator stays
/// with the owner. A shape this driver does not realize offers no ceiling at
/// all, so a child target cannot bound children against an endpoint nothing
/// realizes.
pub fn endpoint_child_support_ceiling(
    spec: &EndpointSpec,
    vocabulary: &dyn EndpointPurposeVocabulary,
) -> Result<ChildSupportCeiling, EndpointBindingError> {
    if endpoint_realization(spec, vocabulary).is_none() {
        return Err(EndpointBindingError::InvalidRequest);
    }
    crate::binding::endpoint_binding_support_ceiling(spec)
}

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointDriverErrorKind {
    /// The durable spec did not decode as the closed Endpoint contract.
    SpecInvalid,
    /// The spec is an Endpoint shape this driver does not realize.
    ShapeUnsupported,
    /// A provider socket effect failed transiently.
    SocketEffect,
    /// Owned children are still retiring; the delete pass requeues.
    DrainPending,
}

impl EndpointDriverErrorKind {
    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::ENDPOINT_SPEC_INVALID,
            Self::ShapeUnsupported => FailureKinds::ENDPOINT_SHAPE_UNSUPPORTED,
            Self::SocketEffect => FailureKinds::ENDPOINT_SOCKET_EFFECT_FAILED,
            Self::DrainPending => FailureKinds::ENDPOINT_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub struct EndpointDriverError {
    kind: EndpointDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl EndpointDriverError {
    fn new(kind: EndpointDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for EndpointDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            EndpointDriverErrorKind::SpecInvalid => "endpoint-spec-invalid",
            EndpointDriverErrorKind::ShapeUnsupported => "endpoint-shape-unsupported",
            EndpointDriverErrorKind::SocketEffect => "endpoint-socket-effect-failed",
            EndpointDriverErrorKind::DrainPending => "endpoint-drain-pending",
        })
    }
}

impl std::error::Error for EndpointDriverError {}

/// Typed in-memory status projection (never persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDriverStatus {
    /// A socket realization effect is in flight.
    Realizing,
    /// The endpoint is realized and observable.
    Realized,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for Endpoint rows. The Endpoint base
/// carries `providerRef` inside its typed contract, so the decoder
/// reconstructs the complete typed object.
pub fn endpoint_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| spec.base_with_provider_ref())
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing effect surface the Endpoint driver needs. The
/// production implementation is this crate's own effects service, built over
/// the daemon-supplied facets; test doubles implement the same seam.
///
/// Every method but one has a closed answer. The exception is the private
/// host observation a Provider-committed socket shape is realized behind: it
/// is the daemon's private state (KTD5), so the default observes nothing and
/// the production implementation answers through the daemon facet the
/// composition root supplies (U6). A shape that needs it is therefore never
/// admitted as realized in a composition that has not wired it - it stays
/// unrealized, which is the honest answer, not a readiness claim nothing
/// proved.
#[async_trait::async_trait]
pub trait EndpointDriverEffects: EndpointPurposeVocabulary {
    /// Whether the endpoint's socket is currently realized and observable.
    async fn socket_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool;

    /// Realize the endpoint's socket (the local Unix virtiofsd case).
    async fn ensure_socket(&self, producer_ref: &ResourceRef, purpose: &str)
        -> Result<(), String>;

    /// Remove the endpoint realization - endpoint-first teardown. Idempotent
    /// under retry.
    async fn remove_socket(&self, producer_ref: &ResourceRef, purpose: &str)
        -> Result<(), String>;

    /// The daemon's private observation of the exact socket standing behind
    /// `endpoint_ref` for `purpose`, or `None` when nothing is standing
    /// there (KTD8).
    ///
    /// The answer is an opaque minted handle and nothing else: no path, no
    /// `(dev, ino)` pair, and no host error crosses it, so this crate cannot
    /// name where the socket is even when it is standing.
    async fn observe_host_socket(
        &self,
        _endpoint_ref: &ResourceRef,
        _purpose: &str,
    ) -> Option<crate::facets::RealizationHandle> {
        None
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the composition must construct to instantiate the Endpoint
/// driver factory for one zone.
pub struct EndpointDriverArgs {
    /// The zone the driver serves.
    pub zone: String,
    /// The daemon-supplied facet set the family's effects are built from
    /// (R2): the host socket surface and the two row-evidence probes. The
    /// family never receives a daemon-built effect port.
    pub facets: crate::facets::EndpointEffectFacets,
}

/// [`ResourceDriverFactory`] for the `Endpoint` resource type. Construction
/// is infallible by contract.
pub struct EndpointDriverFactory {
    types: [ResourceTypeName; 1],
    args: EndpointDriverArgs,
}

impl EndpointDriverFactory {
    /// Build the factory over the zone's facet set.
    pub fn new(args: EndpointDriverArgs) -> Self {
        Self {
            types: [WellKnownType::ENDPOINT.to_resource_type_name()],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for EndpointDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(EndpointDriver::new(
            // The driver builds its effects from the declared facets; no
            // externally built port appears at this construction site (R2).
            Arc::new(crate::effects_service::EndpointEffectsService::new(
                self.args.facets.clone(),
            )),
        ))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// The closed connectability state a host-socket realization publishes.
///
/// The two states are all a consumer may read about a socket this daemon
/// owns privately. There is no third state that names a path, an inode, or a
/// reason: a consumer that cannot connect defers on the state it can read
/// (R15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointConnectability {
    /// The exact socket this endpoint names is bound and accepting
    /// connections.
    Connectable,
    /// Nothing proved a connectable socket behind this endpoint.
    Unavailable,
}

impl EndpointConnectability {
    /// The closed wire slug this state publishes.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Connectable => "connectable",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What one pass proved about the realization behind this row (R15, R17).
///
/// The whole of what leaves the driver in a status projection is here, and
/// every field is either a committed fact or an opaque token: the observed
/// row generation, the producer generation a Provider-committed worker shape
/// was proved against, the closed connectability state of a
/// Provider-committed host socket, and the incarnation token. No locator, no
/// `(dev, ino)` pair, and no host error is representable here at all.
struct EndpointReadiness {
    /// Whether this pass proved a realization standing.
    realized: bool,
    /// The opaque token, present only once a realization stands.
    incarnation: Option<crate::endpoint::RealizationIncarnation>,
    /// The producer row generation the proof was made against.
    producer_generation: Option<u64>,
    /// The closed connectability state, for a host-socket shape.
    connectability: Option<EndpointConnectability>,
    /// Whether this pass proved a Provider-committed shape rather than one of
    /// this crate's own families.
    committed: bool,
}

impl EndpointReadiness {
    /// A pass that proved nothing: no token, no generation, no state. This is
    /// what a realization still in flight publishes, and what a shape whose
    /// evidence is absent or unproved publishes.
    fn unrealized() -> Self {
        Self {
            realized: false,
            incarnation: None,
            producer_generation: None,
            connectability: None,
            committed: false,
        }
    }

    /// A pass over one of this crate's own families, whose token is derived
    /// from the committed producer row alone.
    fn realized_family(
        incarnation: Option<crate::endpoint::RealizationIncarnation>,
        producer_generation: Option<u64>,
    ) -> Self {
        Self { realized: true, incarnation, producer_generation, ..Self::unrealized() }
    }

    /// A pass over a Provider-committed shape whose evidence proved nothing
    /// yet.
    fn unrealized_committed(connectability: Option<EndpointConnectability>) -> Self {
        Self { connectability, committed: true, ..Self::unrealized() }
    }
}

/// One Endpoint resource's driver. It drives the resource through the
/// zone's effect port; the zone travels on [`EndpointDriverArgs`].
pub struct EndpointDriver {
    effects: Arc<dyn EndpointDriverEffects>,
    /// Dependencies this row already watches, registered at most once each.
    ///
    /// A Provider-committed shape is realized behind a row or a socket that
    /// moves without this row changing, so the actor has to be woken when it
    /// moves; without that watch a stale producer row or a replaced socket
    /// would keep the readiness this row published (R21).
    watched: Vec<ResourceKey>,
}

impl EndpointDriver {
    /// Build one resource's driver over the zone's effect object.
    pub fn new(effects: Arc<dyn EndpointDriverEffects>) -> Self {
        Self { effects, watched: Vec::new() }
    }

    /// Register one dependency watch, at most once per target.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        // Every projection the target publishes wakes this row, so a
        // generation move and a readiness transition both re-read the
        // evidence instead of waiting for a phase that never moves.
        if ctx
            .watch(target.clone(), WatchCondition::ProjectionChanged)
            .await
            .is_ok()
        {
            self.watched.push(target);
        }
    }

    /// Decode the stored spec into the strict typed Endpoint contract.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<EndpointSpec, EndpointDriverError> {
        let base = ctx
            .spec::<CanonicalJsonObject>()
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        serde_json::from_slice::<EndpointSpec>(&base.to_canonical_bytes())
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))
    }

    /// The closed set of Endpoint shapes the v3 plane realizes: the
    /// binding-owned virtiofsd socket and the guest-runtime control
    /// endpoints the declaring providers commit. Any other shape stays on
    /// the old reconciler until its conversion unit.
    fn check_shape(&self, spec: &EndpointSpec, op: DriverOp) -> Result<(), EndpointDriverError> {
        if endpoint_realization(spec, &*self.effects).is_none() {
            return Err(EndpointDriverError::new(EndpointDriverErrorKind::ShapeUnsupported, op)
                .with_detail(
                    FailureDetail::at("shape")
                        .comparison(FailureComparison::new(
                            "endpoint.shape",
                            "a realized endpoint shape",
                            format!(
                                "class={:?} transport={:?} visibility={:?} lifecycle={:?} purpose={}",
                                spec.endpoint_class(),
                                spec.transport(),
                                spec.visibility(),
                                spec.lifecycle_policy(),
                                spec.purpose().as_str(),
                            ),
                        ))
                        .with_note("no realized shape matches this endpoint contract"),
                ));
        }
        Ok(())
    }

    /// Commit the `EndpointBinding` rows this committed `Endpoint` row owns
    /// and retire the ones it no longer derives.
    ///
    /// The derivation is the family's own: the endpoint's own consumer policy
    /// names the consumers it publishes the exact endpoint to, the endpoint's
    /// own operation allowlist and attachment capacity decide how each of them
    /// reaches it, and the committed slot is a function of the endpoint's own
    /// identity. Nothing a consumer supplied reaches any of that, so a row
    /// cannot widen its own relationship by asking for a different slot, a
    /// different operation, or a different endpoint.
    ///
    /// The pass is idempotent in both directions: the derived names are a
    /// function of the committed identities, so re-ensuring unchanged bytes is
    /// the manager's `Unchanged` answer rather than a second row, and a
    /// derived set that shrank retires every row this source owns that it no
    /// longer derives. The mutation is scoped to the one type relationships
    /// live in, so any other child the endpoint row owns is untouched.
    async fn reconcile_binding_children(
        &self,
        ctx: &mut ResourceContext,
        spec: &EndpointSpec,
        op: DriverOp,
    ) -> Result<(), EndpointDriverError> {
        let zone = ZoneId::parse(ctx.key().zone.as_str())
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        let endpoint_ref = key_ref(ctx.key())
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        let deliveries = crate::binding::declared_endpoint_bindings(&zone, spec, &endpoint_ref)
            .map_err(|refusal| {
                EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op).with_detail(
                    FailureDetail::at("bindings/declared").comparison(
                        FailureComparison::new("endpoint.deliveries", "declared", format!("{refusal}")),
                    ),
                )
            })?;
        let derived = crate::binding::canonical_binding_rows(
            &zone,
            spec,
            &endpoint_ref,
            &deliveries,
        )
        .map_err(|refusal| {
            EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op).with_detail(
                FailureDetail::at("bindings/derive").comparison(
                    FailureComparison::new("endpoint.bindingRows", "derived", format!("{refusal}")),
                ),
            )
        })?;
        for row in &derived {
            ctx.ensure_child(ChildEnsure {
                type_name: ResourceTypeName::new(ENDPOINT_BINDING_TYPE_NAME),
                name: row.name().as_str().to_owned(),
                spec: row.spec().to_vec(),
                metadata: Vec::new(),
            })
            .await
            .map_err(|_| {
                EndpointDriverError::new(EndpointDriverErrorKind::DrainPending, op)
            })?;
        }
        for row in ctx
            .children()
            .await
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::DrainPending, op))?
        {
            if row.key.type_name != ENDPOINT_BINDING_TYPE_NAME {
                continue;
            }
            let still_derived = derived.iter().any(|child| child.name().as_str() == row.key.name);
            if !still_derived && !row.deleting {
                // Obsolete relationship: the manager retires it and owns its
                // own teardown (R9/F3). A row already marked deleting is
                // skipped, because the manager refuses a second delete.
                ctx.delete(&row.key)
                    .await
                    .map_err(|_| {
                        EndpointDriverError::new(EndpointDriverErrorKind::DrainPending, op)
                    })?;
            }
        }
        Ok(())
    }

    /// Derive this row's realization-incarnation token (KTD8).
    ///
    /// The token is a pure function of COMMITTED identities: this endpoint's
    /// own row identity and generation, its producer's store-assigned identity
    /// and the generation that producer row is at, and the service fingerprint
    /// this row declares. Nothing a consumer supplies takes part, and nothing
    /// host-shaped leaves this function - the value is a digest, so the same
    /// row yields the same token and any of those facts moving yields a
    /// different one.
    ///
    /// `None` when this row's producer is not a row this manager serves: there
    /// is then no committed producer identity to bind the realization to, so
    /// the pass publishes NO realization evidence rather than naming an
    /// incarnation it cannot prove. A dependent that needs one defers on it,
    /// which is the honest answer and not a hole.
    async fn realization_incarnation(
        &self,
        ctx: &mut ResourceContext,
        spec: &EndpointSpec,
        op: DriverOp,
    ) -> Result<(Option<crate::endpoint::RealizationIncarnation>, u64), EndpointDriverError> {
        let producer_key = ResourceKey::new(
            ctx.key().zone.as_str(),
            spec.producer_ref().resource_type().as_str(),
            spec.producer_ref().name().as_str(),
        );
        let producer = match ctx.lookup_view(&producer_key).await {
            d2b_resource_runtime::context::RowLookup::Present { row, .. } => row,
            _ => return Ok((None, 0)),
        };
        let producer_uid = ResourceUid::from_bytes(&producer.uid)
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        let endpoint_ref = key_ref(ctx.key())
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        crate::endpoint::RealizationIncarnation::derive(
            ctx.key().zone.as_str(),
            &endpoint_ref,
            ctx.generation(),
            producer_uid.as_str(),
            producer.generation,
            producer.generation,
            spec.service_fingerprint().map(BoundedText::as_str),
        )
        .map(|incarnation| (Some(incarnation), producer.generation))
        .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))
    }

    /// Prove one Provider-committed shape's realization from the evidence its
    /// own Provider committed it behind (R15).
    ///
    /// The two evidence kinds are the Provider's own and this crate's role
    /// is to prove them, never to widen them:
    ///
    /// - a worker shape is realized behind the producer row the endpoint
    ///   declares, so readiness needs that row to be present, not deleting,
    ///   and reporting `Ready` FOR ITS CURRENT GENERATION, read from the
    ///   generation-fenced view rather than from the row's durable spec. A
    ///   stale status, a replaced producer, or an absent row proves nothing
    ///   and publishes no token. The row is also watched, so a move
    ///   re-proves instead of leaving a stale token standing (R21).
    /// - a host socket shape is realized behind private daemon state, so
    ///   readiness needs the daemon's private observation and nothing else.
    ///   The observation answers with an opaque minted handle; absent,
    ///   unconnectable, and replaced-at-the-same-locator are one answer, and
    ///   no locator, `(dev, ino)` pair, or host error crosses into it.
    ///
    /// Either way the token is a digest over committed facts plus the
    /// Provider's own reconnect generation, never over a locator (KTD8).
    async fn committed_readiness(
        &mut self,
        ctx: &mut ResourceContext,
        spec: &EndpointSpec,
        shape: CommittedEndpointShape,
        op: DriverOp,
    ) -> Result<EndpointReadiness, EndpointDriverError> {
        let endpoint_ref = key_ref(ctx.key())
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
        let fingerprint = spec.service_fingerprint().map(BoundedText::as_str);
        // The committed facts the token is a digest of are read once, up
        // front: the evidence read below mutably borrows the context, and a
        // token that re-read a fact mid-pass could straddle two
        // observations.
        let zone = ctx.key().zone.to_owned();
        let endpoint_generation = ctx.generation();
        let derive = |producer: &str, producer_generation: u64| {
            crate::endpoint::RealizationIncarnation::derive(
                &zone,
                &endpoint_ref,
                endpoint_generation,
                producer,
                producer_generation,
                shape.reconnect_generation(),
                fingerprint,
            )
            .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))
        };
        match shape.realization().provider_evidence() {
            Some(ProviderRealizationEvidence::HostSocket) => {
                let observed = self
                    .effects
                    .observe_host_socket(&endpoint_ref, spec.purpose().as_str())
                    .await;
                let Some(handle) = observed else {
                    return Ok(EndpointReadiness::unrealized_committed(Some(
                        EndpointConnectability::Unavailable,
                    )));
                };
                let incarnation = derive(handle.nonce().as_str(), handle.rotation())?;
                Ok(EndpointReadiness {
                    realized: true,
                    incarnation: Some(incarnation),
                    producer_generation: None,
                    connectability: Some(EndpointConnectability::Connectable),
                    committed: true,
                })
            }
            Some(ProviderRealizationEvidence::ProducerRow) => {
                let producer_key = ResourceKey::new(
                    ctx.key().zone.as_str(),
                    spec.producer_ref().resource_type().as_str(),
                    spec.producer_ref().name().as_str(),
                );
                self.watch_once(ctx, producer_key.clone()).await;
                let d2b_resource_runtime::context::RowLookup::Present { row: producer, .. } =
                    ctx.lookup_view(&producer_key).await
                else {
                    return Ok(EndpointReadiness::unrealized_committed(None));
                };
                // The row is the realization, so it has to be standing: a
                // row on its way out proves nothing, and `observed_status`
                // is the accessor that refuses a status published for an
                // older generation, which is what "current" means here.
                if producer.deleting || producer.observed_status() != Some(ResourceStatus::Ready) {
                    return Ok(EndpointReadiness::unrealized_committed(None));
                }
                let producer_uid = ResourceUid::from_bytes(&producer.uid)
                    .map_err(|_| EndpointDriverError::new(EndpointDriverErrorKind::SpecInvalid, op))?;
                let incarnation = derive(producer_uid.as_str(), producer.generation)?;
                Ok(EndpointReadiness {
                    realized: true,
                    incarnation: Some(incarnation),
                    producer_generation: Some(producer.generation),
                    connectability: None,
                    committed: true,
                })
            }
            None => Ok(EndpointReadiness::unrealized_committed(None)),
        }
    }

    /// Publish the endpoint's own readiness evidence.
    ///
    /// The published layer is what a dependent reads to decide whether this
    /// exact realization is standing: the observed row generation, the
    /// generation a Provider-committed worker shape was proved against or the
    /// closed connectability state of a Provider-committed host socket, and -
    /// only once the endpoint is realized - the opaque incarnation token. No
    /// locator, no `(dev, ino)` pair, and no host error crosses it (R15, R17).
    fn publish_readiness(
        &self,
        ctx: &mut ResourceContext,
        spec: &EndpointSpec,
        readiness: &EndpointReadiness,
    ) {
        let mut endpoint = serde_json::Map::new();
        endpoint.insert(
            "readiness".to_owned(),
            serde_json::Value::String(if readiness.realized { "realized" } else { "realizing" }.to_owned()),
        );
        endpoint.insert(
            "generation".to_owned(),
            serde_json::Value::from(ctx.generation()),
        );
        if let Some(incarnation) = readiness.incarnation.as_ref() {
            endpoint.insert(
                "incarnation".to_owned(),
                serde_json::Value::String(incarnation.as_str().to_owned()),
            );
        }
        // The generation the readiness was proved against, published only for
        // a Provider-committed shape: a dependent fences "this endpoint is
        // ready at the producer generation I read" on it, and a producer that
        // moved since is exactly the staleness it exists to catch.
        if readiness.committed
            && let Some(producer_generation) = readiness.producer_generation
        {
            endpoint.insert(
                "observedProducerGeneration".to_owned(),
                serde_json::Value::from(producer_generation),
            );
        }
        if let Some(connectability) = readiness.connectability {
            endpoint.insert(
                "connectionAvailability".to_owned(),
                serde_json::Value::String(connectability.slug().to_owned()),
            );
        }
        // The source-derived EXPECTED binding set: one entry per canonical row
        // this endpoint's own publication intent mints for a consumer. A
        // dependent derives what it requires from THIS, never from a
        // consumer-local slot table and never from the rows that happen to
        // exist (R18). The digests travel with each entry so the dependent can
        // tell an authorization-only change from a row that simply moved.
        endpoint.insert(
            "bindings".to_owned(),
            serde_json::Value::Array(self.expected_bindings_layer(ctx, spec, readiness.producer_generation)),
        );
        ctx.set_status_projection(serde_json::json!({ "endpoint": serde_json::Value::Object(endpoint) }));
    }

    /// The `/endpoint/bindings` layer: the canonical rows this endpoint
    /// publishes, each with the authority facts a dependent compares.
    ///
    /// Every entry is derived here, by the driver's own derivation, from the
    /// endpoint row and its publication intent. A relationship the endpoint
    /// cannot derive contributes no entry, so a dependent's expected set is
    /// exactly the set this source mints.
    fn expected_bindings_layer(
        &self,
        ctx: &ResourceContext,
        spec: &EndpointSpec,
        producer_generation: Option<u64>,
    ) -> Vec<serde_json::Value> {
        let Ok(zone) = ZoneId::parse(ctx.key().zone.as_str()) else {
            return Vec::new();
        };
        let Ok(endpoint_ref) = key_ref(ctx.key()) else {
            return Vec::new();
        };
        let Ok(deliveries) =
            crate::binding::declared_endpoint_bindings(&zone, spec, &endpoint_ref)
        else {
            return Vec::new();
        };
        deliveries
            .iter()
            .filter_map(|delivery| {
                let slot = delivery.slot();
                let authorization =
                    crate::binding::binding_authorization_digest(
                        &zone,
                        spec,
                        &endpoint_ref,
                        ctx.generation(),
                        delivery.consumer().as_ref(),
                        slot,
                    )
                    .ok()?;
                let dependency = crate::binding::binding_dependency_revision(
                    ctx.generation(),
                    producer_generation.unwrap_or_default(),
                );
                Some(serde_json::json!({
                    "name": crate::binding::binding_row_name(
                        &zone,
                        &endpoint_ref,
                        delivery.consumer().as_ref(),
                        slot,
                    ).ok()?.as_str(),
                    "endpoint": endpoint_ref.to_canonical_string(),
                    "consumer": delivery.consumer().as_ref().to_canonical_string(),
                    "slot": slot.as_str(),
                    "authorizationDigest": authorization,
                    "dependencyRevision": dependency,
                }))
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl ResourceDriver for EndpointDriver {
    type Error = EndpointDriverError;

    fn classify_error(&self, error: &EndpointDriverError) -> DriverFailure {
        let failure = match error.kind {
            EndpointDriverErrorKind::SpecInvalid | EndpointDriverErrorKind::ShapeUnsupported => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            EndpointDriverErrorKind::SocketEffect => DriverFailure::error(
                error.op,
                error.kind.failure_kind(),
                FailureClass::Retryable,
            ),
            EndpointDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode plus the daemon-owned shape check (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_shape(&spec, DriverOp::Validate)?;
        Ok(())
    }

    /// Probe the socket on the host target: present adopts the realized
    /// endpoint, absent waits for reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_shape(&spec, DriverOp::Recover)?;
        if let Some(shape) = provider_committed_endpoint_shape(&spec, &*self.effects) {
            // A restart RE-OBSERVES the evidence before it may republish
            // `Ready`, and it names no incarnation of its own: this process
            // witnessed no realization for the row it adopts, so a token
            // belongs to the next reconcile pass, which re-reads the same
            // evidence and derives one from committed facts (R18).
            let mut readiness =
                self.committed_readiness(ctx, &spec, shape, DriverOp::Recover).await?;
            let adopted = readiness.realized;
            readiness.incarnation = None;
            self.publish_readiness(ctx, &spec, &readiness);
            if adopted {
                ctx.set_status(EndpointDriverStatus::Realized);
                return Ok(RecoveryOutcome::Adopted);
            }
            return Ok(RecoveryOutcome::Missing);
        }
        if self
            .effects
            .socket_present(spec.producer_ref(), spec.purpose().as_str())
            .await
        {
            // A restart publishes NO adoption token: this actor witnessed no
            // host effect for the incarnation that is there now, so it names
            // none. The next reconcile pass derives one from committed
            // identities, and a dependent defers until then rather than
            // trusting a token this process could not have observed (R18).
            self.publish_readiness(ctx, &spec, &EndpointReadiness::realized_family(None, None));
            ctx.set_status(EndpointDriverStatus::Realized);
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: the committed `EndpointBinding` relationships
    /// converge first, then the socket present converges; otherwise the
    /// realization effect spawns as a long effect (the mailbox never blocks
    /// on it).
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_shape(&spec, DriverOp::Reconcile)?;
        // The committed relationship is the graph's statement of what this row
        // delivers, so it is reconciled before the socket is: a socket that is
        // not bound yet still has consumers admitted to it, and a consumer
        // whose row is going away still has a relationship to withdraw.
        self.reconcile_binding_children(ctx, &spec, DriverOp::Reconcile)
            .await?;
        let op = DriverOp::Reconcile;
        if let Some(shape) = provider_committed_endpoint_shape(&spec, &*self.effects) {
            let readiness = self.committed_readiness(ctx, &spec, shape, op).await?;
            self.publish_readiness(ctx, &spec, &readiness);
            if readiness.realized {
                ctx.set_status(EndpointDriverStatus::Realized);
                return Ok(ReconcileOutcome::Satisfied);
            }
            // No effect is spawned for a Provider-committed shape: its
            // realization belongs to the Provider that committed it - a worker
            // it launched, or a socket the daemon owns - so this driver
            // creates and mutates nothing. The pass re-proves on the next
            // tick, or the moment the dependency it watches moves.
            ctx.set_status(EndpointDriverStatus::Realizing);
            return Ok(ReconcileOutcome::RetryScheduled);
        }
        let (incarnation, producer_generation) = self.realization_incarnation(ctx, &spec, op).await?;
        if self
            .effects
            .socket_present(spec.producer_ref(), spec.purpose().as_str())
            .await
        {
            self.publish_readiness(
                ctx,
                &spec,
                &EndpointReadiness::realized_family(incarnation, Some(producer_generation)),
            );
            ctx.set_status(EndpointDriverStatus::Realized);
            return Ok(ReconcileOutcome::Satisfied);
        }
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let producer_ref = spec.producer_ref().clone();
        let purpose = spec.purpose().as_str().to_owned();
        tokio::spawn(async move {
            let result = effects.ensure_socket(&producer_ref, &purpose).await;
            let effect_result = match result {
                Ok(()) => d2b_resource_runtime::context::EffectResult::Completed,
                Err(error) => d2b_resource_runtime::context::EffectResult::Failed(
                    DriverFailure::error(
                        DriverOp::Reconcile,
                        FailureKinds::ENDPOINT_SOCKET_EFFECT_FAILED,
                        FailureClass::Retryable,
                    )
                    .at("reconcile/socket")
                    .with_comparison(FailureComparison::new(
                        "endpoint.socket",
                        "realized",
                        "ensure failed",
                    ))
                    .with_note(error),
                ),
            };
            let _ = effect_sender.send(d2b_resource_runtime::context::EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        // A realization still in flight publishes the not-realized class and
        // NO token: a dependent must not read a standing incarnation out of a
        // pass whose socket effect has not landed yet.
        self.publish_readiness(ctx, &spec, &EndpointReadiness::unrealized());
        ctx.set_status(EndpointDriverStatus::Realizing);
        Ok(ReconcileOutcome::InProgress { operation })
    }

    /// Drain step: every owned child finalizes before this resource's own
    /// teardown. The call nudges each owned child through its own
    /// finalize-before-delete pass and requeues this pass while any child row
    /// is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| {
                EndpointDriverError::new(EndpointDriverErrorKind::DrainPending, DriverOp::Delete)
            })?;
        Ok(())
    }

    /// Teardown: remove the socket realization. Endpoint-first ordering is
    /// preserved: the endpoint driver's own effect runs BEFORE the worker
    /// Process child is deleted (the binding driver's delete encodes the
    /// full ordering; this driver supplies the endpoint leg of it).
    /// Idempotent under retry.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok(spec) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing durable to clean up; converged without effects.
            return Ok(());
        };
        if provider_committed_endpoint_shape(&spec, &*self.effects).is_some() {
            // A Provider-committed shape's realization is torn down by the
            // Provider that committed it: the worker it launched goes with the
            // producer lifecycle, and a socket the daemon owns is the daemon's
            // to remove. This driver created neither, so it removes neither,
            // and calling the socket effect here would reach a target this
            // shape never declared (R13).
            return Ok(());
        }
        let result = self
            .effects
            .remove_socket(spec.producer_ref(), spec.purpose().as_str())
            .await;
        result.map_err(|error| {
            EndpointDriverError::new(EndpointDriverErrorKind::SocketEffect, DriverOp::Delete)
                .with_detail(
                    FailureDetail::at("delete/socket")
                        .comparison(FailureComparison::new(
                            "endpoint.socket",
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

/// The execution domains the Endpoint type can be reconciled in.
///
/// Derived from the placement contract: `Endpoint` names no placement anchor
/// (`PlacementAnchor::canonical_for` resolves none), so an Endpoint row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its own Host domain. A realized producer may live in a Guest; the effects
/// reach that row through the manager, not through this row's placement.
const ENDPOINT_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Endpoint realization reads while reconciling.
///
/// Derived from the driver's row reads: the guest-runtime control evidence
/// reads the producer's committed `Process` row (the guest's VMM child for
/// the Guest-produced purpose), the device-worker evidence reads the producer
/// `Process` row, and the virtiofsd socket target resolves through the
/// binding's committed `VolumeBinding` row.
const ENDPOINT_READS: &[WellKnownType] = &[
    WellKnownType::PROCESS,
    WellKnownType::GUEST,
    WellKnownType::VOLUME_BINDING,
];

/// The Endpoint type's driver declaration.
///
/// `Endpoint` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot serve
/// the converted endpoint shapes without it, so it must be registered before
/// the plane opens. The type is not exportable: `ResourceExport` admits only
/// qualified `*.d2bus.org.*Service` types, so an endpoint can never be an
/// export subject. The driver serves no broker operations and creates no
/// children through this declaration; the endpoint children the volume
/// binding realizes are created by that family. The declaration carries the
/// family's declared effects service ([`crate::effects_service::ENDPOINT_EFFECTS_SERVICE`]),
/// which the daemon hosts per zone from the family's registered factory.
pub fn endpoint_descriptor(args: EndpointDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::ENDPOINT,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: ENDPOINT_EXECUTION_DOMAINS,
        exportable: false,
        reads: ENDPOINT_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[ENDPOINT_EFFECTS_SERVICE],
        decoder: endpoint_spec_decoder(),
        factory: Arc::new(EndpointDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted socket port (ordering and
// idempotence observed through the recorded calls).
// ---------------------------------------------------------------------------

/// The typed reference one committed row's store-assigned key names.
///
/// The key is the authority the manager reports the row under, so the
/// reference this builds is the row's own committed identity and never a
/// string this crate assembled from a declaration.
fn key_ref(key: &ResourceKey) -> Result<ResourceRef, EndpointBindingError> {
    ResourceRef::parse(&format!(
        "{}/{}",
        key.type_name.as_str(),
        key.name
    ))
    .map_err(|_| EndpointBindingError::WrongResourceType)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{ execution_policy::BoundedToken, ResourceRef };
use crate::endpoint::{ EndpointAttachmentPolicy, EndpointClass, EndpointConsumerPolicy,
            EndpointLifecyclePolicy, EndpointLocality, EndpointOperation, EndpointSpec,
            EndpointTransport, EndpointVisibility, };
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext,
        WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;

    use super::{
        EndpointDriver, EndpointDriverArgs, EndpointDriverEffects, EndpointDriverFactory,
        EndpointPurposeVocabulary, GuestControlProducer, endpoint_spec_decoder,
    };
    use crate::test_support::FakeSocketEffects;

    // -- fakes ---------------------------------------------------------------

    /// Dead manager: these Endpoint flows make no manager calls. It also
    /// carries the owned-row set the finalize gate reads: `delete` records
    /// each child retirement nudge, and every other mutating route still
    /// fails, so an unexpected flow is caught.
    struct DeadManager {
        owned: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
        deleted: tokio::sync::Mutex<Vec<ResourceKey>>,
    }

    impl DeadManager {
        fn new() -> Self {
            Self {
                owned: tokio::sync::Mutex::new(Vec::new()),
                deleted: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        fn with_owned(row: StoredDesiredResource) -> Arc<Self> {
            Arc::new(Self {
                owned: tokio::sync::Mutex::new(vec![row]),
                deleted: tokio::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for DeadManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.deleted.lock().await.push(key.clone());
            self.owned.lock().await.retain(|row| row.key != *key);
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.owned.lock().await.clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn test_row(spec: &EndpointSpec) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Endpoint", "endpoint"),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: Some([0x43; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::to_vec(spec).expect("endpoint spec bytes"),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn fixture(row: StoredDesiredResource) -> ResourceContext {
        fixture_with(row, Arc::new(DeadManager::new()))
    }

    fn fixture_with(
        row: StoredDesiredResource,
        manager: Arc<dyn ManagerEndpoint>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            endpoint_spec_decoder(),
            manager,
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        )
    }

    async fn driver(effects: Arc<FakeSocketEffects>) -> Box<dyn DynResourceDriver> {
        let factory = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: "work".to_owned(),
            facets: effects.facet_set(),
        });
        factory
            .create(&ResourceKey::new("work", "Endpoint", "endpoint"))
            .await
    }

    // -- endpoint spec fixtures ------------------------------------------------

    /// The shape fields the admission rules read for one Endpoint fixture.
    struct Shape<'a> {
        provider: &'a str,
        producer: &'a str,
        class: EndpointClass,
        transport: EndpointTransport,
        purpose: &'a str,
        locality: EndpointLocality,
        visibility: EndpointVisibility,
    }

    /// One Endpoint fixture from its shape, its attachment posture, and the
    /// consumer surface the declaring provider commits.
    fn endpoint_spec(
        shape: Shape<'_>,
        attachments: (bool, u16),
        subjects: &[&str],
        components: &[&str],
        operations: &[EndpointOperation],
    ) -> EndpointSpec {
        EndpointSpec::new(
            ResourceRef::parse(shape.provider).expect("provider"),
            ResourceRef::parse(shape.producer).expect("producer"),
            shape.class,
            shape.transport,
            BoundedToken::parse(shape.purpose).expect("purpose"),
            None,
            shape.locality,
            shape.visibility,
            EndpointAttachmentPolicy::new(attachments.0, attachments.1).expect("attachment policy"),
            EndpointConsumerPolicy::new(
                subjects
                    .iter()
                    .map(|subject| ResourceRef::parse(subject).expect("subject"))
                    .collect(),
                components
                    .iter()
                    .map(|component| BoundedToken::parse(*component).expect("component"))
                    .collect(),
                operations.to_vec(),
            )
            .expect("consumer policy"),
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .expect("endpoint spec")
    }

    // -- realize happy path ----------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_realizes_the_socket_through_a_long_effect() {
        let fake = FakeSocketEffects::new();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake.clone()).await;

        d.validate(&mut ctx).await.expect("validate");
        assert_eq!(
            d.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "socket absent: nothing to adopt"
        );

        // Pass one spawns the realization effect; the mailbox never blocks.
        match d.reconcile(&mut ctx).await.expect("reconcile") {
            ReconcileOutcome::InProgress { .. } => {}
            other => panic!("expected InProgress, got {other:?}"),
        }
        // Let the spawned effect task reach its send.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(fake.call_order().contains(&"ensure-socket"));

        // Pass two observes the realized socket and reports satisfied.
        assert_eq!(
            d.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            ctx.status::<super::EndpointDriverStatus>(),
            Some(&super::EndpointDriverStatus::Realized)
        );
    }

    fn virtiofsd_endpoint_spec() -> EndpointSpec {
        endpoint_spec(
            Shape {
                provider: "Provider/volume-virtiofs",
                producer: "Process/vol-worker",
                class: EndpointClass::Service,
                transport: EndpointTransport::Unix,
                purpose: "virtiofsd",
                locality: EndpointLocality::HostLocal,
                visibility: EndpointVisibility::Provider,
            },
            (false, 0),
            // No named consumer: these cases drive the socket realization
            // leg, and an endpoint that names nobody derives no relationship
            // row, so the child seam stays out of what they observe. The
            // relationship leg is driven end to end in
            // `tests/endpoint_delivery.rs`, over a manager that really commits.
            &[],
            &[],
            &[EndpointOperation::Resolve, EndpointOperation::Observe],
        )
    }

    // -- recover: adopt a realized socket ----------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_a_realized_socket() {
        let fake = FakeSocketEffects::new();
        fake.make_present();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake).await;
        assert_eq!(
            d.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
    }

    // -- finalize: owned children retire before the socket teardown ----------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_socket_teardown() {
        let manager = DeadManager::with_owned(StoredDesiredResource {
            key: ResourceKey::new("work", "Process", "vol-worker"),
            uid: [0x77; 16],
            generation: 1,
            owner_uid: Some([0x42; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        });
        let fake = FakeSocketEffects::new();
        let mut ctx = fixture_with(test_row(&virtiofsd_endpoint_spec()), manager.clone());
        let mut d = driver(fake.clone()).await;

        // A live owned child: the pass requeues and the socket teardown does
        // not run.
        let failure = d.finalize(&mut ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            manager.deleted.lock().await.len(),
            1,
            "the owned child is nudged through its own finalize-before-delete pass"
        );
        assert!(fake.call_order().is_empty(), "the socket teardown has not run");

        // The manager removed the retired child row: the same pass converges
        // without any socket effect.
        d.finalize(&mut ctx).await.expect("converged once the child retired");
        assert!(fake.call_order().is_empty(), "finalize runs no socket effect");
    }

    // -- delete: endpoint-first teardown leg --------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_removes_the_socket_before_any_worker_teardown() {
        let fake = FakeSocketEffects::new();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake.clone()).await;

        d.delete(&mut ctx).await.expect("delete");
        assert_eq!(
            fake.call_order(),
            vec!["remove-socket"],
            "endpoint driver removes the socket; the binding driver's delete runs \
             this endpoint leg BEFORE the worker Process deletion"
        );
        // Retry is idempotent.
        d.delete(&mut ctx).await.expect("delete retry");
        assert_eq!(
            fake.call_order(),
            vec!["remove-socket", "remove-socket"]
        );
    }

    // -- shape guard -----------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn non_virtiofsd_shapes_are_rejected_at_validate() {
        // The virtiofsd endpoint on a transport the family does not realize.
        let replacement = endpoint_spec(
            Shape {
                provider: "Provider/volume-virtiofs",
                producer: "Process/vol-worker",
                class: EndpointClass::Service,
                transport: EndpointTransport::Tcp,
                purpose: "virtiofsd",
                locality: EndpointLocality::HostLocal,
                visibility: EndpointVisibility::Provider,
            },
            (false, 0),
            &["Provider/volume-virtiofs"],
            &[],
            &[EndpointOperation::Resolve],
        );
        let mut ctx = fixture(test_row(&replacement));
        let mut d = driver(FakeSocketEffects::new()).await;
        let failure = d.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn malformed_spec_decodes_to_a_terminal_failure() {
        let row = StoredDesiredResource {
            spec: serde_json::json!({ "nonsense": true }).to_string().into_bytes(),
            ..test_row(&virtiofsd_endpoint_spec())
        };
        let mut ctx = fixture(row);
        let mut d = driver(FakeSocketEffects::new()).await;
        let failure = d.reconcile(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    // -- guest-runtime control endpoints ------------------------------------------

    /// The producer and locality the Cloud Hypervisor provider's own child
    /// roles commit for one guest-runtime control purpose: `ch-api` is
    /// created on the guest's VMM Process and materialized host-local (the
    /// Cloud Hypervisor API socket lives on the host beside the VMM),
    /// `guest-control` on the Guest and materialized cross-domain.
    fn guest_control_shape(purpose: &str) -> (&'static str, EndpointLocality) {
        match purpose {
            "ch-api" => ("Process/acceptance-guest-vmm", EndpointLocality::HostLocal),
            "guest-control" => ("Guest/acceptance-guest", EndpointLocality::CrossDomain),
            other => panic!("no committed control shape for {other:?}"),
        }
    }

    /// One control endpoint exactly as the provider commits it for `purpose`.
    fn guest_control_endpoint_spec(purpose: &str) -> EndpointSpec {
        let (producer, locality) = guest_control_shape(purpose);
        control_endpoint_spec(purpose, producer, locality)
    }

    fn control_endpoint_spec(
        purpose: &str,
        producer: &str,
        locality: EndpointLocality,
    ) -> EndpointSpec {
        endpoint_spec(
            Shape {
                provider: "Provider/runtime-cloud-hypervisor",
                producer,
                class: EndpointClass::Control,
                transport: EndpointTransport::OpaqueCarriage,
                purpose,
                locality,
                visibility: EndpointVisibility::Provider,
            },
            (true, 1),
            &[],
            &[],
            &[EndpointOperation::Resolve],
        )
    }

/// One device-worker endpoint from the posture the Device TPM Provider's
    /// projection declares: the swtpm worker Process is the producer, the
    /// carriage is opaque, and the purpose names the class.
    fn device_worker_endpoint_spec_with(
        purpose: &str,
        class: EndpointClass,
        producer: &str,
        locality: EndpointLocality,
        visibility: EndpointVisibility,
    ) -> EndpointSpec {
        endpoint_spec(
            Shape {
                provider: "Provider/device-tpm",
                producer,
                class,
                transport: EndpointTransport::OpaqueCarriage,
                purpose,
                locality,
                visibility,
            },
            (true, 1),
            &[],
            &["runtime-cloud-hypervisor"],
            &[EndpointOperation::Resolve],
        )
    }

    /// One device-worker endpoint exactly as the Device TPM Provider's
    /// projection declares it: purpose names the class, the swtpm worker
    /// Process is the producer, and the posture is owner-scoped host-local.
    fn device_worker_endpoint_spec(purpose: &str) -> EndpointSpec {
        let class = FakeSocketEffects::new()
            .device_worker_endpoint_class(purpose)
            .expect("declared device-worker purpose");
        device_worker_endpoint_spec_with(
            purpose,
            class,
            "Process/swtpm-tpm",
            EndpointLocality::HostLocal,
            EndpointVisibility::Owner,
        )
    }

    /// The Device TPM Provider's declared worker-socket rows are admitted: the
    /// v3 plane realizes them on the producer worker row's own evidence, so the
    /// rows reach a defined state instead of `endpoint-shape-unsupported`.
    #[test]
    fn provider_committed_device_worker_shapes_are_admitted() {
        let vocabulary = FakeSocketEffects::new();
        for purpose in ["swtpm-tpm-socket", "swtpm-control-socket"] {
            let spec = device_worker_endpoint_spec(purpose);
            assert_eq!(
                super::endpoint_realization(&spec, &*vocabulary),
                Some(super::EndpointRealization::DeviceWorkerSocket),
                "{purpose} is one of the Device TPM Provider's worker sockets",
            );
        }
    }

    /// A device-worker look-alike stays refused at validate: the purpose must
    /// be one the provider declares, with the class that purpose names, on a
    /// Process producer, and with the owner-scoped host-local posture - not
    /// every device-shaped Endpoint, and not a declared purpose on another
    /// class or producer type.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn device_worker_look_alikes_stay_refused() {
        let server = "swtpm-tpm-socket";
        let with = |class, producer: &str, locality, visibility| {
            device_worker_endpoint_spec_with(server, class, producer, locality, visibility)
        };
        for spec in [
            // The declared purpose on the other declared class.
            with(
                EndpointClass::Control,
                "Process/swtpm-tpm",
                EndpointLocality::HostLocal,
                EndpointVisibility::Owner,
            ),
            // A declared purpose produced by a Guest rather than the worker.
            with(
                EndpointClass::Device,
                "Guest/acceptance-guest",
                EndpointLocality::HostLocal,
                EndpointVisibility::Owner,
            ),
            // A declared purpose on a provider-visible or non-host-local row.
            with(
                EndpointClass::Device,
                "Process/swtpm-tpm",
                EndpointLocality::CrossDomain,
                EndpointVisibility::Owner,
            ),
            with(
                EndpointClass::Device,
                "Process/swtpm-tpm",
                EndpointLocality::HostLocal,
                EndpointVisibility::Provider,
            ),
        ] {
            let mut ctx = fixture(test_row(&spec));
            let mut d = driver(FakeSocketEffects::new()).await;
            let failure = d.validate(&mut ctx).await.expect_err("terminal");
            assert_eq!(failure.class(), FailureClass::Terminal);
        }
    }

    /// A look-alike control endpoint stays refused at validate: the admitted
    /// family is the Cloud Hypervisor provider's fixed child roles with the
    /// producer and locality that role is committed with - not every
    /// control-shaped Endpoint, and not either role's purpose on the other
    /// role's producer.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn look_alike_control_endpoints_stay_refused() {
        let guest = "Guest/acceptance-guest";
        let vmm = "Process/acceptance-guest-vmm";
        let with_transport = |purpose: &str, producer: &str, locality, transport| {
            endpoint_spec(
                Shape {
                    provider: "Provider/runtime-cloud-hypervisor",
                    producer,
                    class: EndpointClass::Control,
                    transport,
                    purpose,
                    locality,
                    visibility: EndpointVisibility::Provider,
                },
                (false, 0),
                &[],
                &[],
                &[],
            )
        };
        let cases = [
            (
                "an undeclared purpose",
                with_transport(
                    "aca-sandbox-agent",
                    guest,
                    EndpointLocality::CrossDomain,
                    EndpointTransport::OpaqueCarriage,
                ),
            ),
            (
                "a non-carriage transport",
                with_transport(
                    "guest-control",
                    guest,
                    EndpointLocality::CrossDomain,
                    EndpointTransport::Unix,
                ),
            ),
            (
                "guest-control on the VMM Process producer",
                control_endpoint_spec("guest-control", vmm, EndpointLocality::CrossDomain),
            ),
            (
                "ch-api on the Guest producer",
                control_endpoint_spec("ch-api", guest, EndpointLocality::HostLocal),
            ),
            (
                "ch-api materialized cross-domain",
                control_endpoint_spec("ch-api", vmm, EndpointLocality::CrossDomain),
            ),
            (
                "guest-control materialized host-local",
                control_endpoint_spec("guest-control", guest, EndpointLocality::HostLocal),
            ),
        ];
        for (case, spec) in cases {
            let mut ctx = fixture(test_row(&spec));
            let mut d = driver(FakeSocketEffects::new()).await;
            assert_eq!(
                d.validate(&mut ctx).await.expect_err(case).class(),
                FailureClass::Terminal,
                "{case} must stay refused",
            );
        }
    }

    /// The guest's nested VMM carries the `ch-api` and `guest-control`
    /// rendezvous, so their converted rows are the plane's to realize; the
    /// driver admits both (their row's actor owns the status the guest's
    /// provider controller gates on) and runs the same
    /// validate/recover/reconcile/delete verbs as any other admitted shape.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_control_shapes_realize_through_the_endpoint_verbs() {
        for purpose in ["ch-api", "guest-control"] {
            let fake = FakeSocketEffects::new();
            let mut ctx = fixture(test_row(&guest_control_endpoint_spec(purpose)));
            let mut d = driver(fake.clone()).await;

            d.validate(&mut ctx).await.expect("validate admits the shape");
            // The VMM evidence the production probe reads is not present in
            // this unit fixture: recover reports the endpoint missing and
            // reconcile realizes it through the effect port.
            assert_eq!(
                d.recover(&mut ctx).await.expect("recover"),
                RecoveryOutcome::Missing
            );
            match d.reconcile(&mut ctx).await.expect("reconcile") {
                ReconcileOutcome::InProgress { .. } => {}
                other => panic!("expected InProgress, got {other:?}"),
            }
            // The evidence family realizes through its evidence facet, not
            // the host socket facet: script the evidence row Ready (the
            // production probe reads the guest's committed VMM row), and the
            // socket facet records no call for the evidence purposes.
            fake.make_present();
            for _ in 0..16 {
                tokio::task::yield_now().await;
            }
            assert!(
                !fake.call_order().contains(&"ensure-socket"),
                "an evidence purpose realizes through the evidence facet, never the host socket"
            );
            assert_eq!(
                d.reconcile(&mut ctx).await.expect("reconcile"),
                ReconcileOutcome::Satisfied
            );
            d.delete(&mut ctx).await.expect("delete converges");
        }
    }

    /// A failing socket effect maps to the registered retryable
    /// socket-effect failure on the endpoint-first teardown leg.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_maps_a_socket_effect_failure_to_retryable_socket_effect() {
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = EndpointDriver::new(Arc::new(FailingSocketEffects));
        let failure = d.delete(&mut ctx).await.expect_err("retryable");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            failure.kind(),
            d2b_resource_runtime::error::FailureKinds::ENDPOINT_SOCKET_EFFECT_FAILED
        );
    }

    /// A failing ensure surfaces through the spawned long effect as the same
    /// retryable socket-effect failure the delete leg reports.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_reports_a_failed_socket_effect_as_retryable() {
        let (mut ctx, mut effects_rx) =
            fixture_with_effects_rx(test_row(&virtiofsd_endpoint_spec()));
        let mut d = EndpointDriver::new(Arc::new(FailingSocketEffects));
        match d.reconcile(&mut ctx).await.expect("reconcile") {
            ReconcileOutcome::InProgress { .. } => {}
            other => panic!("expected InProgress, got {other:?}"),
        }
        let completed = effects_rx.recv().await.expect("effect completed");
        let d2b_resource_runtime::context::EffectResult::Failed(failure) = completed.result else {
            panic!("expected a failed effect result");
        };
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(
            failure.kind(),
            d2b_resource_runtime::error::FailureKinds::ENDPOINT_SOCKET_EFFECT_FAILED
        );
    }

    /// One resource context with the effects mailbox retained, so the
    /// spawned long-effect results are observable.
    fn fixture_with_effects_rx(
        row: StoredDesiredResource,
    ) -> (
        ResourceContext,
        tokio::sync::mpsc::UnboundedReceiver<
            d2b_resource_runtime::context::EffectCompleted,
        >,
    ) {
        let (effects_tx, effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            ResourceContext::new(
                row,
                endpoint_spec_decoder(),
                Arc::new(DeadManager::new()),
                Arc::new(NullRequeue),
                effects_tx,
                notify_tx,
            ),
            effects_rx,
        )
    }

    /// Delete on an undecodable stored spec converges: nothing durable to
    /// clean up, no socket effect runs.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_on_an_undecodable_stored_spec_converges_without_effects() {
        let fake = FakeSocketEffects::new();
        let row = StoredDesiredResource {
            spec: serde_json::json!({ "nonsense": true }).to_string().into_bytes(),
            ..test_row(&virtiofsd_endpoint_spec())
        };
        let mut ctx = fixture(row);
        let mut d = driver(fake.clone()).await;
        d.delete(&mut ctx).await.expect("converged");
        assert!(
            fake.call_order().is_empty(),
            "undecodable spec: no socket effect runs"
        );
    }

    /// Scripted failing port: every socket effect refuses, so the driver's
    /// effect-failure classification is exercised.
    struct FailingSocketEffects;

    impl EndpointPurposeVocabulary for FailingSocketEffects {
        fn guest_control_producer(&self, _purpose: &str) -> Option<GuestControlProducer> {
            None
        }

        fn device_worker_endpoint_class(&self, _purpose: &str) -> Option<EndpointClass> {
            None
        }
    }

    #[async_trait::async_trait]
    impl EndpointDriverEffects for FailingSocketEffects {
        async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
            false
        }

        async fn ensure_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            Err("socket ensure refused".to_owned())
        }

        async fn remove_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            Err("socket remove refused".to_owned())
        }
    }

    // -- Provider-committed shapes (U5, KTD5, KTD8) -----------------------------

    use std::collections::HashMap;

    use d2b_contracts_resource::v3::execution_policy::BoundedText;
    use d2b_resource_runtime::ResourceStatus;
    use d2b_resource_runtime::manager::ResourceView;

    use super::{
        CommittedEndpointShape, EndpointConnectability, EndpointRealization,
        provider_committed_endpoint_shape,
    };
    use crate::endpoint::{
        EndpointBindingPublication, RealizationIncarnation,
    };
    use crate::facets::MIN_REALIZATION_NONCE_CHARS;
    use crate::facets::RealizationHandle;
    use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
    use d2b_resource_runtime::context::WatchCondition;

    /// The reconnect generation every committed fixture is bound to.
    const RECONNECT: u64 = 3;
    /// The Provider reference the committed fixtures declare.
    const DISPLAY_PROVIDER: &str = "Provider/display-wayland";
    /// The session's Host execution target: the producer of its compositor
    /// socket, which the daemon resolves privately.
    const HOST: &str = "Host/host-system";
    /// The host proxy worker row: the producer of the private data carriage.
    const PROXY: &str = "Process/proxy";
    /// The guest frontend worker row: the producer of the cross-domain
    /// transport.
    const FRONTEND: &str = "Process/frontend";

    /// One committed shape's structural axes.
    ///
    /// The axes a Provider owns and this crate cannot guess: provider,
    /// producer, class, transport, purpose, locality, visibility, lifecycle.
    struct CommittedShape<'a> {
        provider: &'a str,
        producer: &'a str,
        class: EndpointClass,
        transport: EndpointTransport,
        purpose: &'a str,
        locality: EndpointLocality,
        visibility: EndpointVisibility,
        lifecycle: EndpointLifecyclePolicy,
    }

    /// One Endpoint fixture built from its complete committed shape.
    fn committed_endpoint(
        shape: CommittedShape<'_>,
        fingerprint: &str,
        attachments: (bool, u16),
        subjects: &[&str],
        operations: &[EndpointOperation],
        publication: Option<&[&str]>,
    ) -> EndpointSpec {
        let spec = EndpointSpec::new(
            ResourceRef::parse(shape.provider).expect("provider"),
            ResourceRef::parse(shape.producer).expect("producer"),
            shape.class,
            shape.transport,
            BoundedToken::parse(shape.purpose).expect("purpose"),
            Some(BoundedText::parse(fingerprint).expect("fingerprint")),
            shape.locality,
            shape.visibility,
            EndpointAttachmentPolicy::new(attachments.0, attachments.1)
                .expect("attachment policy"),
            EndpointConsumerPolicy::new(
                subjects
                    .iter()
                    .map(|subject| ResourceRef::parse(subject).expect("subject"))
                    .collect(),
                Vec::new(),
                operations.to_vec(),
            )
            .expect("consumer policy"),
            shape.lifecycle,
        )
        .expect("endpoint spec");
        spec.with_binding_publication(match publication {
            Some(subjects) => EndpointBindingPublication::named(
                subjects
                    .iter()
                    .map(|subject| ResourceRef::parse(subject).expect("subject"))
                    .collect(),
            )
            .expect("publication"),
            None => EndpointBindingPublication::None,
        })
    }

    /// The host compositor shape: a transport the session's Host target
    /// realizes privately, published to the proxy that connects to it.
    fn compositor_shape() -> EndpointSpec {
        committed_endpoint(
            CommittedShape {
                provider: DISPLAY_PROVIDER,
                producer: HOST,
                class: EndpointClass::Transport,
                transport: EndpointTransport::Unix,
                purpose: "display-compositor",
                locality: EndpointLocality::CrossDomain,
                visibility: EndpointVisibility::Owner,
                lifecycle: EndpointLifecyclePolicy::RecycleWithProducer,
            },
            "display-wayland-compositor-r3",
            (false, 0),
            &[PROXY],
            &[EndpointOperation::Resolve],
            Some(&[PROXY]),
        )
    }

    /// The host proxy's private data carriage: consumed by exactly the
    /// session's guest frontend row, under the attach operation.
    fn proxy_shape() -> EndpointSpec {
        committed_endpoint(
            CommittedShape {
                provider: DISPLAY_PROVIDER,
                producer: PROXY,
                class: EndpointClass::Data,
                transport: EndpointTransport::FdAttachment,
                purpose: "wayland-cross-domain",
                locality: EndpointLocality::CrossDomain,
                visibility: EndpointVisibility::Owner,
                lifecycle: EndpointLifecyclePolicy::RecycleWithProducer,
            },
            "display-wayland-data-v3-r3",
            (true, 1),
            &[FRONTEND],
            &[EndpointOperation::Attach, EndpointOperation::Resolve],
            Some(&[FRONTEND]),
        )
    }

    /// The guest frontend's own cross-domain transport: it gates the
    /// session's aggregate readiness and publishes no relationship at all.
    fn frontend_shape() -> EndpointSpec {
        committed_endpoint(
            CommittedShape {
                provider: DISPLAY_PROVIDER,
                producer: FRONTEND,
                class: EndpointClass::Transport,
                transport: EndpointTransport::Vsock,
                purpose: "guest-cross-domain",
                locality: EndpointLocality::CrossDomain,
                visibility: EndpointVisibility::Owner,
                lifecycle: EndpointLifecyclePolicy::RecycleWithProducer,
            },
            "guest-frontend-v3-r3",
            (false, 0),
            &[],
            &[EndpointOperation::Resolve],
            None,
        )
    }

    /// The vocabulary one declaring Provider commits: the three shapes above,
    /// each matched in full against the spec it committed.
    ///
    /// The fixtures mirror the shapes the display Provider publishes; the
    /// constants themselves belong to that crate, and this mirror exists
    /// because the Endpoint crate cannot depend on the Provider that
    /// implements the seam (KTD5). The matching is deliberately the whole
    /// spec, so a fixture that differs on one axis is a look-alike.
    struct CommittedVocabulary {
        shapes: Vec<(EndpointSpec, CommittedEndpointShape)>,
    }

    impl CommittedVocabulary {
        fn display() -> Self {
            Self {
                shapes: vec![
                    (
                        compositor_shape(),
                        CommittedEndpointShape::new(
                            EndpointRealization::HostSocketTransport,
                            RECONNECT,
                        ),
                    ),
                    (
                        proxy_shape(),
                        CommittedEndpointShape::new(
                            EndpointRealization::WorkerDataAttachment,
                            RECONNECT,
                        ),
                    ),
                    (
                        frontend_shape(),
                        CommittedEndpointShape::new(
                            EndpointRealization::WorkerCrossDomainTransport,
                            RECONNECT,
                        ),
                    ),
                ],
            }
        }
    }

    impl EndpointPurposeVocabulary for CommittedVocabulary {
        fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
            self.shapes
                .iter()
                .find(|(committed, _)| committed == spec)
                .map(|(_, shape)| *shape)
        }
    }

    /// The production effect surface with one Provider's vocabulary injected
    /// and one daemon-side host observation.
    ///
    /// The socket effect calls are recorded and must stay empty: a
    /// Provider-committed shape's realization belongs to the Provider that
    /// committed it, so this driver creates and removes nothing for it.
    struct CommittedEffects {
        vocabulary: CommittedVocabulary,
        host: tokio::sync::Mutex<Option<RealizationHandle>>,
        socket_calls: tokio::sync::Mutex<Vec<&'static str>>,
    }

    impl CommittedEffects {
        fn new(vocabulary: CommittedVocabulary) -> Arc<Self> {
            Arc::new(Self {
                vocabulary,
                host: tokio::sync::Mutex::new(None),
                socket_calls: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        fn display() -> Arc<Self> {
            Self::new(CommittedVocabulary::display())
        }

        async fn observe(&self, handle: Option<RealizationHandle>) {
            *self.host.lock().await = handle;
        }

        async fn socket_calls(&self) -> Vec<&'static str> {
            self.socket_calls.lock().await.clone()
        }
    }

    impl EndpointPurposeVocabulary for CommittedEffects {
        fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
            self.vocabulary.committed_endpoint_shape(spec)
        }
    }

    #[async_trait::async_trait]
    impl EndpointDriverEffects for CommittedEffects {
        async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
            self.socket_calls.lock().await.push("present");
            false
        }

        async fn ensure_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            self.socket_calls.lock().await.push("ensure");
            Ok(())
        }

        async fn remove_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            self.socket_calls.lock().await.push("remove");
            Ok(())
        }

        async fn observe_host_socket(
            &self,
            _endpoint_ref: &ResourceRef,
            _purpose: &str,
        ) -> Option<RealizationHandle> {
            self.host.lock().await.clone()
        }
    }

    /// The manager a Provider-committed pass reads: the producer views it
    /// serves, the children it derived, and the watches it registered.
    struct CommittedManager {
        views: HashMap<ResourceKey, ResourceView>,
        ensured: tokio::sync::Mutex<Vec<ChildEnsure>>,
        watched: tokio::sync::Mutex<Vec<(ResourceKey, WatchCondition)>>,
    }

    impl CommittedManager {
        fn new(views: Vec<ResourceView>) -> Arc<Self> {
            Arc::new(Self {
                views: views.into_iter().map(|view| (view.key.clone(), view)).collect(),
                ensured: tokio::sync::Mutex::new(Vec::new()),
                watched: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        async fn ensured(&self) -> Vec<String> {
            self.ensured
                .lock()
                .await
                .iter()
                .map(|child| child.name.clone())
                .collect()
        }

        async fn watches(&self) -> Vec<(String, WatchCondition)> {
            self.watched
                .lock()
                .await
                .iter()
                .map(|(key, condition)| (key.name.clone(), condition.clone()))
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for CommittedManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.ensured.lock().await.push(child.clone());
            let key = ResourceKey::new(
                "work",
                child.type_name.as_str(),
                &child.name,
            );
            Ok(EnsureOutcome::Created(StoredDesiredResource {
                key,
                uid: [0x51; 16],
                generation: 1,
                owner_uid: Some([0x43; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec,
                metadata: child.metadata,
                created_at: 0,
            }))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(None)
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            Ok(self.views.get(key).cloned())
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(Vec::new())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.watched
                .lock()
                .await
                .push((registration.target, registration.condition));
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    // -- fixtures ---------------------------------------------------------------

    /// One committed Endpoint row under `name`, at `generation`.
    fn committed_row(name: &str, spec: &EndpointSpec, generation: u64) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Endpoint", name),
            uid: [0x42; 16],
            generation,
            owner_uid: Some([0x43; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::to_vec(spec).expect("endpoint spec bytes"),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The live view of one producer row: ready AT `generation`.
    fn producer_view(name: &str, uid: [u8; 16], generation: u64) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", "Process", name),
            uid,
            generation,
            deleting: false,
            provenance: ResourceProvenance::Resource,
            spec: Vec::new(),
            metadata: Vec::new(),
            owner_key: None,
            status: Some(ResourceStatus::Ready),
            status_generation: Some(generation),
            status_projection: None,
        }
    }

    /// The view of a producer row whose `Ready` status was published for an
    /// EARLIER generation: the status is real, and it is not observed state
    /// of the row that is there now.
    fn stale_producer_view(name: &str, uid: [u8; 16], generation: u64) -> ResourceView {
        ResourceView {
            status_generation: Some(generation.saturating_sub(1)),
            ..producer_view(name, uid, generation)
        }
    }

    /// The view of a producer row that is on its way out: a row being retired
    /// is not the realization standing behind the endpoint that names it.
    fn retiring_producer_view(name: &str, uid: [u8; 16], generation: u64) -> ResourceView {
        ResourceView { deleting: true, ..producer_view(name, uid, generation) }
    }

    /// One committed Endpoint actor's driver.
    ///
    /// The verbs are inherent here so each case drives the TYPED driver: the
    /// erased `DynResourceDriver` blanket is also in scope for the boxed
    /// fixtures above, and calling through it would test the erased
    /// boundary instead of the code under test.
    struct CommittedActor {
        driver: EndpointDriver,
    }

    impl CommittedActor {
        fn new(effects: Arc<CommittedEffects>) -> Self {
            Self { driver: EndpointDriver::new(effects) }
        }

        async fn validate(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<(), super::EndpointDriverError> {
            <EndpointDriver as super::ResourceDriver>::validate(&mut self.driver, ctx).await
        }

        async fn reconcile(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<ReconcileOutcome, super::EndpointDriverError> {
            <EndpointDriver as super::ResourceDriver>::reconcile(&mut self.driver, ctx).await
        }

        async fn recover(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<RecoveryOutcome, super::EndpointDriverError> {
            <EndpointDriver as super::ResourceDriver>::recover(&mut self.driver, ctx).await
        }

        /// The structural verdict the actor publishes for one failure.
        fn classify(&self, error: &super::EndpointDriverError) -> d2b_resource_runtime::error::DriverFailure {
            <EndpointDriver as super::ResourceDriver>::classify_error(&self.driver, error)
        }
    }

    fn committed_driver(effects: Arc<CommittedEffects>) -> CommittedActor {
        CommittedActor::new(effects)
    }

    /// A handle minted the way a daemon mints one: opaque hex, at least the
    /// KTD8 width, and distinct per realization.
    ///
    /// The contract's token grammar opens with a letter, so a minted nonce is
    /// spelled with a fixed prefix; the WIDTH is what KTD8 fixes, and the
    /// seed past it is what the daemon minted.
    fn handle(seed: &str, rotation: u64) -> RealizationHandle {
        let nonce = format!("minted-{seed}");
        RealizationHandle::mint(BoundedToken::parse(nonce).expect("nonce"), rotation)
            .expect("a handle at or above the KTD8 width")
    }

    /// The projection one committed pass published.
    fn published(ctx: &mut ResourceContext) -> serde_json::Value {
        ctx.take_status_projection().expect("a published projection")
    }

    /// The opaque incarnation token one published projection carries.
    fn token(projection: &serde_json::Value) -> String {
        projection
            .pointer("/endpoint/incarnation")
            .and_then(|value| value.as_str())
            .expect("a published incarnation token")
            .to_owned()
    }

    /// Each of the three committed shapes validates and reaches actor
    /// `Ready` on the evidence its own Provider committed it behind (R14).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn each_committed_shape_validates_and_reaches_ready() {
        // The host socket: the daemon's private observation is the evidence.
        let effects = CommittedEffects::display();
        effects.observe(Some(handle("a1b2c3d4e5f60718293a4b5c6d7e8f90", 1))).await;
        let manager = CommittedManager::new(Vec::new());
        let mut ctx =
            fixture_with(committed_row("compositor", &compositor_shape(), 2), manager);
        let mut d = committed_driver(effects.clone());
        d.validate(&mut ctx).await.expect("the committed shape validates");
        assert_eq!(
            d.reconcile(&mut ctx).await.expect("a committed pass"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            ctx.status::<super::EndpointDriverStatus>(),
            Some(&super::EndpointDriverStatus::Realized)
        );
        let projection = published(&mut ctx);
        assert_eq!(
            projection.pointer("/endpoint/readiness").and_then(|v| v.as_str()),
            Some("realized")
        );
        assert_eq!(
            projection
                .pointer("/endpoint/connectionAvailability")
                .and_then(|v| v.as_str()),
            Some("connectable")
        );
        assert!(
            projection
                .pointer("/endpoint/incarnation")
                .and_then(|v| v.as_str())
                .is_some(),
            "a standing realization publishes its opaque token"
        );
        assert!(
            effects.socket_calls().await.is_empty(),
            "a Provider-committed shape runs no socket effect: {:?}",
            effects.socket_calls().await
        );

        // The two worker shapes: the producer row is the evidence.
        for (name, spec, producer) in [
            ("proxy", proxy_shape(), "proxy"),
            ("frontend", frontend_shape(), "frontend"),
        ] {
            let effects = CommittedEffects::display();
            let manager = CommittedManager::new(vec![producer_view(producer, [0x61; 16], 4)]);
            let mut ctx = fixture_with(committed_row(name, &spec, 1), manager.clone());
            let mut d = committed_driver(effects.clone());
            d.validate(&mut ctx).await.expect("the committed shape validates");
            assert_eq!(
                d.reconcile(&mut ctx).await.expect("a committed pass"),
                ReconcileOutcome::Satisfied,
                "{name} is realized behind its producer row"
            );
            assert_eq!(
                ctx.status::<super::EndpointDriverStatus>(),
                Some(&super::EndpointDriverStatus::Realized)
            );
            let projection = published(&mut ctx);
            assert_eq!(
                projection
                    .pointer("/endpoint/observedProducerGeneration")
                    .and_then(|v| v.as_u64()),
                Some(4)
            );
            assert!(
                projection
                    .pointer("/endpoint/incarnation")
                    .and_then(|v| v.as_str())
                    .is_some()
            );
            assert!(
                effects.socket_calls().await.is_empty(),
                "{name} runs no socket effect: {:?}",
                effects.socket_calls().await
            );
            assert_eq!(
                manager.watches().await,
                vec![(producer.to_owned(), WatchCondition::ProjectionChanged)],
                "{name} watches the producer row it is realized behind, so a move re-proves"
            );
        }
    }

    /// A look-alike on ANY committed axis is refused terminally (R14).
    ///
    /// The admitted set is the three shapes the Provider committed in full.
    /// A near miss - another class, another transport, another producer, the
    /// committed purpose declared by another Provider, another locality,
    /// visibility, or lifecycle, a fingerprint minted for an earlier reconnect
    /// generation, a consumer the Provider does not publish to, an operation
    /// it does not admit, or a publication it did not declare - is
    /// `endpoint-shape-unsupported`, never an admission with a warning.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn committed_look_alikes_are_refused_terminally() {
        let proxy = |class,
                     transport,
                     producer,
                     provider,
                     locality,
                     visibility,
                     lifecycle,
                     purpose,
                     fingerprint,
                     subjects,
                     operations,
                     publication| {
            committed_endpoint(
                CommittedShape {
                    provider,
                    producer,
                    class,
                    transport,
                    purpose,
                    locality,
                    visibility,
                    lifecycle,
                },
                fingerprint,
                (true, 1),
                subjects,
                operations,
                publication,
            )
        };
        let cross_domain = EndpointLocality::CrossDomain;
        let owner = EndpointVisibility::Owner;
        let recycled = EndpointLifecyclePolicy::RecycleWithProducer;
        let attach: &[EndpointOperation] =
            &[EndpointOperation::Attach, EndpointOperation::Resolve];
        let published_to = Some(&[FRONTEND][..]);
        let look_alikes: Vec<(&str, EndpointSpec)> = vec![
            (
                "class",
                proxy(
                    EndpointClass::Service,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "transport",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::Vsock,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "producer",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    "Process/impostor",
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "provider",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    "Provider/device-tpm",
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "locality",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    EndpointLocality::HostLocal,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "visibility",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    EndpointVisibility::Provider,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "lifecycle",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    EndpointLifecyclePolicy::Pinned,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "purpose",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-other",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "fingerprint",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r2",
                    &[FRONTEND],
                    attach,
                    published_to,
                ),
            ),
            (
                "consumer",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND, "Process/impostor"],
                    attach,
                    published_to,
                ),
            ),
            (
                "operation",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    &[EndpointOperation::Resolve, EndpointOperation::Observe],
                    published_to,
                ),
            ),
            (
                "publication",
                proxy(
                    EndpointClass::Data,
                    EndpointTransport::FdAttachment,
                    PROXY,
                    DISPLAY_PROVIDER,
                    cross_domain,
                    owner,
                    recycled,
                    "wayland-cross-domain",
                    "display-wayland-data-v3-r3",
                    &[FRONTEND],
                    attach,
                    None,
                ),
            ),
        ];
        assert_eq!(look_alikes.len(), 12, "every committed axis has a look-alike");
        let vocabulary = CommittedVocabulary::display();
        for (axis, spec) in &look_alikes {
            assert_eq!(
                provider_committed_endpoint_shape(spec, &vocabulary),
                None,
                "the Provider commits no {axis} look-alike"
            );
            let mut ctx = fixture_with(
                committed_row("look-alike", spec, 1),
                Arc::new(DeadManager::new()),
            );
            let mut d = committed_driver(CommittedEffects::display());
            let error = d.validate(&mut ctx).await.expect_err("terminal");
            let failure = d.classify(&error);
            assert_eq!(
                failure.class(),
                FailureClass::Terminal,
                "{axis} is refused terminally"
            );
            assert_eq!(
                failure.kind().code(),
                "endpoint-shape-unsupported",
                "the {axis} refusal is the closed shape slug"
            );
        }
    }

    /// A producer row that is stale, retiring, absent, or replaced never
    /// leaves the endpoint ready under the previous token (R15).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_stale_or_replaced_producer_row_publishes_no_current_readiness() {
        for view in [
            // A `Ready` status published for an EARLIER generation: the
            // status is real, and it is not observed state of the row that is
            // there now.
            stale_producer_view("proxy", [0x61; 16], 4),
            // A row on its way out.
            retiring_producer_view("proxy", [0x61; 16], 4),
        ] {
            let manager = CommittedManager::new(vec![view]);
            let mut ctx = fixture_with(committed_row("proxy", &proxy_shape(), 1), manager);
            let mut d = committed_driver(CommittedEffects::display());
            assert_eq!(
                d.reconcile(&mut ctx).await.expect("a committed pass"),
                ReconcileOutcome::RetryScheduled,
                "an unproved realization re-proves rather than claiming readiness"
            );
            let projection = published(&mut ctx);
            assert_eq!(
                projection.pointer("/endpoint/readiness").and_then(|v| v.as_str()),
                Some("realizing")
            );
            assert!(
                projection.pointer("/endpoint/incarnation").is_none(),
                "no token is published for an unproved realization: {projection}"
            );
        }

        // An absent producer proves nothing either.
        let mut ctx = fixture_with(
            committed_row("proxy", &proxy_shape(), 1),
            CommittedManager::new(Vec::new()),
        );
        let mut d = committed_driver(CommittedEffects::display());
        assert_eq!(
            d.reconcile(&mut ctx).await.expect("a committed pass"),
            ReconcileOutcome::RetryScheduled
        );
        assert!(published(&mut ctx).pointer("/endpoint/incarnation").is_none());

        // A REPLACED producer row - same key, same name, a new store-assigned
        // identity - is a different realization, so the token moves with it.
        let replaced = |uid: [u8; 16]| {
            let manager = CommittedManager::new(vec![producer_view("proxy", uid, 4)]);
            let mut ctx = fixture_with(committed_row("proxy", &proxy_shape(), 1), manager);
            let mut d = committed_driver(CommittedEffects::display());
            async move {
                d.reconcile(&mut ctx).await.expect("a committed pass");
                token(&published(&mut ctx))
            }
        };
        let before = replaced([0x61; 16]).await;
        let after = replaced([0x62; 16]).await;
        assert_ne!(
            before, after,
            "a replaced producer row is a different incarnation"
        );
    }

    /// A socket that is absent, unconnectable, or replaced at the same
    /// locator invalidates the readiness the previous pass published (R15).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_socket_that_moves_invalidates_prior_readiness() {
        let effects = CommittedEffects::display();
        effects.observe(Some(handle("a1b2c3d4e5f60718293a4b5c6d7e8f90", 1))).await;
        let compositor_pass = |effects: Arc<CommittedEffects>| async move {
            let mut ctx = fixture_with(
                committed_row("compositor", &compositor_shape(), 2),
                CommittedManager::new(Vec::new()),
            );
            let mut d = committed_driver(effects.clone());
            (d.reconcile(&mut ctx).await.expect("a committed pass"), published(&mut ctx))
        };

        let (outcome, projection) = compositor_pass(effects.clone()).await;
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let standing = token(&projection);

        // Nothing standing there: the socket is gone or does not accept a
        // connection. Both are one answer, and neither is readiness.
        effects.observe(None).await;
        let (outcome, projection) = compositor_pass(effects.clone()).await;
        assert_eq!(outcome, ReconcileOutcome::RetryScheduled);
        assert_eq!(
            projection
                .pointer("/endpoint/connectionAvailability")
                .and_then(|v| v.as_str()),
            Some("unavailable")
        );
        assert!(
            projection.pointer("/endpoint/incarnation").is_none(),
            "an absent socket publishes no incarnation at all: {projection}"
        );

        // The same locator, a different socket: the daemon minted a new
        // handle, so the realization standing there is a new one.
        effects.observe(Some(handle("0f1e2d3c4b5a69788796a5b4c3d2e1f0", 2))).await;
        let (_, projection) = compositor_pass(effects.clone()).await;
        assert_ne!(
            standing,
            token(&projection),
            "a socket replaced at the same locator is a different incarnation"
        );
    }

    /// Readiness at one incarnation cannot satisfy a delivery proved at
    /// another (R17, R18).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn readiness_at_one_incarnation_cannot_satisfy_another_delivery() {
        let manager = CommittedManager::new(vec![producer_view("proxy", [0x61; 16], 4)]);
        let mut ctx = fixture_with(committed_row("proxy", &proxy_shape(), 1), manager);
        let mut d = committed_driver(CommittedEffects::display());
        d.reconcile(&mut ctx).await.expect("a committed pass");
        // The producer identity is the store-assigned uid the manager reports
        // for that row, spelled the way the contract spells one.
        let producer_uid = ResourceUid::from_bytes(&[0x61; 16]).expect("a bounded uid");
        let endpoint_token = RealizationIncarnation::derive(
            "work",
            &ResourceRef::parse("Endpoint/proxy").expect("endpoint ref"),
            1,
            producer_uid.as_str(),
            4,
            RECONNECT,
            Some("display-wayland-data-v3-r3"),
        )
        .expect("a bounded token");
        assert_eq!(
            token(&published(&mut ctx)),
            endpoint_token.as_str(),
            "the endpoint publishes the token its own evidence derives"
        );

        // A relationship delivered against a DIFFERENT realization of the
        // same endpoint - here the same row at another generation.
        let other = RealizationIncarnation::derive(
            "work",
            &ResourceRef::parse("Endpoint/proxy").expect("endpoint ref"),
            2,
            producer_uid.as_str(),
            4,
            RECONNECT,
            Some("display-wayland-data-v3-r3"),
        )
        .expect("a bounded token");
        assert!(
            !crate::binding::BindingDeliveryProjection::Delivered {
                incarnation: other,
                generation: 1,
            }
            .proves_delivery(&endpoint_token),
            "a delivery at another incarnation proves nothing about this one"
        );
        let same_incarnation = crate::binding::BindingDeliveryProjection::Delivered {
            incarnation: endpoint_token.clone(),
            generation: 1,
        };
        assert!(
            same_incarnation.proves_delivery(&endpoint_token),
            "and the delivery at THIS incarnation is what it proves"
        );
    }

    /// A stale reconnect fingerprint, or a moved endpoint generation, is
    /// never published as current readiness (R15).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_stale_fingerprint_or_endpoint_generation_is_never_current() {
        // The fingerprint of an EARLIER reconnect generation is not a shape
        // the Provider commits any more, so it never reaches a readiness
        // question at all.
        let stale = committed_endpoint(
            CommittedShape {
                provider: DISPLAY_PROVIDER,
                producer: PROXY,
                class: EndpointClass::Data,
                transport: EndpointTransport::FdAttachment,
                purpose: "wayland-cross-domain",
                locality: EndpointLocality::CrossDomain,
                visibility: EndpointVisibility::Owner,
                lifecycle: EndpointLifecyclePolicy::RecycleWithProducer,
            },
            "display-wayland-data-v3-r2",
            (true, 1),
            &[FRONTEND],
            &[EndpointOperation::Attach, EndpointOperation::Resolve],
            Some(&[FRONTEND]),
        );
        let mut ctx = fixture_with(
            committed_row("proxy", &stale, 1),
            Arc::new(DeadManager::new()),
        );
        let mut d = committed_driver(CommittedEffects::display());
        assert!(
            d.validate(&mut ctx).await.is_err(),
            "an earlier generation's fingerprint is not a committed shape"
        );

        // A moved endpoint generation is a different realization: the token
        // moves with the row, and the published generation is the current one.
        let mut tokens = Vec::new();
        for generation in [1_u64, 2] {
            let manager = CommittedManager::new(vec![producer_view("proxy", [0x61; 16], 4)]);
            let mut ctx = fixture_with(committed_row("proxy", &proxy_shape(), generation), manager);
            let mut d = committed_driver(CommittedEffects::display());
            d.reconcile(&mut ctx).await.expect("a committed pass");
            let projection = published(&mut ctx);
            assert_eq!(
                projection.pointer("/endpoint/generation").and_then(|v| v.as_u64()),
                Some(generation)
            );
            tokens.push(token(&projection));
        }
        assert_ne!(
            tokens[0], tokens[1],
            "a moved endpoint generation rotates the token"
        );
    }

    /// No sentinel host path, device/inode value, or raw observation error
    /// reaches the published status, a diagnostic, or a `Debug` rendering
    /// (R15, R17).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn no_host_fact_reaches_a_status_a_diagnostic_or_a_log_line() {
        // The daemon minted a handle whose nonce carries the host facts it
        // would otherwise have had to report: a path, a device, and an inode.
        let sentinel_seed = "sentinelhostpath-2f00-3badc0de-0123456789ab";
        let sentinel_error = "connection refused: /run/d2b/host-systems/sentinel.sock";
        let sentinels = ["sentinelhostpath", "2f00", "3badc0de", "/run/d2b"];
        let minted = handle(sentinel_seed, 1);
        assert!(minted.nonce().as_str().len() > MIN_REALIZATION_NONCE_CHARS);
        assert!(
            minted.nonce().as_str().contains("sentinelhostpath"),
            "the daemon's nonce carries the host facts it would otherwise report"
        );
        assert!(
            !format!("{minted:?}").contains("sentinelhostpath"),
            "a handle renders redacted, so a log line cannot carry the token"
        );

        let effects = CommittedEffects::display();
        effects.observe(Some(minted)).await;
        let mut ctx = fixture_with(
            committed_row("compositor", &compositor_shape(), 2),
            CommittedManager::new(Vec::new()),
        );
        let mut d = committed_driver(effects.clone());
        d.reconcile(&mut ctx).await.expect("a committed pass");
        let rendered = published(&mut ctx).to_string();
        for sentinel in sentinels {
            assert!(
                !rendered.contains(sentinel),
                "the published status carries {sentinel:?}: {rendered}"
            );
        }

        // The refusal of a look-alike names the committed axes it compared,
        // and no host fact of any kind.
        let look_alike = committed_endpoint(
            CommittedShape {
                provider: DISPLAY_PROVIDER,
                producer: HOST,
                class: EndpointClass::Transport,
                transport: EndpointTransport::Unix,
                purpose: "display-other",
                locality: EndpointLocality::CrossDomain,
                visibility: EndpointVisibility::Owner,
                lifecycle: EndpointLifecyclePolicy::RecycleWithProducer,
            },
            "display-wayland-compositor-r3",
            (false, 0),
            &[PROXY],
            &[EndpointOperation::Resolve],
            Some(&[PROXY]),
        );
        let mut ctx = fixture_with(
            committed_row("compositor", &look_alike, 2),
            Arc::new(DeadManager::new()),
        );
        let mut d = committed_driver(effects.clone());
        let error = d.validate(&mut ctx).await.expect_err("terminal");
        let diagnostic = format!("{error:?} {error}");
        for sentinel in sentinels {
            assert!(
                !diagnostic.contains(sentinel),
                "the refusal carries {sentinel:?}: {diagnostic}"
            );
        }

        // A socket effect the Provider's shape must never run stays silent,
        // and an unobservable socket reports the closed state rather than an
        // error string.
        effects.observe(None).await;
        let mut ctx = fixture_with(
            committed_row("compositor", &compositor_shape(), 2),
            CommittedManager::new(Vec::new()),
        );
        let mut d = committed_driver(effects.clone());
        d.reconcile(&mut ctx).await.expect("a committed pass");
        assert!(!published(&mut ctx).to_string().contains(sentinel_error));
        assert!(effects.socket_calls().await.is_empty());
        assert_eq!(EndpointConnectability::Unavailable.slug(), "unavailable");
    }

    /// A handle is minted with at least the KTD8 width, and a narrower one is
    /// refused rather than installed as the fence a launch gate compares
    /// (KTD8).
    #[test]
    fn a_handle_below_the_ktd8_width_is_refused() {
        let narrow = BoundedToken::parse("abc123").expect("a bounded token");
        assert!(
            RealizationHandle::mint(narrow, 1).is_none(),
            "a source that cannot state {} bits of entropy is not installed",
            MIN_REALIZATION_NONCE_CHARS * 4
        );
    }

    /// The token is unpredictable, rotates on replacement and on daemon
    /// restart, never collides, and cannot be reproduced from a locator
    /// (KTD8).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn incarnations_rotate_and_never_come_from_a_locator() {
        let effects = CommittedEffects::display();
        let compositor_pass = |effects: Arc<CommittedEffects>| async move {
            let mut ctx = fixture_with(
                committed_row("compositor", &compositor_shape(), 2),
                CommittedManager::new(Vec::new()),
            );
            let mut d = committed_driver(effects.clone());
            d.reconcile(&mut ctx).await.expect("a committed pass");
            token(&published(&mut ctx))
        };

        let first = handle("1111111111111111aaaaaaaaaaaaaaaa", 1);
        effects.observe(Some(first.clone())).await;
        let standing = compositor_pass(effects.clone()).await;
        // The same realization observed twice is the same incarnation: the
        // comparison a launch gate makes depends on that.
        assert_eq!(compositor_pass(effects.clone()).await, standing);

        // A replacement at the same locator rotates it.
        effects.observe(Some(handle("2222222222222222bbbbbbbbbbbbbbbb", 2))).await;
        let replaced = compositor_pass(effects.clone()).await;
        assert_ne!(replaced, standing);

        // The rotation counter is an input in its own right: a daemon that
        // rebinds and re-mints the SAME value still names a different
        // realization than it did at the earlier rotation.
        effects.observe(Some(handle("1111111111111111aaaaaaaaaaaaaaaa", 2))).await;
        assert_ne!(compositor_pass(effects.clone()).await, standing);

        // A daemon restart mints a new handle for the same locator, so a
        // token an earlier daemon published cannot be replayed against this
        // one's.
        effects.observe(Some(handle("3333333333333333cccccccccccccccc", 3))).await;
        let restarted = compositor_pass(effects.clone()).await;
        assert_ne!(restarted, standing);
        assert_ne!(restarted, replaced);

        // Distinct realizations never collide, and none of them is a function
        // of anything a reader can see: the locator the daemon holds is not an
        // input, so two handles of equal width differ only because the daemon
        // minted them apart.
        let mut seen: Vec<String> = Vec::new();
        for index in 0_u64..64 {
            effects.observe(Some(handle(&format!("{index:032x}"), index))).await;
            let candidate = compositor_pass(effects.clone()).await;
            assert!(
                !seen.contains(&candidate),
                "two realizations must not share an incarnation: {candidate}"
            );
            seen.push(candidate);
        }
        assert!(
            seen.iter().all(|value| value.starts_with("incarnation-")),
            "the published value is the opaque token, never a locator: {seen:?}"
        );

        // Two endpoints over ONE handle are still two realizations: the
        // endpoint's own row identity is part of the derivation, so a shared
        // socket cannot lend one row a consumer's token.
        effects.observe(Some(first)).await;
        let mut other = fixture_with(
            committed_row("compositor-two", &compositor_shape(), 2),
            CommittedManager::new(Vec::new()),
        );
        let mut d = committed_driver(effects.clone());
        d.reconcile(&mut other).await.expect("a committed pass");
        assert_ne!(standing, token(&published(&mut other)));
    }

    /// Restart recovery re-observes the evidence before it may republish
    /// `Ready`, and names no incarnation it did not witness (R18).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn restart_recovery_reobserves_before_republishing_ready() {
        let effects = CommittedEffects::display();
        effects.observe(Some(handle("4444444444444444dddddddddddddddd", 1))).await;
        let mut ctx = fixture_with(
            committed_row("compositor", &compositor_shape(), 2),
            CommittedManager::new(Vec::new()),
        );
        let mut before_restart = committed_driver(effects.clone());
        before_restart.reconcile(&mut ctx).await.expect("a committed pass");
        let published_before = token(&published(&mut ctx));

        // The daemon restarts: the socket is rebound, the daemon mints a new
        // handle, and the actor is a fresh one that has witnessed nothing.
        effects.observe(Some(handle("5555555555555555eeeeeeeeeeeeeeee", 2))).await;
        let mut ctx = fixture_with(
            committed_row("compositor", &compositor_shape(), 2),
            CommittedManager::new(Vec::new()),
        );
        let mut after_restart = committed_driver(effects.clone());
        assert_eq!(
            after_restart
                .recover(&mut ctx)
                .await
                .expect("recovery re-observes"),
            RecoveryOutcome::Adopted,
            "recovery adopts only after re-observing the evidence"
        );
        let projection = published(&mut ctx);
        assert_eq!(
            projection.pointer("/endpoint/readiness").and_then(|v| v.as_str()),
            Some("realized")
        );
        assert!(
            projection.pointer("/endpoint/incarnation").is_none(),
            "a restart publishes no adoption token: {projection}"
        );

        // The next pass derives this daemon's own token.
        after_restart.reconcile(&mut ctx).await.expect("a committed pass");
        assert_ne!(
            published_before,
            token(&published(&mut ctx)),
            "a token minted before the restart cannot be replayed after it"
        );
    }

    /// The three committed shapes derive exactly the two canonical binding
    /// rows the session really has, and the frontend's own endpoint derives
    /// none (R20).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_three_committed_shapes_derive_exactly_two_binding_rows() {
        let manager = CommittedManager::new(vec![
            producer_view("proxy", [0x61; 16], 4),
            producer_view("frontend", [0x62; 16], 4),
        ]);
        let effects = CommittedEffects::display();
        effects.observe(Some(handle("6666666666666666ffffffffffffffff", 1))).await;
        for (name, spec) in [
            ("compositor", compositor_shape()),
            ("proxy", proxy_shape()),
            ("frontend", frontend_shape()),
        ] {
            let mut ctx = fixture_with(committed_row(name, &spec, 1), manager.clone());
            let mut d = committed_driver(effects.clone());
            d.reconcile(&mut ctx).await.expect("a committed pass");
        }
        let derived = manager.ensured().await;
        assert_eq!(
            derived.len(),
            2,
            "two relationships across three endpoints: {derived:?}"
        );
        assert!(
            derived
                .iter()
                .all(|name| name.starts_with("endpoint-binding-")),
            "and both are the family's own canonical row names: {derived:?}"
        );

        let zone = ZoneId::parse("work").expect("zone");
        let consumers = |name: &str, spec: &EndpointSpec| -> Vec<String> {
            let endpoint = ResourceRef::parse(&format!("Endpoint/{name}")).expect("endpoint ref");
            crate::binding::declared_endpoint_bindings(&zone, spec, &endpoint)
                .expect("the source derives its relationships")
                .iter()
                .map(|delivery| delivery.consumer().as_ref().to_canonical_string())
                .collect()
        };
        assert_eq!(
            consumers("compositor", &compositor_shape()),
            vec![PROXY.to_owned()],
            "the host proxy consumes the compositor socket"
        );
        assert_eq!(
            consumers("proxy", &proxy_shape()),
            vec![FRONTEND.to_owned()],
            "the guest frontend attaches to the proxy's own endpoint"
        );
        assert!(
            consumers("frontend", &frontend_shape()).is_empty(),
            "the guest frontend's endpoint gates aggregate readiness and publishes none"
        );
    }

    /// A committed pass publishes in memory only: it writes no durable status
    /// and commits only the relationships it derived (R13).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_committed_pass_publishes_only_in_memory() {
        let manager = CommittedManager::new(vec![producer_view("proxy", [0x61; 16], 4)]);
        let mut ctx = fixture_with(committed_row("proxy", &proxy_shape(), 1), manager.clone());
        let mut d = committed_driver(CommittedEffects::display());
        assert_eq!(
            d.reconcile(&mut ctx).await.expect("a committed pass"),
            ReconcileOutcome::Satisfied
        );
        // The actor owns the status: this pass left exactly one projection in
        // the context and committed only `EndpointBinding` children.
        assert!(published(&mut ctx).pointer("/endpoint").is_some());
        let derived = manager.ensured().await;
        assert_eq!(derived.len(), 1);
        assert!(
            derived[0].starts_with("endpoint-binding-"),
            "the pass commits a relationship and nothing else: {derived:?}"
        );
        assert!(
            derived.iter().all(|name| name != "proxy"),
            "an endpoint row never re-commits itself as a child"
        );
    }
}

