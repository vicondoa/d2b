//! ResourceContext - the capability surface handed to drivers (U4, KTD3;
//! spec sections 12, 14, 15).

pub const MODULE_NAME: &str = "context";
use std::any::Any;
use std::cell::OnceCell;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::error::{FailureComparison, ResourceError};
use crate::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use crate::manager::ResourceView;
use crate::spec_store::EnsureOutcome;
use crate::target::TargetHandle;

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
    /// Named custom predicate; the target actor's driver supplies the
    /// predicate implementation by id.
    ///
    /// Not implemented: the target actor evaluates every custom predicate as
    /// unsatisfied, so readiness is expressed with [`Self::Ready`] until the
    /// driver-supplied predicate hook lands (U6+).
    Custom(String),
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
/// if status matches condition { notify(subscriber, Satisfied) }
/// else { watchers.insert(watch_id, ...) }
/// ```
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

/// Messages a driver's context sends to the owning manager actor (R2:
/// drivers never touch the spec store; the manager is the only writer).
/// Every durable mutation is request/reply, and the reply fires only after
/// the manager's commit - drivers observe persist-before-spawn by
/// construction (F1, AE1).
///
/// There is deliberately NO spawn-shaped call on this surface: child actor
/// creation is the manager's commit-then-spawn handler (U3).
#[derive(Debug)]
pub enum ManagerCall {
    EnsureChild {
        parent: ResourceKey,
        child: ChildEnsure,
        reply: oneshot::Sender<Result<EnsureOutcome, ResourceError>>,
    },
    Get {
        key: ResourceKey,
        reply: oneshot::Sender<Result<Option<StoredDesiredResource>, ResourceError>>,
    },
    /// Live runtime view (row plus published status) for one resource
    /// (`ResourceContext::get_view`).
    GetView {
        key: ResourceKey,
        reply: oneshot::Sender<Result<Option<ResourceView>, ResourceError>>,
    },
    Delete {
        key: ResourceKey,
        reply: oneshot::Sender<Result<(), ResourceError>>,
    },
    ListOwned {
        owner_uid: [u8; 16],
        reply: oneshot::Sender<Result<Vec<StoredDesiredResource>, ResourceError>>,
    },
    RegisterWatch {
        /// The subscription: target, condition, and the subscriber's notify
        /// sender. The manager routes it to the target actor and records the
        /// dependency edge from `subscriber` (spec section 16).
        registration: WatchRegistration,
        subscriber: ResourceKey,
        reply: oneshot::Sender<Result<WatchId, ResourceError>>,
    },
    CancelWatch {
        watch: WatchId,
        reply: oneshot::Sender<Result<(), ResourceError>>,
    },
}

/// The injected manager endpoint behind a [`ResourceContext`] (KTD3): every
/// durable child mutation and every internal-watch registration rides this
/// surface, never the spec store. U3's manager implements it directly or
/// over its mailbox (see [`ChannelManagerEndpoint`]).
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
}

/// Endpoint over a manager request channel. The manager actor consumes the
/// [`ManagerCall`] stream (or wraps it into its own mailbox messages).
pub struct ChannelManagerEndpoint {
    tx: mpsc::Sender<ManagerCall>,
}

