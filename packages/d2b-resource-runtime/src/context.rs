//! ResourceContext - the capability surface handed to drivers (U4, KTD3;
//! spec sections 12, 14, 15).

pub const MODULE_NAME: &str = "context";
use std::any::Any;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{FailureComparison, ResourceError};
use crate::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use crate::manager::ResourceView;
use crate::spec_store::EnsureOutcome;

// ---------------------------------------------------------------------------
// Long effects (R5; spec section 14)
// ---------------------------------------------------------------------------

/// Runtime-only identifier of one spawned long effect (R5, R6: never
/// persisted). Drivers obtain ids through
/// [`ResourceContext::begin_operation`] and completion arrives as
/// [`EffectCompleted`] carrying the same id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationId(u64);

/// Result of one long effect (spec section 14). Failures are reported only
/// through the structured [`crate::error::DriverFailure`] surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectResult {
    /// The external effect completed successfully.
    Completed,
    /// The effect failed; the actor owns retry/backoff from the closed
    /// class (R13).
    Failed(crate::error::DriverFailure),
}

/// Typed completion message for a spawned long effect (spec section 14).
/// Drivers spawn the work with tokio and report through
/// [`ResourceContext::effect_sender`]; U3 wires this type into the resource
/// actor's mailbox so reconcile continues without the mailbox ever having
/// blocked on the effect.
#[derive(Debug)]
pub struct EffectCompleted {
    pub operation: OperationId,
    pub result: EffectResult,
}

// ---------------------------------------------------------------------------
// Internal watches (R12; spec section 15)
// ---------------------------------------------------------------------------

/// Minimal internal watch condition, evaluated by the target actor against
/// its in-memory status (never the store, R12).
///
/// A condition is either LEVEL or EDGE, and the target actor answers the two
/// differently:
///
/// - LEVEL ([`Self::Ready`]) answers whether the target is ready NOW. It is
///   the only shape that may be satisfied on arrival, and so it is the only
///   shape a registration cannot hold unconditionally: a target that already
///   reports ready spends the registration the moment it lands.
/// - EDGE ([`Self::ReadyChanged`], [`Self::ProjectionChanged`]) answers
///   whether THIS transition moved the thing the condition names. No
///   transition moves it, no arrival satisfies it, so a subscriber can hold
///   one unconditionally and learns about every later change under a phase
///   that never moves - including a readiness phase STOPPING.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchCondition {
    /// Satisfied when the watched resource's status reports ready.
    ///
    /// Satisfaction wakes the *subscriber's actor*
    /// (`ResourceMsg::DependencySatisfied`), which reconciles; the driver
    /// then proves readiness by reading the observed state back through
    /// [`ResourceContext::get_view`] (the watch is the wake-up, the read is
    /// the proof). This is the seam readiness checks use.
    Ready,
    /// Satisfied when the watched resource ENTERS or LEAVES `Ready`.
    ///
    /// [`Self::Ready`] cannot express the second half of that: a target that
    /// already reports ready has the arrival answer spent before the
    /// registration can stand, so a subscriber watching a ready target holds
    /// nothing and is never told when the target stops being ready - a
    /// degraded row that publishes no new projection (a lost target, a
    /// refused operation) keeps reporting `Ready` and stays silent. This
    /// condition is the other half, and it is never answered on arrival, so
    /// the same subscriber can hold it for a ready target as well.
    ReadyChanged,
    /// Named custom predicate; the target actor's driver supplies the
    /// predicate implementation by id.
    ///
    /// Not implemented: the target actor evaluates every custom predicate as
    /// unsatisfied, so readiness is expressed with [`Self::Ready`] until the
    /// driver-supplied predicate hook lands (U6+).
    Custom(String),
    /// Satisfied when the watched resource publishes a NEW `status.resource`
    /// projection.
    ///
    /// A readiness phase cannot express a delivery downgrade: a relationship
    /// whose endpoint was replaced, or whose authorization was withdrawn,
    /// keeps reporting `Ready` while its evidence layer changes underneath.
    /// This condition is how a dependent learns about that change (R21): the
    /// target actor notifies on every transition that carries a projection
    /// different from the one it published before, so the subscriber re-reads
    /// the evidence instead of waiting for a phase that never changes.
    ProjectionChanged,
}

/// Runtime-only watch id allocated by the target actor at registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchId(pub u64);

/// Notification delivered into the subscribing actor's mailbox when a
/// registered condition is satisfied. Exactly once per registration (AE2):
/// evaluation and registration serialize in the target actor's mailbox, so
/// a condition that flips while the registration message is still queued
/// still notifies exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSatisfied {
    pub watch: WatchId,
    pub target: ResourceKey,
}

/// One internal watch registration (R12; spec section 15).
///
/// ATOMICITY INVARIANT (AE2): the target actor must evaluate the condition
/// and register the watch in ONE mailbox handler:
///
/// ```text
/// if condition_is_level && condition_holds_now(status) {
///     notify(subscriber, Satisfied)
/// } else {
///     watchers.insert(watch_id, ...)
/// }
/// ```
///
/// An EDGE condition is never satisfied there, because no transition is
/// being evaluated: it holds from the first transition that moves it.
///
/// Status transitions evaluate registered watches in the same handler. U3
/// enforces this by construction inside `ResourceActor`; the shapes here
/// carry the contract (a registration is a message, delivery goes through
/// the subscriber's mailbox sender, and nothing is persisted).
#[derive(Debug)]
pub struct WatchRegistration {
    /// The resource whose status is watched.
    pub target: ResourceKey,
    pub condition: WatchCondition,
    /// The subscriber's mailbox sender for [`WatchSatisfied`].
    pub notify: mpsc::UnboundedSender<WatchSatisfied>,
}

// ---------------------------------------------------------------------------
// Manager routing (R2; spec sections 12, 16)
// ---------------------------------------------------------------------------

/// Desired child row payload for a parent actor's child mutation (R8, R9).
/// The manager derives the child's durable identity from the parent key
/// (`zone`, `type_name`, `name`), persists ownership (`owner_uid` =
/// parent's uid, provenance `ResourceProvenance::Resource`), and commits
/// BEFORE creating or updating the child actor (F1).
#[derive(Debug, Clone)]
pub struct ChildEnsure {
    pub type_name: ResourceTypeName,
    pub name: String,
    /// Opaque encoded spec envelope for the child row.
    pub spec: Vec<u8>,
    /// Opaque encoded metadata envelope (finalizers, annotations, ...).
    pub metadata: Vec<u8>,
}

/// The injected manager endpoint behind a [`ResourceContext`] (KTD3): every
/// durable child mutation and every internal-watch registration rides this
/// surface, never the spec store. U3's manager implements it directly or
/// over its mailbox.
#[async_trait]
pub trait ManagerEndpoint: Send + Sync + 'static {
    async fn ensure_child(
        &self,
        parent: &ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError>;
    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError>;
    /// Live runtime view of one resource, its published status included
    /// (the manager's in-memory projection; see
    /// [`ResourceContext::get_view`] for the absent/unknown semantics).
    async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError>;
    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError>;
    async fn list_owned(&self, owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError>;
    async fn register_watch(
        &self,
        subscriber: &ResourceKey,
        registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError>;
    async fn cancel_watch(&self, watch: WatchId) -> Result<(), ResourceError>;

    /// Every committed row of `type_name` in `zone` (R18, R22).
    ///
    /// The completeness scope of a row with NO owner: an owner-scoped listing
    /// answers an empty set for it without asking anyone, so the only scope
    /// that can say what the Zone publishes for this consumer is the Zone
    /// itself. Committed rows and not views: a reader names the rows it must
    /// observe and reads each one through [`Self::view`] itself.
    ///
    /// The default refuses rather than answering an empty set. An empty set is
    /// a STATEMENT - "the Zone holds no such row" - and a default that
    /// returned one would let every endpoint that cannot carry the read mint a
    /// launch with no delivery at all. Refusing leaves the read unproven, which
    /// is what a plane that cannot answer it honestly is.
    async fn list_zone_type(
        &self,
        _zone: &str,
        _type_name: &str,
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        Err(ResourceError::ManagerRejected {
            reason: "endpoint carries no zone-scoped row listing".to_owned(),
        })
    }

    /// Create a source-owned binding under the source controller's
    /// authenticated authority (U6, KTD2).
    ///
    /// KTD2 routes a typed consumer request through the source controller: the
    /// controller admits the request and then asks the manager to create the
    /// source-owned binding. That second step is this method, and it carries
    /// the controller's authenticated evidence - not the owner's name - so a
    /// driver cannot mint a relationship by holding a parent reference.
    ///
    /// The default refuses. An endpoint that cannot carry that evidence must
    /// not quietly fall back to [`Self::ensure_child`], which is the
    /// un-authenticated path KTD2 removes; refusing keeps the fallback
    /// unreachable rather than merely unused.
    async fn ensure_source_owned_binding(
        &self,
        _parent: &ResourceKey,
        _evidence: crate::manager::AuthenticatedMutation,
        _child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        Err(ResourceError::ManagerRejected {
            reason: "endpoint carries no authenticated source-controller authority".to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// Lookup classification (issue #511)
// ---------------------------------------------------------------------------

/// The plane one row read was answered from (issue #511). Every
/// [`RowLookup`] records it, so a status, a log line, or a requeue reason
/// can name where an answer came from instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupPlane {
    /// The v3 manager plane: the manager's row projection, the authority of
    /// every converted resource's actor.
    Manager,
}

impl LookupPlane {
    /// The stable label both sides of a comparison render.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manager => "manager",
        }
    }
}

impl std::fmt::Display for LookupPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One cross-plane row read's classification (issue #511): the single
/// canonical answer to "is the row there yet?", "can the plane answer?",
/// and "did the read fail?" - so no read site invents its own mapping from
/// "nothing there" to defer-or-fail.
///
/// The defaults are issue #511's "defer unless proven terminal", fixed here
/// rather than re-explained at each call site:
///
/// - [`RowLookup::Present`] - the plane answered with the row; proceed.
/// - [`RowLookup::Absent`] - the plane answered and holds no row. This is
///   the row-not-created-yet answer: **defer with a requeue**, never
///   terminal, whatever the caller expected to find.
/// - [`RowLookup::Unavailable`] - the plane could not answer (manager RPC
///   failure, no published plane, unreachable store, uncommitted identity).
///   Retryable by construction: **defer with a requeue**, never reported as
///   absence.
/// - [`RowLookup::Error`] - the read answered with an unusable payload and
///   this detail (a row that does not decode, a malformed projection).
///   **Defer with a requeue as well**: even a failed read is not terminal by
///   itself.
///
/// The default for a non-`Present` answer is to defer and requeue. A call
/// site that fails terminal must first name its terminal evidence - the
/// committed row that cannot decode, the structurally invalid spec - and
/// surface it (status projection or log), so a terminal status is always
/// evidence-backed and never inferred from absence or from an unanswered
/// plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowLookup<T> {
    /// The plane answered with the row.
    Present { row: T, plane: LookupPlane },
    /// The plane answered and holds no row.
    Absent { plane: LookupPlane },
    /// The plane could not answer.
    Unavailable { plane: LookupPlane },
    /// The read answered with an unusable payload and this detail.
    Error { plane: LookupPlane, detail: String },
}

impl<T> RowLookup<T> {
    /// The compared values one non-present lookup yields (issue #508): the
    /// caller expected `expected`, and the plane answered with the observed
    /// side this renders. `Present` returns `None` (the read proceeded).
    ///
    /// One shared projection, so every read site names the same field and the
    /// same observed answer instead of inventing its own wording.
    pub fn failure_comparison(
        &self,
        field: &'static str,
        expected: &str,
    ) -> Option<FailureComparison> {
        let observed = match self {
            Self::Present { .. } => return None,
            Self::Absent { plane } => format!("absent (plane={plane})"),
            Self::Unavailable { plane } => format!("unavailable (plane={plane})"),
            Self::Error { plane, .. } => format!("unreadable (plane={plane})"),
        };
        Some(FailureComparison::new(field, expected, observed))
    }

    /// The read's own detail when the plane answered with an unusable
    /// payload; for the failure note (bounded at construction).
    pub fn error_detail(&self) -> Option<&str> {
        match self {
            Self::Error { detail, .. } => Some(detail.as_str()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Requeue (R13; spec section 32)
// ---------------------------------------------------------------------------

/// Runtime-only requeue id returned by [`ResourceContext::requeue_after`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequeueId(pub u64);

/// Sink for runtime-only requeue schedules (R13). The actor implementation
/// translates a schedule into exactly one ractor timer delivering one
/// reconcile, and cancels pending schedules on the delete path. Nothing is
/// persisted: after restart the actor reconciles immediately instead of
/// restoring timers.
pub trait RequeueScheduler: Send + Sync + 'static {
    /// Schedule exactly one reconcile for `key` after `after`.
    fn schedule(&self, key: ResourceKey, after: Duration) -> RequeueId;
    /// Cancel a pending schedule (no-op when already delivered).
    fn cancel(&self, id: RequeueId);
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// Decode hook the manager wires into each context: typed per resource type
/// at wiring time, object-erased here. The decoded value is cached in the
/// context; [`ResourceContext::spec`] downcasts it.
pub trait SpecDecoder: Send + Sync + 'static {
    fn decode(&self, envelope: &[u8]) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Build an erased [`SpecDecoder`] from a typed decode function (one per
/// resource type; the manager wires it when constructing the context).
pub fn typed_spec_decoder<T, F, E>(decode: F) -> Arc<dyn SpecDecoder>
where
    T: Any + Send,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn(&[u8]) -> Result<T, E> + Send + Sync + 'static,
{
    struct FnDecoder<F>(F);

    impl<F> SpecDecoder for FnDecoder<F>
    where
        F: Fn(&[u8]) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>>
            + Send
            + Sync
            + 'static,
    {
        fn decode(&self, envelope: &[u8]) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            (self.0)(envelope)
        }
    }

    Arc::new(FnDecoder(move |envelope: &[u8]| {
        decode(envelope)
            .map(|decoded| Box::new(decoded) as Box<dyn Any + Send>)
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
    }))
}

// ---------------------------------------------------------------------------
// ResourceContext
// ---------------------------------------------------------------------------

/// The capability surface handed to drivers (KTD3; spec section 12).
///
/// One instance per resource actor; the actor rebuilds it on generation
/// change and passes `&mut` into the driver calls. Invariants:
///
/// - Drivers never touch the spec store: child mutations, lookups, and
///   internal-watch registrations route through the injected
///   [`ManagerEndpoint`] (R2), whose replies fire only after the manager's
///   commit (F1, AE1).
/// - No blocking call exists on this surface (KTD12): durable operations are
///   async request/reply, requeue is a non-blocking schedule call, and long
///   effects are spawned (`begin_operation` + `effect_sender`) with
///   completion arriving as a typed [`EffectCompleted`] message (R5).
/// - Status and spec are in-memory only (R6, R11): the status slot is an
///   erased value in the actor, the spec decodes from the stored envelope
///   through the wired [`SpecDecoder`] and is cached.
pub struct ResourceContext {
    row: StoredDesiredResource,
    decoder: Arc<dyn SpecDecoder>,
    manager: Arc<dyn ManagerEndpoint>,
    requeue: Arc<dyn RequeueScheduler>,
    effects: mpsc::UnboundedSender<EffectCompleted>,
    watch_notify: mpsc::UnboundedSender<WatchSatisfied>,
    next_operation: u64,
    decoded_spec: OnceCell<Box<dyn Any + Send>>,
    status: Option<Box<dyn Any + Send>>,
    /// Free-form status projection for the read surfaces (R11: in-memory
    /// only, never persisted). The erased status slot above is typed and
    /// driver-private; this is the closed-JSON `status.resource` layer the
    /// manager renders onto the wire for rows whose consumer contract
    /// carries one (the Cloud Hypervisor Guest runtime status).
    status_projection: Option<serde_json::Value>,
    /// The owning resource's key, resolved by the manager from the row's
    /// owner uid. Drivers select their launch shape from it; `None` for
    /// roots and for rows whose owner row is not in this manager.
    owner_key: Option<crate::identity::ResourceKey>,
    /// The committed target binding the manager resolved for this row (R19,
    /// R29). It carries the exact execution target, the committed uid, and -
    /// for a Guest-targeted row - the live session generation, so a driver
    /// reaches the authenticated target session through the binding instead
    /// of through a coarse handle that said nothing about authority.
    target: Option<crate::target::TargetBinding>,
    /// The internal-watch registrations this row holds that are still
    /// standing in their target actor's mailbox, each mapped to the target
    /// that holds it.
    ///
    /// This is the runtime's own record of what this row is subscribed to,
    /// and it is COMPLETE: a registration leaves it in exactly two cases,
    /// both of which this row is told about - the target notified on it
    /// ([`Self::mark_watch_spent`], AE2) or the target's actor went away
    /// ([`Self::mark_target_watches_lost`], spec section 16). Nothing else
    /// ends a registration's life, so a driver never has to guess from
    /// evidence whether it is still subscribed: a target pass can satisfy
    /// every registration it holds and republish the very pair it published
    /// before, which is exactly the case no comparison of evidence can read.
    live_watches: HashMap<WatchId, ResourceKey>,
}

impl ResourceContext {
    /// Assembled by the resource actor (U3) from the stored row and the
    /// manager-wired hooks.
    pub fn new(
        row: StoredDesiredResource,
        decoder: Arc<dyn SpecDecoder>,
        manager: Arc<dyn ManagerEndpoint>,
        requeue: Arc<dyn RequeueScheduler>,
        effects: mpsc::UnboundedSender<EffectCompleted>,
        watch_notify: mpsc::UnboundedSender<WatchSatisfied>,
    ) -> Self {
        Self {
            row,
            decoder,
            manager,
            requeue,
            effects,
            watch_notify,
            next_operation: 1,
            decoded_spec: OnceCell::new(),
            status: None,
            status_projection: None,
            owner_key: None,
            target: None,
            live_watches: HashMap::new(),
        }
    }

    /// Carry this row's outstanding internal-watch registrations onto a
    /// rebuilt context (spec change, generation move).
    ///
    /// The registrations live in the target actors' mailboxes, not in this
    /// context, so a rebuild that forgot them would make every id this row
    /// remembers read as spent and re-arm duplicates on top of registrations
    /// that are still standing.
    pub(crate) fn take_live_watches(&mut self) -> HashMap<WatchId, ResourceKey> {
        std::mem::take(&mut self.live_watches)
    }

    /// Adopt the registrations carried onto a rebuilt context (the actor's
    /// spec-change path).
    pub(crate) fn with_live_watches(mut self, live: HashMap<WatchId, ResourceKey>) -> Self {
        self.live_watches = live;
        self
    }

    /// Attach the owning resource's key (manager-resolved).
    pub fn with_owner_key(mut self, owner_key: Option<crate::identity::ResourceKey>) -> Self {
        self.owner_key = owner_key;
        self
    }

    /// Attach the committed target binding (manager-resolved, U13).
    pub fn with_target(mut self, target: crate::target::TargetBinding) -> Self {
        self.target = Some(target);
        self
    }

    /// The committed target binding this row realizes through.
    ///
    /// `None` only for a context assembled outside the resource actor (a unit
    /// test driving a driver directly), where no target layer exists. A row
    /// the manager committed always has one, including a Host-targeted row:
    /// its binding reports [`TargetBinding::is_guest`] as `false` and the
    /// driver runs its effects locally.
    pub fn target(&self) -> Option<&crate::target::TargetBinding> {
        self.target.as_ref()
    }

    /// The owning resource's key, when this resource is an owned child and
    /// its owner row is known to this manager. Drivers that key their launch
    /// intent on the owner (provider controllers, binding-owned workers)
    /// read it from here.
    pub fn owner_key(&self) -> Option<&crate::identity::ResourceKey> {
        self.owner_key.as_ref()
    }

    /// Durable identity of the resource being driven.
    pub fn key(&self) -> &ResourceKey {
        &self.row.key
    }

    /// Stable 16-byte resource uid (survives generation changes).
    pub fn uid(&self) -> &[u8; 16] {
        &self.row.uid
    }

    /// Durable generation of the current desired spec.
    pub fn generation(&self) -> u64 {
        self.row.generation
    }

    /// Owner resource uid, when this resource is an owned child (R8).
    ///
    /// Deviation from spec section 12's `owner() -> Option<&ResourceKey>`:
    /// the U2 row persists the owner by uid only; the manager maps uid to
    /// key where a key is needed.
    pub fn owner(&self) -> Option<&[u8; 16]> {
        self.row.owner_uid.as_ref()
    }

    /// Opaque authored metadata envelope of the current desired row.
    ///
    /// The manager resolves an owner key only for owners that are rows it
    /// manages; a driver that must name an owner which is not a managed row
    /// (an owned child of an unconverted resource) reads the authored owner
    /// reference here. The spec itself is reached through [`Self::spec`].
    pub fn metadata(&self) -> &[u8] {
        &self.row.metadata
    }

    /// Typed decode of the stored spec envelope through the wired hook.
    /// Decoded once per context, then cached; decode failures and type
    /// mismatches surface as [`ResourceError::SpecDecode`].
    pub fn spec<T: Any + Send>(&self) -> Result<&T, ResourceError> {
        if self.decoded_spec.get().is_none() {
            let decoded = self
                .decoder
                .decode(&self.row.spec)
                .map_err(|source| ResourceError::SpecDecode { key: self.row.key.clone(), source })?;
            let _ = self.decoded_spec.set(decoded);
        }
        self.decoded_spec
            .get()
            .expect("decoded above")
            .downcast_ref::<T>()
            .ok_or_else(|| ResourceError::SpecDecode {
                key: self.row.key.clone(),
                source: "decoded spec type does not match the requested type".to_string().into(),
            })
    }

    /// In-memory status slot (R11): runtime-only, zero persistent writes.
    /// `None` when unset or when the stored status is not `T`.
    pub fn status<T: Any>(&self) -> Option<&T> {
        self.status.as_ref().and_then(|status| status.downcast_ref::<T>())
    }

    /// Replace the in-memory status slot (R11: never persisted).
    pub fn set_status<T: Any + Send>(&mut self, status: T) {
        self.status = Some(Box::new(status));
    }

    /// Publish the wire-visible `status.resource` layer of this row (R11:
    /// in-memory only). The actor takes it after the pass that set it and the
    /// manager renders it onto the row's status; a driver that publishes no
    /// projection leaves the layer empty, exactly as today.
    pub fn set_status_projection(&mut self, projection: serde_json::Value) {
        self.status_projection = Some(projection);
    }

    /// Take the pending status projection, if any. Called by the actor after
    /// each driver pass: the projection belongs to the pass that set it, so
    /// it is consumed, never carried into a later status.
    pub fn take_status_projection(&mut self) -> Option<serde_json::Value> {
        self.status_projection.take()
    }

    /// Fetch a resource row by key through the manager.
    ///
    /// The canonical classified form is [`Self::lookup`] (issue #511):
    /// prefer it in new code so absence, an unanswerable plane, and a failed
    /// read stay distinct.
    pub async fn get(&mut self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        self.manager.get(key).await
    }

    /// Fetch a resource row by key through the manager, classified per
    /// issue #511: `Present` carries the row, `Absent` is the honest
    /// not-(yet)-created answer, and a manager that cannot answer is
    /// `Unavailable` - never absence. The default for a non-`Present`
    /// answer is to defer and requeue unless the call site has named
    /// terminal evidence.
    pub async fn lookup(&mut self, key: &ResourceKey) -> RowLookup<StoredDesiredResource> {
        match self.manager.get(key).await {
            Ok(Some(row)) => RowLookup::Present {
                row,
                plane: LookupPlane::Manager,
            },
            Ok(None) => RowLookup::Absent {
                plane: LookupPlane::Manager,
            },
            Err(_) => RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            },
        }
    }

    /// Live state of another resource (KTD3): the manager's in-memory
    /// runtime view for `key` - the committed row plus the status its actor
    /// last published (R11: memory only, zero store reads) and the row
    /// generation that status was published for.
    ///
    /// This is *observed* state, and the answer is explicit about what is
    /// not known:
    ///
    /// - `Ok(None)`: **absent** - no row for `key` exists in this manager's
    ///   Zone (never created, already retired, or owned by another Zone).
    /// - `Ok(Some(view))` with `view.status == None`: the row exists but no
    ///   actor has ever published a status for it (spawn still in flight,
    ///   poisoned spawn, actor restart). **Unknown**, never "not ready".
    /// - `Ok(Some(view))` with
    ///   `view.status_generation != Some(view.generation)`: the last
    ///   published status describes an older generation, so it is not
    ///   observed state of the current row.
    ///   [`ResourceView::observed_status`] folds both of the last two cases
    ///   into `None`.
    /// - `Err(ResourceError::ManagerUnavailable(_))`: the manager could not
    ///   answer; `Err(ResourceError::ManagerRejected { .. })`: the manager
    ///   refused the call.
    ///   Never reported as absence.
    ///
    /// Readiness of a child or dependency is therefore
    /// `view.observed_status() == Some(ResourceStatus::Ready)`; when the
    /// answer is not-ready, a [`Self::watch`] on [`WatchCondition::Ready`]
    /// wakes this resource's actor on the transition and the next reconcile
    /// re-reads here.
    ///
    /// The read is one manager mailbox round-trip answered from the
    /// manager's in-memory projection (no store access, KTD12), and it
    /// blocks nothing but the calling driver's own await - the same shape as
    /// [`Self::get`]. The classified form is [`Self::lookup_view`] (issue
    /// #511).
    pub async fn get_view(&mut self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        self.manager.view(key).await
    }

    /// The [`RowLookup`] form of [`Self::get_view`] (issue #511): the
    /// manager plane's live runtime view for `key`, classified so absence
    /// (`Absent` - no row exists) stays distinct from a manager that cannot
    /// answer (`Unavailable`). A non-`Present` answer defers by default
    /// unless the call site names terminal evidence.
    pub async fn lookup_view(&mut self, key: &ResourceKey) -> RowLookup<ResourceView> {
        match self.manager.view(key).await {
            Ok(Some(view)) => RowLookup::Present {
                row: view,
                plane: LookupPlane::Manager,
            },
            Ok(None) => RowLookup::Absent {
                plane: LookupPlane::Manager,
            },
            Err(_) => RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            },
        }
    }

    /// Ensure an owned child resource through the manager (R8, R9): the
    /// manager commits the child row BEFORE creating or updating the child
    /// actor, and the reply fires only after that commit (F1, AE1). Child
    /// identity is deterministic from the parent key and the child type +
    /// name; the manager derives uid, generation, ownership, and provenance.
    pub async fn ensure_child(&mut self, child: ChildEnsure) -> Result<EnsureOutcome, ResourceError> {
        self.manager.ensure_child(&self.row.key, child).await
    }

    /// Delete a resource through the manager (R10: the manager marks
    /// deleting durably before cleanup starts).
    pub async fn delete(&mut self, key: &ResourceKey) -> Result<(), ResourceError> {
        self.manager.delete(key).await
    }

    /// Rows of the resources this resource owns (R8, R9).
    pub async fn children(&mut self) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        self.manager.list_owned(self.row.uid).await
    }

    /// Rows owned by this resource's OWNER (R8).
    ///
    /// A dependent derives what it requires from the committed declarations
    /// its owner publishes, and those declarations are siblings: a session
    /// owns the `Process` rows and the `Endpoint` rows together, so the
    /// endpoints a process consumes are this row's siblings rather than its
    /// children. The read is the existing owner-scoped listing applied to the
    /// owner uid this row already carries, so it adds no new manager surface.
    ///
    /// `Vec::new()` for a row with no owner: a root row has no siblings. That
    /// is what the listing can answer, and it is NOT a statement that nothing
    /// publishes for this row - the Zone is, and [`Self::zone_rows`] is how a
    /// reader asks it.
    pub async fn owner_siblings(&mut self) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        match self.row.owner_uid {
            Some(owner) => self.manager.list_owned(owner).await,
            None => Ok(Vec::new()),
        }
    }

    /// Committed rows of `type_name` in this row's own Zone (R18, R22).
    ///
    /// The completeness scope for a row with no owner: a root Process is
    /// admitted through the API or a Nix ingest rather than as a session child,
    /// so its owner-scoped listing answers an empty set without asking anyone,
    /// and only a Zone-scoped listing can say what the Zone publishes for it.
    /// It is deliberately NOT the scope for an owned row, whose committed child
    /// set is settled by its owner's own publication.
    pub async fn zone_rows(
        &mut self,
        type_name: &str,
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        self.manager.list_zone_type(&self.row.key.zone, type_name).await
    }

    /// Finalize every resource this one owns, children first (F3; owner
    /// directive 2026-09-11): the parent's `finalize` handler runs this before
    /// its own drain work, so no resource tears down ahead of what it owns.
    ///
    /// Depth-first by construction: each owned child reaches this same call
    /// from its own finalize pass, so a grandchild is finalized before the
    /// child that owns it.
    ///
    /// Idempotent under retry, and non-blocking (R5): every owned child's
    /// deletion is (re)requested - the manager's `Remove` is idempotent and
    /// the child's own finalize/delete are retry-idempotent - and the call
    /// then reports [`ResourceError::ChildrenDraining`] while any owned child
    /// row is still live. The caller classifies that retryable and requeues,
    /// so the parent never holds its mailbox waiting on a child.
    pub async fn finalize_owned_resources(&mut self) -> Result<(), ResourceError> {
        let owned = self.manager.list_owned(self.row.uid).await?;
        if owned.is_empty() {
            return Ok(());
        }
        for child in &owned {
            // The manager already cascaded the deletion when this resource
            // was marked deleting; requesting it again is the idempotent
            // nudge that guarantees a child whose actor missed the first
            // cascade (e.g. spawned between passes) runs its own
            // finalize-before-delete pass.
            let _ = self.manager.delete(&child.key).await;
        }
        Err(ResourceError::ChildrenDraining {
            zone: self.row.key.zone.clone(),
            type_name: self.row.key.type_name.clone(),
            name: self.row.key.name.clone(),
        })
    }

    /// Register an internal watch on another resource (R12; spec section
    /// 15). Routed through the manager to the target actor, which evaluates
    /// and registers in one mailbox handler (AE2); satisfaction arrives as
    /// [`WatchSatisfied`] on this resource's notify channel.
    pub async fn watch(
        &mut self,
        target: ResourceKey,
        condition: WatchCondition,
    ) -> Result<WatchId, ResourceError> {
        let watch = self
            .manager
            .register_watch(
                &self.row.key,
                WatchRegistration {
                    target: target.clone(),
                    condition,
                    notify: self.watch_notify.clone(),
                },
            )
            .await?;
        // The registration exists in the target actor's mailbox from here on,
        // so this row records it - against that target - until the target
        // notifies on it or this row releases it. See [`Self::watch_is_live`].
        self.live_watches.insert(watch, target);
        Ok(watch)
    }

    /// Release one internal watch this row registered ([`Self::watch`]).
    ///
    /// A runtime registration is one-shot - AE2 satisfies and REMOVES it - so
    /// a driver that keeps one target subscribed across passes has to release
    /// the registration it is replacing. Without this the target's watcher set
    /// grows by one registration per pass and every later change on that
    /// target is delivered once per spent registration. The manager's routing
    /// record for the registration goes with it, so a release is also what
    /// keeps one manager entry per row+target from outliving the row.
    ///
    /// Releasing is idempotent and answers `Ok` for an id that is already
    /// gone: the target removed it (AE2), or this row released it before, and
    /// in both cases the registration is not standing, which is what the
    /// caller asked for. The only `Err` is a release the manager could not
    /// be asked for at all.
    pub async fn cancel_watch(&mut self, watch: WatchId) -> Result<(), ResourceError> {
        self.manager.cancel_watch(watch).await?;
        // Only a release that took is a release: the registration is gone from
        // the target's mailbox, so it is no longer standing. A refused release
        // leaves the id live and the caller its reason to retry.
        self.live_watches.remove(&watch);
        Ok(())
    }

    /// Whether `watch` is still standing in the target actor's mailbox.
    ///
    /// The answer is the runtime's own record, and it is complete: a
    /// registration stops being live in exactly two cases, both of which this
    /// row is told about. The target actor satisfied it (AE2) - the target
    /// actor notifies this row, which reaches the actor as
    /// [`crate::ResourceMsg::DependencySatisfied`] and is recorded by
    /// [`Self::mark_watch_spent`] - or the target's actor went away and took
    /// the registration with it, recorded by [`Self::mark_target_watches_lost`].
    ///
    /// So this answers exactly what a re-arm has to ask, and nothing else can:
    /// evidence that reads back unchanged cannot tell a spent registration
    /// from a standing one, because a target pass can satisfy every
    /// registration it holds and republish the very projection it published
    /// before.
    pub fn watch_is_live(&self, watch: WatchId) -> bool {
        self.live_watches.contains_key(&watch)
    }

    /// Record that the target satisfied one of this row's registrations.
    ///
    /// Called by the actor as it handles
    /// [`crate::ResourceMsg::DependencySatisfied`], which is where the target
    /// actor's notification enters this row's mailbox. The id stops being live
    /// at exactly the moment the target removed it, so the driver's next pass
    /// sees a spent registration and re-arms it (see [`Self::watch_is_live`]).
    pub fn mark_watch_spent(&mut self, watch: WatchId) {
        self.live_watches.remove(&watch);
    }

    /// Record that `target`'s actor is gone, so every registration this row
    /// held on it died with that actor.
    ///
    /// Called by the actor as it handles
    /// [`crate::ResourceMsg::DependencyChanged`], which the manager sends a
    /// dependent when the target actor exits (R17, spec section 16). The
    /// respawned actor starts with an EMPTY watcher set, so a row that still
    /// read its old ids live would place nothing, hold registrations no target
    /// has, and never be woken by that target again: the pass this message
    /// triggers is the one chance to re-arm, and it must not read them live.
    ///
    /// Registrations on every OTHER target are untouched: their actors did not
    /// go anywhere, and their records never had a reason to move.
    pub fn mark_target_watches_lost(&mut self, target: &ResourceKey) {
        self.live_watches.retain(|_, held| held != target);
    }

    /// Schedule exactly one reconcile after `after` (R13; spec section 32).
    /// Runtime-only: the schedule rides the injected [`RequeueScheduler`]
    /// and is never persisted.
    pub fn requeue_after(&mut self, after: Duration) -> RequeueId {
        self.requeue.schedule(self.row.key.clone(), after)
    }

    /// Allocate the runtime-only id for a long effect the driver is about to
    /// spawn; report completion through [`ResourceContext::effect_sender`]
    /// with the same id (R5; spec section 14).
    pub fn begin_operation(&mut self) -> OperationId {
        let id = OperationId(self.next_operation);
        self.next_operation += 1;
        id
    }

    /// Sender for typed [`EffectCompleted`] messages back to the owning
    /// resource actor (U3 wires the mailbox side).
    pub fn effect_sender(&self) -> mpsc::UnboundedSender<EffectCompleted> {
        self.effects.clone()
    }
}

// ---------------------------------------------------------------------------
// Service driver context (U3, R7)
// ---------------------------------------------------------------------------

/// The manager endpoint behind a fail-closed [`ServiceResourceContext`]:
/// every call refuses. The composition state of a zone that hosts services
/// without a wired manager seam.
struct FailClosedManager;

#[async_trait]
impl ManagerEndpoint for FailClosedManager {
    async fn ensure_child(
        &self,
        _parent: &ResourceKey,
        _child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn get(&self, _key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        _registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }

    async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
        Err(ResourceError::ManagerRejected { reason: "no manager seam".into() })
    }
}

/// The generic driver context a provider service reaches resource state
/// through (R7): the manager-plane read surface of the zone, never the spec
/// store directly.
///
/// A service is not a resource actor and holds no row of its own, so this
/// is the [`ResourceContext`] read surface without the row-bound parts: the
/// same injected [`ManagerEndpoint`] seam, exposed for service
/// invocations. The daemon builds one per zone over the manager endpoint
/// and hands it to a hosted service through the invocation's capability
/// object; a service reads resource state here and daemon-structural state
/// only through its declared state cells (R7).
#[derive(Clone)]
pub struct ServiceResourceContext {
    manager: Arc<dyn ManagerEndpoint>,
}

impl ServiceResourceContext {
    /// Wrap the zone's manager endpoint.
    pub fn over(manager: Arc<dyn ManagerEndpoint>) -> Self {
        Self { manager }
    }

    /// A fail-closed context: every read refuses. The unwired composition
    /// state of a zone that hosts services - a service invocation then
    /// reaches no resource state rather than guessing at a store.
    pub fn fail_closed() -> Self {
        Self {
            manager: Arc::new(FailClosedManager),
        }
    }

    /// The stored desired row for `key`.
    pub async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        self.manager.get(key).await
    }

    /// The manager plane's live runtime view for `key`, its published
    /// status included.
    pub async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        self.manager.view(key).await
    }

    /// The [`RowLookup`] form of [`Self::get`] (issue #511): classified so
    /// absence (`Absent` - no row exists) stays distinct from a manager
    /// that cannot answer (`Unavailable`).
    pub async fn lookup(&self, key: &ResourceKey) -> RowLookup<StoredDesiredResource> {
        match self.manager.get(key).await {
            Ok(Some(row)) => RowLookup::Present {
                row,
                plane: LookupPlane::Manager,
            },
            Ok(None) => RowLookup::Absent {
                plane: LookupPlane::Manager,
            },
            Err(_) => RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            },
        }
    }

    /// The [`RowLookup`] form of [`Self::view`] (issue #511).
    pub async fn lookup_view(&self, key: &ResourceKey) -> RowLookup<ResourceView> {
        match self.manager.view(key).await {
            Ok(Some(view)) => RowLookup::Present {
                row: view,
                plane: LookupPlane::Manager,
            },
            Ok(None) => RowLookup::Absent {
                plane: LookupPlane::Manager,
            },
            Err(_) => RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            },
        }
    }
}