impl ChannelManagerEndpoint {
    pub fn new(tx: mpsc::Sender<ManagerCall>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ManagerEndpoint for ChannelManagerEndpoint {
    async fn ensure_child(&self, parent: &ResourceKey, child: ChildEnsure) -> Result<EnsureOutcome, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::EnsureChild { parent: parent.clone(), child, reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::Get { key: key.clone(), reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::GetView { key: key.clone(), reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::Delete { key: key.clone(), reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn list_owned(&self, owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::ListOwned { owner_uid, reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn register_watch(
        &self,
        subscriber: &ResourceKey,
        registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::RegisterWatch {
                registration,
                subscriber: subscriber.clone(),
                reply,
            })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    async fn cancel_watch(&self, watch: WatchId) -> Result<(), ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ManagerCall::CancelWatch { watch, reply })
            .await
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
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
    /// The pre-v3 durable store plane: rows no actor serves (yet), read
    /// through the store handle.
    Store,
}

impl LookupPlane {
    /// The stable label both sides of a comparison render.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manager => "manager",
            Self::Store => "store",
        }
    }
}

impl std::fmt::Display for LookupPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The default disposition of one [`RowLookup`] under issue #511's rule
/// "defer unless proven terminal".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupDisposition {
    /// The row was present; the caller proceeds with it.
    Proceed,
    /// The caller defers and requeues: a retryable failure (R13) - the actor
    /// schedules exactly one reconcile after its backoff and publishes no
    /// terminal status.
    Defer,
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
/// [`RowLookup::disposition`] returns that default. A call site that fails
/// terminal must first name its terminal evidence - the committed row that
/// cannot decode, the structurally invalid spec - and surface it (status
/// projection or log), so a terminal status is always evidence-backed and
/// never inferred from absence or from an unanswered plane.
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
    /// The plane this lookup was answered from.
    pub const fn plane(&self) -> LookupPlane {
        match self {
            Self::Present { plane, .. }
            | Self::Absent { plane }
            | Self::Unavailable { plane }
            | Self::Error { plane, .. } => *plane,
        }
    }

    /// The rule's default disposition (issue #511): `Present` proceeds;
    /// `Absent`, `Unavailable`, and `Error` all defer with a requeue.
    /// Terminal is never a default of this classification.
    pub const fn disposition(&self) -> LookupDisposition {
        match self {
            Self::Present { .. } => LookupDisposition::Proceed,
            Self::Absent { .. } | Self::Unavailable { .. } | Self::Error { .. } => {
                LookupDisposition::Defer
            }
        }
    }

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
    target: TargetHandle,
    /// Directory-backed target binding (U13): the guest handle, the session
    /// generation it is bound to, and the directory every guest operation
    /// re-validates through. `None` when the manager runs without a target
    /// directory (scaffold and unit fixtures).
    target_binding: Option<crate::target::TargetBinding>,
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
}

impl ResourceContext {
    /// Assembled by the resource actor (U3) from the stored row, the
    /// resource's target, and the manager-wired hooks.
    pub fn new(
        row: StoredDesiredResource,
        target: TargetHandle,
        decoder: Arc<dyn SpecDecoder>,
        manager: Arc<dyn ManagerEndpoint>,
        requeue: Arc<dyn RequeueScheduler>,
        effects: mpsc::UnboundedSender<EffectCompleted>,
        watch_notify: mpsc::UnboundedSender<WatchSatisfied>,
    ) -> Self {
        Self {
            row,
            target,
            target_binding: None,
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
        }
    }

    /// Attach the owning resource's key (manager-resolved).
    pub fn with_owner_key(mut self, owner_key: Option<crate::identity::ResourceKey>) -> Self {
        self.owner_key = owner_key;
        self
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

    /// Execution target of this resource (R19).
    pub fn target(&self) -> &TargetHandle {
        &self.target
    }

    /// Attach the resolved target binding (U13, manager-wired). Drivers that
    /// realize on a guest target work through it: every operation re-reads
    /// the live session instead of trusting a channel the driver kept.
    pub fn with_target_binding(mut self, binding: crate::target::TargetBinding) -> Self {
        self.target_binding = Some(binding);
        self
    }

    /// The resolved target binding, when the manager wired one.
    pub fn target_binding(&self) -> Option<&crate::target::TargetBinding> {
        self.target_binding.as_ref()
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
    /// `Unavailable` - never absence.
    ///
    /// Apply [`RowLookup::disposition`] before choosing an outcome: the
    /// default is to defer and requeue unless the call site has named
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
    /// - `Err(ResourceError::ManagerRpc(_))`: the manager could not answer.
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
    /// answer (`Unavailable`). Apply [`RowLookup::disposition`] before
    /// choosing an outcome.
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
        self.manager
            .register_watch(
                &self.row.key,
                WatchRegistration {
                    target,
                    condition,
                    notify: self.watch_notify.clone(),
                },
            )
            .await
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
    use crate::target::TargetHandle;

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
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn get(&self, _key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRpc("dead manager".into()))
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

        pub(crate) fn take_receiver(&self) -> mpsc::UnboundedReceiver<RequeueId> {
            self.inner
                .delivered_rx
                .lock()
                .take()
                .expect("receiver taken once")
        }
    }

    impl RequeueScheduler for TokioRequeue {
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
                TargetHandle::Host,
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

    use parking_lot::Mutex;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;

    use super::test_support::{fixture, test_row, DeadManager, FailingDecoder, NullRequeue, TokioRequeue};
    use super::{
        LookupDisposition, LookupPlane, ManagerEndpoint, RequeueScheduler, RowLookup,
        WatchCondition, WatchId, WatchRegistration, WatchSatisfied, typed_spec_decoder,
    };
    use crate::error::ResourceError;
    use crate::identity::{ResourceKey, ResourceProvenance, ResourceTypeName};
    use crate::manager::ResourceView;
    use crate::spec_store::EnsureOutcome;

    // -- Manager routing (R2; spec section 12) --------------------------------

    /// Persist-before-spawn at the context surface (F1, AE1): `ensure_child`
    /// routes exactly ONE EnsureChild request through the manager and the
    /// driver's future resolves only after the manager's commit ack. The
    /// manager-side ordering (commit BEFORE spawning the child actor, R7) is
    /// enforced in U3's handler; the exhaustive match in the stub pins that
    /// this surface cannot even express a spawn-shaped call.
    #[tokio::test]
    async fn ensure_child_sends_one_persist_request_and_awaits_commit_ack() {
        let (tx, mut rx) = mpsc::channel::<super::ManagerCall>(4);
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let order_stub = order.clone();
        let calls_stub = calls.clone();
        let stub = tokio::spawn(async move {
            while let Some(call) = rx.recv().await {
                match call {
                    super::ManagerCall::EnsureChild { parent, child, reply } => {
                        assert_eq!(parent.type_name, "Volume");
                        assert_eq!(child.type_name, ResourceTypeName::new("Process"));
                        assert_eq!(child.name, "worker-0");
                        calls_stub.fetch_add(1, Ordering::SeqCst);
                        order_stub.lock().push("persist");
                        let row = test_row(&parent.zone, child.type_name.as_str(), &child.name);
                        let _ = reply.send(Ok(EnsureOutcome::Created(row)));
                    }
                    other => panic!("context sent a non-persist call: {other:?}"),
                }
            }
        });
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            super::ChannelManagerEndpoint::new(tx),
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
            order_driver.lock().push("driver-after-commit-ack");
            outcome
        });

        let outcome = driver_step.await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Created(_)));
        stub.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "exactly one persist request");
        assert_eq!(
            *order.lock(),
            vec!["persist", "driver-after-commit-ack"],
            "the commit is recorded before the driver proceeds"
        );
    }

    /// A closed manager channel or a dropped request surfaces as
    /// `ResourceError::ManagerRpc`, never as a silent no-op.
    #[tokio::test]
    async fn manager_failures_surface_as_manager_rpc_errors() {
        // Channel closed before the call.
        let (tx, rx) = mpsc::channel::<super::ManagerCall>(1);
        drop(rx);
        let endpoint = super::ChannelManagerEndpoint::new(tx);
        let key = ResourceKey::new("z", "Volume", "data");
        let error = endpoint.get(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerRpc(_)));
        let error = endpoint.view(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerRpc(_)), "the live read fails loudly too");