#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tokio::sync::mpsc;

    use super::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
        WatchId, WatchRegistration, WatchSatisfied,
    };
    use crate::error::ResourceError;
    use crate::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use crate::manager::ResourceView;
    use crate::spec_store::EnsureOutcome;

    /// Decoder that always fails; tests wiring their own decode hooks pass
    /// [`super::typed_spec_decoder`] closures instead.
    pub(crate) struct FailingDecoder;

    impl SpecDecoder for FailingDecoder {
        fn decode(
            &self,
            _envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            Err("decode failed".into())
        }
    }

    /// Requeue scheduler that does nothing; used where requeue is not the
    /// behavior under test.
    #[derive(Clone, Default)]
    pub(crate) struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    /// Manager endpoint that always fails; used where manager calls are not
    /// the behavior under test.
    #[derive(Clone, Default)]
    pub(crate) struct DeadManager;

    #[async_trait::async_trait]
    impl ManagerEndpoint for DeadManager {
        async fn ensure_child(&self, _parent: &ResourceKey, _child: ChildEnsure) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn get(&self, _key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead manager".into()))
        }
    }

    /// Manager endpoint holding a fixed owned-child set: `list_owned`
    /// answers it, `delete` records the child keys it is asked to remove,
    /// and every other call fails loudly. Used by the erased-boundary test
    /// for the child-first finalize contract ([`super::ResourceContext::finalize_owned_resources`]).
    #[derive(Clone, Default)]
    pub(crate) struct OwnedChildrenManager {
        children: Vec<StoredDesiredResource>,
        deleted: Arc<Mutex<Vec<ResourceKey>>>,
    }

    impl OwnedChildrenManager {
        pub(crate) fn with_children(children: Vec<StoredDesiredResource>) -> Self {
            Self { children, deleted: Arc::new(Mutex::new(Vec::new())) }
        }

        /// Child keys the manager was asked to delete, in order.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        pub(crate) fn deleted_keys(&self) -> Vec<ResourceKey> {
            self.deleted.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for OwnedChildrenManager {
        async fn ensure_child(&self, _parent: &ResourceKey, _child: ChildEnsure) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected ensure_child".into() })
        }

        async fn get(&self, _key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected get".into() })
        }

        async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected view".into() })
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.deleted.lock().push(key.clone()); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(())
        }

        async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.children.clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected register_watch".into() })
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected cancel_watch".into() })
        }
    }

    /// Contract-level requeue scheduler over tokio paused time: `schedule`
    /// starts exactly one timer per call, `cancel` aborts it. The production
    /// implementation is a ractor timer inside the resource actor (U3); this
    /// fake proves the requeue contract shapes.
    #[derive(Clone)]
    pub(crate) struct TokioRequeue {
        inner: Arc<TokioRequeueInner>,
    }

    struct TokioRequeueInner {
        delivered_tx: mpsc::UnboundedSender<RequeueId>,
        delivered_rx: Mutex<Option<mpsc::UnboundedReceiver<RequeueId>>>,
        pending: Mutex<HashMap<u64, tokio::task::JoinHandle<()>>>,
        next: AtomicU64,
    }

    impl TokioRequeue {
        pub(crate) fn new() -> Self {
            let (tx, rx) = mpsc::unbounded_channel();
            Self {
                inner: Arc::new(TokioRequeueInner {
                    delivered_tx: tx,
                    delivered_rx: Mutex::new(Some(rx)),
                    pending: Mutex::new(HashMap::new()),
                    next: AtomicU64::new(1),
                }),
            }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        pub(crate) fn take_receiver(&self) -> mpsc::UnboundedReceiver<RequeueId> {
            self.inner
                .delivered_rx
                .lock()
                .take()
                .expect("receiver taken once")
        }
    }

    impl RequeueScheduler for TokioRequeue {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn schedule(&self, _key: ResourceKey, after: std::time::Duration) -> RequeueId {
            let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
            let tx = self.inner.delivered_tx.clone();
            let handle = tokio::spawn(async move {
                tokio::time::sleep(after).await;
                let _ = tx.send(RequeueId(id));
            });
            self.inner.pending.lock().insert(id, handle);
            RequeueId(id)
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn cancel(&self, id: RequeueId) {
            if let Some(handle) = self.inner.pending.lock().remove(&id.0) {
                handle.abort();
            }
        }
    }

    pub(crate) fn test_row(zone: &str, type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(zone, type_name, name),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"spec-envelope".to_vec(),
            metadata: Vec::new(),
            created_at: 1_725_000_000,
        }
    }

    pub(crate) struct Fixture {
        pub(crate) ctx: ResourceContext,
        pub(crate) effects: mpsc::UnboundedReceiver<super::EffectCompleted>,
        pub(crate) watch_notifications: mpsc::UnboundedReceiver<WatchSatisfied>,
    }

    pub(crate) fn fixture(
        row: StoredDesiredResource,
        manager: impl ManagerEndpoint + 'static,
        requeue: impl RequeueScheduler + 'static,
        decoder: Arc<dyn SpecDecoder>,
    ) -> Fixture {
        let (effects_tx, effects_rx) = mpsc::unbounded_channel();
        let (notify_tx, notify_rx) = mpsc::unbounded_channel();
        Fixture {
            ctx: ResourceContext::new(
                row,
                decoder,
                Arc::new(manager),
                Arc::new(requeue),
                effects_tx,
                notify_tx,
            ),
            effects: effects_rx,
            watch_notifications: notify_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;

    use super::test_support::{fixture, test_row, DeadManager, FailingDecoder, NullRequeue, TokioRequeue};
    use super::{
        ChildEnsure, LookupPlane, ManagerEndpoint, RequeueScheduler, RowLookup, WatchCondition,
        WatchId, WatchRegistration, WatchSatisfied, typed_spec_decoder,
    };
    use crate::error::ResourceError;
    use crate::identity::{ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource};
    use crate::manager::ResourceView;
    use crate::spec_store::EnsureOutcome;

    /// Test-local manager request enum carried by [`ChannelEndpointStub`].
    /// The production request surface this mirrors was deleted as zero-caller
    /// API (audit B6); the routing and failure-mapping assertions the
    /// in-module tests pinned against it survive against this stub, which
    /// keeps the request/reply shapes of the variants the tests exercise.
    #[derive(Debug)]
    enum StubCall {
        EnsureChild {
            parent: ResourceKey,
            child: ChildEnsure,
            reply: oneshot::Sender<Result<EnsureOutcome, ResourceError>>,
        },
        Get {
            key: ResourceKey,
            reply: oneshot::Sender<Result<Option<StoredDesiredResource>, ResourceError>>,
        },
        GetView {
            key: ResourceKey,
            reply: oneshot::Sender<Result<Option<ResourceView>, ResourceError>>,
        },
        RegisterWatch {
            registration: WatchRegistration,
            subscriber: ResourceKey,
            reply: oneshot::Sender<Result<WatchId, ResourceError>>,
        },
        CancelWatch {
            watch: WatchId,
            reply: oneshot::Sender<Result<(), ResourceError>>,
        },
    }

    /// Channel endpoint stub implementing [`ManagerEndpoint`] over
    /// [`StubCall`]: the deleted production channel endpoint's request/reply
    /// shape, kept in-module so the routing, ordering, and failure-mapping
    /// assertions of the tests below survive the deletion.
    struct ChannelEndpointStub {
        tx: mpsc::Sender<StubCall>,
    }

    impl ChannelEndpointStub {
        fn new(tx: mpsc::Sender<StubCall>) -> Self {
            Self { tx }
        }
    }

    #[async_trait]
    impl ManagerEndpoint for ChannelEndpointStub {
        async fn ensure_child(&self, parent: &ResourceKey, child: ChildEnsure) -> Result<EnsureOutcome, ResourceError> {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(StubCall::EnsureChild { parent: parent.clone(), child, reply })
                .await
                .map_err(|_| ResourceError::ManagerUnavailable("manager channel closed".into()))?;
            rx.await.map_err(|_| ResourceError::ManagerUnavailable("manager dropped the request".into()))?
        }

        async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(StubCall::Get { key: key.clone(), reply })
                .await
                .map_err(|_| ResourceError::ManagerUnavailable("manager channel closed".into()))?;
            rx.await.map_err(|_| ResourceError::ManagerUnavailable("manager dropped the request".into()))?
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(StubCall::GetView { key: key.clone(), reply })
                .await
                .map_err(|_| ResourceError::ManagerUnavailable("manager channel closed".into()))?;
            rx.await.map_err(|_| ResourceError::ManagerUnavailable("manager dropped the request".into()))?
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "delete not exercised in-module".into() })
        }

        async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "list_owned not exercised in-module".into() })
        }

        async fn register_watch(
            &self,
            subscriber: &ResourceKey,
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(StubCall::RegisterWatch {
                    registration,
                    subscriber: subscriber.clone(),
                    reply,
                })
                .await
                .map_err(|_| ResourceError::ManagerUnavailable("manager channel closed".into()))?;
            rx.await.map_err(|_| ResourceError::ManagerUnavailable("manager dropped the request".into()))?
        }

        async fn cancel_watch(&self, watch: WatchId) -> Result<(), ResourceError> {
            let (reply, rx) = oneshot::channel();
            self.tx
                .send(StubCall::CancelWatch { watch, reply })
                .await
                .map_err(|_| ResourceError::ManagerUnavailable("manager channel closed".into()))?;
            rx.await.map_err(|_| ResourceError::ManagerUnavailable("manager dropped the request".into()))?
        }
    }

    // -- Manager routing (R2; spec section 12) --------------------------------

    /// Persist-before-spawn at the context surface (F1, AE1): `ensure_child`
    /// routes exactly ONE EnsureChild request through the manager and the
    /// driver's future resolves only after the manager's commit ack. The
    /// manager-side ordering (commit BEFORE spawning the child actor, R7) is
    /// enforced in U3's handler; the exhaustive match in the stub pins that
    /// this surface cannot even express a spawn-shaped call.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_child_sends_one_persist_request_and_awaits_commit_ack() {
        let (tx, mut rx) = mpsc::channel::<StubCall>(4);
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let order_stub = order.clone();
        let calls_stub = calls.clone();
        let stub = tokio::spawn(async move {
            while let Some(call) = rx.recv().await {
                match call {
                    StubCall::EnsureChild { parent, child, reply } => {
                        assert_eq!(parent.type_name, "Volume");
                        assert_eq!(child.type_name, ResourceTypeName::new("Process"));
                        assert_eq!(child.name, "worker-0");
                        calls_stub.fetch_add(1, Ordering::SeqCst);
                        order_stub.lock().push("persist"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                        let row = test_row(&parent.zone, child.type_name.as_str(), &child.name);
                        let _ = reply.send(Ok(EnsureOutcome::Created(row)));
                    }
                    other => panic!("context sent a non-persist call: {other:?}"),
                }
            }
        });
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            ChannelEndpointStub::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let order_driver = order.clone();
        let driver_step = tokio::spawn(async move {
            let outcome = fixture
                .ctx
                .ensure_child(super::ChildEnsure {
                    type_name: ResourceTypeName::new("Process"),
                    name: "worker-0".into(),
                    spec: b"child-spec".to_vec(),
                    metadata: Vec::new(),
                })
                .await
                .expect("ensure_child");
            // The driver only proceeds past ensure once the commit ack
            // arrived.
            order_driver.lock().push("driver-after-commit-ack"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            outcome
        });

        let outcome = driver_step.await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Created(_)));
        stub.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "exactly one persist request");
        assert_eq!(
            *order.lock(), // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            vec!["persist", "driver-after-commit-ack"],
            "the commit is recorded before the driver proceeds"
        );
    }

    /// A closed manager channel or a dropped request surfaces as
    /// `ResourceError::ManagerUnavailable`, never as a silent no-op.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn manager_failures_surface_as_manager_unavailable() {
        // Channel closed before the call.
        let (tx, rx) = mpsc::channel::<StubCall>(1);
        drop(rx);
        let endpoint = ChannelEndpointStub::new(tx);
        let key = ResourceKey::new("z", "Volume", "data");
        let error = endpoint.get(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerUnavailable(_)));
        let error = endpoint.view(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerUnavailable(_)), "the live read fails loudly too");

        // Manager receives the request and drops it without replying.
        let (tx, mut rx) = mpsc::channel::<StubCall>(1);
        let endpoint = ChannelEndpointStub::new(tx);
        tokio::spawn(async move {
            let _ = rx.recv().await; // take the request, never reply
        });
        let error = endpoint.get(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerUnavailable(_)));
    }

    /// The live read (`ResourceContext::get_view`, `ManagerEndpoint::view`)
    /// never fabricates absence: a request the manager drops without
    /// replying surfaces as `ResourceError::ManagerUnavailable`.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn dropped_view_request_surfaces_as_manager_unavailable() {
        let (tx, mut rx) = mpsc::channel::<StubCall>(1);
        let endpoint = ChannelEndpointStub::new(tx);
        tokio::spawn(async move {
            let _ = rx.recv().await; // take the request, never reply
        });
        let key = ResourceKey::new("z", "Volume", "data");
        let error = endpoint.view(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerUnavailable(_)));
    }

    /// One classified read against a scripted manager: `respond` answers the
    /// single call the read makes, and the returned context runs the read.
    fn scripted_read(
        respond: impl FnOnce(StubCall) + Send + 'static,
    ) -> (super::ResourceContext, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<StubCall>(1);
        let stub = tokio::spawn(async move {
            match rx.recv().await {
                Some(call) => respond(call),
                None => panic!("classified read sent no call"),
            }
        });
        let harness = fixture(
            test_row("z", "Volume", "data"),
            ChannelEndpointStub::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        (harness.ctx, stub)
    }

    /// The classified read surface over a scripted manager: `Present` carries
    /// the row, a manager that holds no row answers `Absent`, and a manager
    /// that cannot answer reports `Unavailable` - never absence.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn classified_lookup_maps_present_absent_and_unanswerable_manager() {
        let key = ResourceKey::new("z", "Volume", "data");
        let row = test_row("z", "Volume", "data");

        // The manager answers with the row.
        let answer = row.clone();
        let (mut ctx, stub) = scripted_read(move |call| match call {
            StubCall::Get { reply, .. } => {
                let _ = reply.send(Ok(Some(answer)));
            }
            other => panic!("classified lookup sent a non-Get call: {other:?}"),
        });
        assert_eq!(
            ctx.lookup(&key).await,
            RowLookup::Present {
                row,
                plane: LookupPlane::Manager,
            },
        );
        stub.await.unwrap();

        // The manager answers that it holds no row: absence, not failure.
        let (mut ctx, stub) = scripted_read(|call| match call {
            StubCall::Get { reply, .. } => {
                let _ = reply.send(Ok(None));
            }
            other => panic!("classified lookup sent a non-Get call: {other:?}"),
        });
        assert_eq!(
            ctx.lookup(&key).await,
            RowLookup::Absent {
                plane: LookupPlane::Manager,
            },
        );
        stub.await.unwrap();

        // The manager cannot answer: unavailable, never absence.
        let (mut ctx, stub) = scripted_read(|call| match call {
            StubCall::Get { reply, .. } => {
                let _ = reply.send(Err(ResourceError::ManagerUnavailable("no answer".into())));
            }
            other => panic!("classified lookup sent a non-Get call: {other:?}"),
        });
        assert_eq!(
            ctx.lookup(&key).await,
            RowLookup::Unavailable {
                plane: LookupPlane::Manager,
            },
        );
        stub.await.unwrap();

        // The view read classifies the same shapes.
        let view = ResourceView {
            key: key.clone(),
            uid: [0x42; 16],
            generation: 3,
            deleting: false,
            provenance: ResourceProvenance::Api,
            spec: b"spec-envelope".to_vec(),
            metadata: Vec::new(),
            owner_key: None,
            status: None,
            status_generation: None,
            status_projection: None,
        };
        let answer = view.clone();
        let view_key = key.clone();
        let (mut ctx, stub) = scripted_read(move |call| match call {
            StubCall::GetView { key, reply } => {
                assert_eq!(key, view_key, "the view read targets the requested key");
                let _ = reply.send(Ok(Some(answer)));
            }
            other => panic!("classified view lookup sent a non-GetView call: {other:?}"),
        });
        assert_eq!(
            ctx.lookup_view(&key).await,
            RowLookup::Present {
                row: view,
                plane: LookupPlane::Manager,
            },
        );
        stub.await.unwrap();
    }

    // -- Requeue (R13; spec section 32) ---------------------------------------

    /// One `requeue_after` call schedules exactly one reconcile after the
    /// delay - not zero, not two. The production timer is a ractor timer
    /// inside ResourceActor (U3); this proves the contract shapes with a
    /// tokio-time scheduler under paused time. Retry state stays runtime-only
    /// (R13): nothing here is persisted.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn requeue_after_delivers_exactly_one_reconcile_after_the_delay() {
        let requeue = TokioRequeue::new();
        let mut delivered = requeue.take_receiver();
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            DeadManager,
            requeue,
            Arc::new(FailingDecoder),
        );

        let id = fixture.ctx.requeue_after(Duration::from_secs(60));
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(59)).await;
        tokio::task::yield_now().await;
        assert!(delivered.try_recv().is_err(), "no reconcile before the delay");
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(delivered.try_recv().ok(), Some(id), "exactly one reconcile at the delay");
        tokio::time::advance(Duration::from_secs(3600)).await;
        tokio::task::yield_now().await;
        assert!(delivered.try_recv().is_err(), "exactly one reconcile total");
    }

    /// The delete path cancels pending requeues (the actor cancels on delete;
    /// U3 owns the wiring): cancellation suppresses exactly the cancelled
    /// schedule and leaves other pending schedules intact.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn concurrent_delete_cancels_only_the_cancelled_requeue() {
        let requeue = TokioRequeue::new();
        let mut delivered = requeue.take_receiver();
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            DeadManager,
            requeue.clone(),
            Arc::new(FailingDecoder),
        );

        let doomed = fixture.ctx.requeue_after(Duration::from_secs(60));
        let kept = fixture.ctx.requeue_after(Duration::from_secs(60));
        tokio::task::yield_now().await;

        // Simulated actor delete path: cancel the doomed schedule.
        requeue.cancel(doomed);

        tokio::time::advance(Duration::from_secs(120)).await;
        tokio::task::yield_now().await;
        assert_eq!(delivered.try_recv().ok(), Some(kept), "the kept schedule still fires");
        assert!(delivered.try_recv().is_err(), "the cancelled schedule never fires");
    }

    // -- Typed spec decode ----------------------------------------------------

    /// `spec<T>()` decodes the stored envelope through the wired hook once,
    /// caches the decoded value, downcasts on access, and reports decode
    /// failures and type mismatches as `ResourceError::SpecDecode`.
    #[test]
    fn spec_typed_decode_caches_and_reports_mismatch_and_failure() {
        #[derive(Debug, PartialEq)]
        struct VolumeSpec {
            mount: String,
        }
        #[derive(Debug, thiserror::Error)]
        #[error("bad envelope")]
        struct BadEnvelope;

        let decode_calls = Arc::new(AtomicUsize::new(0));
        let calls = decode_calls.clone();
        let decoder = typed_spec_decoder(move |bytes: &[u8]| -> Result<VolumeSpec, BadEnvelope> {
            calls.fetch_add(1, Ordering::SeqCst);
            if bytes == b"spec-envelope" {
                Ok(VolumeSpec { mount: "/mnt/data".into() })
            } else {
                Err(BadEnvelope)
            }
        });

        let good = fixture(
            test_row("z", "Volume", "data"),
            DeadManager,
            NullRequeue,
            decoder.clone(),
        );
        let spec: &VolumeSpec = good.ctx.spec().expect("typed decode");
        assert_eq!(spec.mount, "/mnt/data");
        // Cached: a second access does not decode again.
        let spec: &VolumeSpec = good.ctx.spec().expect("typed decode again");
        assert_eq!(spec.mount, "/mnt/data");
        assert_eq!(decode_calls.load(Ordering::SeqCst), 1, "decode ran exactly once");

        // Requested type does not match the decoded one.
        let error = good.ctx.spec::<String>().unwrap_err();
        assert!(matches!(error, ResourceError::SpecDecode { .. }));

        // The decode hook itself fails.
        let fixture_bad = fixture(
            test_row("z", "Volume", "data"),
            DeadManager,
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let error = fixture_bad.ctx.spec::<VolumeSpec>().unwrap_err();
        match error {
            ResourceError::SpecDecode { key, source } => {
                assert_eq!(key, ResourceKey::new("z", "Volume", "data"));
                assert_eq!(source.to_string(), "decode failed");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    // -- Internal watch shapes (R12; spec section 15) -------------------------

    /// AE2 at unit scale (R12): evaluate-and-register must be ONE handler on
    /// the target actor's mailbox, so a condition that flips while the
    /// registration message is still queued notifies exactly once, and a
    /// registration whose condition already holds notifies immediately. U3
    /// enforces this inside ResourceActor's mailbox; this test pins the
    /// shapes and the pattern that make the invariant expressible.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn internal_watch_evaluate_and_register_share_one_mailbox_handler() {
        enum TargetMsg {
            BecomeReady,
            Register(WatchRegistration),
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<TargetMsg>();
        let target = ResourceKey::new("z", "Process", "worker-0");
        let actor_target = target.clone();
        let actor = tokio::spawn(async move {
            let mut ready = false;
            let mut next_watch = WatchId(1);
            let mut watchers: Vec<(WatchId, WatchCondition, mpsc::UnboundedSender<WatchSatisfied>)> =
                Vec::new();
            while let Some(msg) = rx.recv().await {
                match msg {
                    TargetMsg::BecomeReady => {
                        ready = true;
                        // Status transition evaluates registered watches in
                        // the same handler.
                        watchers.retain(|(watch, condition, notify)| {
                            if matches!(condition, WatchCondition::Ready) {
                                let _ = notify.send(WatchSatisfied {
                                    watch: *watch,
                                    target: actor_target.clone(),
                                });
                                false
                            } else {
                                true
                            }
                        });
                    }
                    TargetMsg::Register(registration) => {
                        // Evaluate-and-register in ONE handler (spec section
                        // 15): the lost-wakeup race cannot exist.
                        if matches!(registration.condition, WatchCondition::Ready) && ready {
                            let _ = registration.notify.send(WatchSatisfied {
                                watch: next_watch,
                                target: registration.target.clone(),
                            });
                        } else {
                            watchers.push((next_watch, registration.condition, registration.notify));
                        }
                        next_watch = WatchId(next_watch.0 + 1);
                    }
                }
            }
        });

        // A dependent registers while the condition is false.
        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel();
        tx.send(TargetMsg::Register(WatchRegistration {
            target: target.clone(),
            condition: WatchCondition::Ready,
            notify: notify_tx.clone(),
        }))
        .unwrap();
        tokio::task::yield_now().await;
        assert!(notify_rx.try_recv().is_err(), "condition false: no notification yet");

        // The condition flips while a further registration is queued...
        tx.send(TargetMsg::BecomeReady).unwrap();
        // ...and one more registration arrives after the flip (its own
        // subscriber channel).
        let (notify2_tx, mut notify2_rx) = mpsc::unbounded_channel();
        tx.send(TargetMsg::Register(WatchRegistration {
            target: target.clone(),
            condition: WatchCondition::Ready,
            notify: notify2_tx,
        }))
        .unwrap();
        drop(tx);
        actor.await.unwrap();

        // The queued watcher was satisfied exactly once by the transition...
        assert_eq!(notify_rx.try_recv().ok().map(|n| n.target), Some(target.clone()));
        assert!(notify_rx.try_recv().is_err(), "exactly one notification");
        // ...and the post-flip registration was satisfied immediately (the
        // same evaluate-or-register handler covers both orders).
        assert_eq!(notify2_rx.try_recv().ok().map(|n| n.target), Some(target.clone()));
        assert!(notify2_rx.try_recv().is_err(), "exactly one immediate notification");
    }

    /// `ctx.watch()` routes the registration through the manager with this
    /// resource's key as the subscriber, and satisfaction arrives on this
    /// resource's notify channel.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn watch_registration_routes_through_the_manager_with_subscriber_identity() {
        let (tx, mut rx) = mpsc::channel::<StubCall>(1);
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            ChannelEndpointStub::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let subscriber = ResourceKey::new("z", "Volume", "data");
        let watched = ResourceKey::new("z", "Process", "worker-0");
        let stub_target = watched.clone();
        let stub = tokio::spawn(async move {
            if let Some(StubCall::RegisterWatch { registration, subscriber: from, reply }) =
                rx.recv().await
            {
                assert_eq!(from, subscriber, "the context reports its own key as subscriber");
                assert_eq!(registration.target, stub_target);
                // The (stub) target actor satisfies the condition immediately.
                let _ = registration.notify.send(WatchSatisfied {
                    watch: WatchId(5),
                    target: registration.target.clone(),
                });
                let _ = reply.send(Ok(WatchId(5)));
            }
        });

        let watch_id = fixture.ctx.watch(watched, WatchCondition::Ready).await.unwrap();
        assert_eq!(watch_id, WatchId(5));
        let satisfied = fixture.watch_notifications.recv().await.unwrap();
        assert_eq!(satisfied.watch, WatchId(5));
        assert_eq!(satisfied.target, ResourceKey::new("z", "Process", "worker-0"));
        stub.await.unwrap();
    }

    /// The registration is live from the moment it is placed until the target
    /// speaks on it or this row releases it, and nothing else moves it.
    ///
    /// This is the seam a re-arm reads: evidence that reads back identical
    /// cannot tell a spent registration from a standing one, because a target
    /// pass satisfies the registration and republishes the very projection it
    /// published before. A release the manager REFUSES leaves the id live,
    /// because the registration is still in the target's mailbox.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_registration_is_live_until_the_target_notifies_or_this_row_releases_it() {
        let (tx, mut rx) = mpsc::channel::<StubCall>(4);
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            ChannelEndpointStub::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let watched = ResourceKey::new("z", "Process", "worker-0");
        let refused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let refused_stub = refused.clone();
        let stub = tokio::spawn(async move {
            while let Some(call) = rx.recv().await {
                match call {
                    StubCall::RegisterWatch { reply, .. } => {
                        let _ = reply.send(Ok(WatchId(7)));
                    }
                    StubCall::CancelWatch { watch, reply } => {
                        if refused_stub.load(Ordering::SeqCst) {
                            let _ = reply.send(Err(ResourceError::ManagerRejected {
                                reason: "release refused".into(),
                            }));
                        } else {
                            let _ = reply.send(Ok(()));
                            assert_eq!(watch, WatchId(7));
                        }
                    }
                    other => panic!("unexpected call: {other:?}"),
                }
            }
        });

        let armed = fixture
            .ctx
            .watch(watched.clone(), WatchCondition::ProjectionChanged)
            .await
            .expect("the registration lands in the target's mailbox");
        assert!(fixture.ctx.watch_is_live(armed), "a placed registration is standing");

        fixture.ctx.mark_watch_spent(armed);
        assert!(
            !fixture.ctx.watch_is_live(armed),
            "the target spoke: the registration is spent and the driver re-arms it"
        );

        let rearmed = fixture
            .ctx
            .watch(watched.clone(), WatchCondition::ProjectionChanged)
            .await
            .expect("the spent registration is replaced");
        assert!(fixture.ctx.watch_is_live(rearmed));

        refused.store(true, Ordering::SeqCst);
        assert!(
            fixture.ctx.cancel_watch(rearmed).await.is_err(),
            "a refused release is reported as such"
        );
        assert!(
            fixture.ctx.watch_is_live(rearmed),
            "a refused release leaves the registration standing in the target"
        );
        stub.abort();
    }

    // -- Service driver context (U3, R7) --------------------------------------

    /// The service driver context routes reads through the injected manager
    /// endpoint and classifies them ([`RowLookup`]): a present row, an
    /// absent row, and an unanswered manager stay distinct, exactly as the
    /// driver context's own reads do.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn service_resource_context_reads_through_the_manager_endpoint() {
        let (tx, mut rx) = mpsc::channel::<StubCall>(4);
        let stub = tokio::spawn(async move {
            while let Some(call) = rx.recv().await {
                match call {
                    StubCall::Get { key, reply } => {
                        if key.name == "present" {
                            let _ = reply.send(Ok(Some(test_row(&key.zone, "Process", &key.name))));
                        } else {
                            let _ = reply.send(Ok(None));
                        }
                    }
                    StubCall::GetView { key, reply } => {
                        if key.name == "present" {
                            let _ = reply.send(Ok(Some(ResourceView {
                                key,
                                uid: [1; 16],
                                generation: 1,
                                deleting: false,
                                provenance: ResourceProvenance::Api,
                                spec: Vec::new(),
                                metadata: Vec::new(),
                                owner_key: None,
                                status: None,
                                status_generation: None,
                                status_projection: None,
                            })));
                        } else {
                            let _ = reply.send(Ok(None));
                        }
                    }
                    other => panic!("service context sent a non-read call: {other:?}"),
                }
            }
        });
        let context = super::ServiceResourceContext::over(Arc::new(ChannelEndpointStub::new(tx)));

        let present_key = ResourceKey::new("z", "Process", "present");
        let absent_key = ResourceKey::new("z", "Process", "absent");
        let row = context.get(&present_key).await.expect("read").expect("row");
        assert_eq!(row.key, present_key);
        assert!(matches!(
            context.lookup(&absent_key).await,
            RowLookup::Absent { plane: LookupPlane::Manager }
        ));
        let view = context.view(&present_key).await.expect("read").expect("view");
        assert_eq!(view.key, present_key);
        assert!(matches!(
            context.lookup_view(&absent_key).await,
            RowLookup::Absent { plane: LookupPlane::Manager }
        ));
        // The context still holds the channel sender; drop it so the stub's
        // receive loop sees the channel close and exits.
        drop(context);
        stub.await.unwrap();
    }

    /// The fail-closed service context refuses every read: an unwired
    /// composition seam classifies as `Unavailable`, never as absence.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn the_fail_closed_service_context_refuses_every_read() {
        let context = super::ServiceResourceContext::fail_closed();
        let key = ResourceKey::new("z", "Process", "worker-0");
        assert!(matches!(
            context.lookup(&key).await,
            RowLookup::Unavailable { plane: LookupPlane::Manager }
        ));
        assert!(matches!(
            context.lookup_view(&key).await,
            RowLookup::Unavailable { plane: LookupPlane::Manager }
        ));
        assert!(context.get(&key).await.is_err());
        assert!(context.view(&key).await.is_err());
    }
}