        // Manager receives the request and drops it without replying.
        let (tx, mut rx) = mpsc::channel::<super::ManagerCall>(1);
        let endpoint = super::ChannelManagerEndpoint::new(tx);
        tokio::spawn(async move {
            let _ = rx.recv().await; // take the request, never reply
        });
        let error = endpoint.get(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerRpc(_)));
    }

    /// The live read (`ResourceContext::get_view`, `ManagerEndpoint::view`)
    /// never fabricates absence: a request the manager drops without
    /// replying surfaces as `ResourceError::ManagerRpc`.
    #[tokio::test]
    async fn dropped_view_request_surfaces_as_manager_rpc_error() {
        let (tx, mut rx) = mpsc::channel::<super::ManagerCall>(1);
        let endpoint = super::ChannelManagerEndpoint::new(tx);
        tokio::spawn(async move {
            let _ = rx.recv().await; // take the request, never reply
        });
        let key = ResourceKey::new("z", "Volume", "data");
        let error = endpoint.view(&key).await.unwrap_err();
        assert!(matches!(error, ResourceError::ManagerRpc(_)));
    }

    // -- Lookup classification (issue #511) -----------------------------------

    /// Issue #511's default is "defer unless proven terminal": across all
    /// four classifications only `Present` proceeds, and every classification
    /// records the plane it was answered from.
    #[test]
    fn every_non_present_lookup_defers_by_default() {
        let present: RowLookup<u8> = RowLookup::Present {
            row: 7,
            plane: LookupPlane::Manager,
        };
        assert_eq!(present.disposition(), LookupDisposition::Proceed);
        assert_eq!(present.plane(), LookupPlane::Manager);

        let absent: RowLookup<u8> = RowLookup::Absent {
            plane: LookupPlane::Manager,
        };
        assert_eq!(absent.disposition(), LookupDisposition::Defer, "absence is not failure");
        assert_eq!(absent.plane(), LookupPlane::Manager);

        let unavailable: RowLookup<u8> = RowLookup::Unavailable {
            plane: LookupPlane::Store,
        };
        assert_eq!(
            unavailable.disposition(),
            LookupDisposition::Defer,
            "an unanswered plane is retryable"
        );
        assert_eq!(unavailable.plane(), LookupPlane::Store);

        let error: RowLookup<u8> = RowLookup::Error {
            plane: LookupPlane::Store,
            detail: "row does not decode".to_owned(),
        };
        assert_eq!(
            error.disposition(),
            LookupDisposition::Defer,
            "a failed read is not terminal by itself"
        );
        assert_eq!(error.plane(), LookupPlane::Store);
    }

    /// One classified read against a scripted manager: `respond` answers the
    /// single call the read makes, and the returned context runs the read.
    fn scripted_read(
        respond: impl FnOnce(super::ManagerCall) + Send + 'static,
    ) -> (super::ResourceContext, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<super::ManagerCall>(1);
        let stub = tokio::spawn(async move {
            match rx.recv().await {
                Some(call) => respond(call),
                None => panic!("classified read sent no call"),
            }
        });
        let harness = fixture(
            test_row("z", "Volume", "data"),
            super::ChannelManagerEndpoint::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        (harness.ctx, stub)
    }

    /// The classified read surface over a scripted manager: `Present` carries
    /// the row, a manager that holds no row answers `Absent`, and a manager
    /// that cannot answer reports `Unavailable` - never absence.
    #[tokio::test]
    async fn classified_lookup_maps_present_absent_and_unanswerable_manager() {
        let key = ResourceKey::new("z", "Volume", "data");
        let row = test_row("z", "Volume", "data");

        // The manager answers with the row.
        let answer = row.clone();
        let (mut ctx, stub) = scripted_read(move |call| match call {
            super::ManagerCall::Get { reply, .. } => {
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
            super::ManagerCall::Get { reply, .. } => {
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
            super::ManagerCall::Get { reply, .. } => {
                let _ = reply.send(Err(ResourceError::ManagerRpc("no answer".into())));
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
        let (mut ctx, stub) = scripted_read(move |call| match call {
            super::ManagerCall::GetView { reply, .. } => {
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

    /// The watch and manager shapes cross channels intact: registrations and
    /// calls are Send and round-trip field-exact.
    #[tokio::test]
    async fn watch_and_manager_shapes_send_and_receive_intact() {
        // WatchRegistration through a channel.
        let (wtx, mut wrx) = mpsc::unbounded_channel::<WatchRegistration>();
        let (notify_tx, mut notify_back) = mpsc::unbounded_channel::<WatchSatisfied>();
        let target = ResourceKey::new("z", "Process", "worker-0");
        wtx.send(WatchRegistration {
            target: target.clone(),
            condition: WatchCondition::Custom("gpu-free".into()),
            notify: notify_tx,
        })
        .unwrap();
        let registration = wrx.recv().await.unwrap();
        assert_eq!(registration.target, target);
        assert_eq!(registration.condition, WatchCondition::Custom("gpu-free".into()));

        // ManagerCall request/reply round-trip.
        let (mtx, mut mrx) = mpsc::channel::<super::ManagerCall>(1);
        let (reply_tx, reply_rx) = oneshot::channel();
        mtx.send(super::ManagerCall::Get { key: target.clone(), reply: reply_tx })
            .await
            .unwrap();
        match mrx.recv().await.unwrap() {
            super::ManagerCall::Get { key, reply } => {
                assert_eq!(key, target);
                let _ = reply.send(Ok(None));
            }
            other => panic!("unexpected call: {other:?}"),
        }
        assert!(matches!(reply_rx.await.unwrap(), Ok(None)));

        // The satisfied notification flows back to the subscriber.
        registration
            .notify
            .send(WatchSatisfied { watch: WatchId(7), target: target.clone() })
            .unwrap();
        let satisfied = notify_back.recv().await.unwrap();
        assert_eq!(satisfied.watch, WatchId(7));
        assert_eq!(satisfied.target, target);
    }

    /// `ctx.watch()` routes the registration through the manager with this
    /// resource's key as the subscriber, and satisfaction arrives on this
    /// resource's notify channel.
    #[tokio::test]
    async fn watch_registration_routes_through_the_manager_with_subscriber_identity() {
        let (tx, mut rx) = mpsc::channel::<super::ManagerCall>(1);
        let mut fixture = fixture(
            test_row("z", "Volume", "data"),
            super::ChannelManagerEndpoint::new(tx),
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let subscriber = ResourceKey::new("z", "Volume", "data");
        let watched = ResourceKey::new("z", "Process", "worker-0");
        let stub_target = watched.clone();
        let stub = tokio::spawn(async move {
            if let Some(super::ManagerCall::RegisterWatch { registration, subscriber: from, reply }) =
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
}