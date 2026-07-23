//! `d2b-realm-router` (ADR 0032): the **codec-neutral**
//! operation router. It reasons only over the semantic
//! [`OperationRequest`] envelope (never a wire encoding), so it owns the
//! three cross-cutting invariants:
//!
//! - **Principal binding**: the request principal MUST match the
//!   authenticated session principal.
//! - **Required-capability derivation**: the capability/scope is derived
//!   from [`OperationKind`] in trusted code, never from a caller-supplied
//!   field.
//! - **Idempotency + dedup ownership**: the router is the single dedup
//!   owner for its scope. Per ADR 0032 the dedup record is keyed by the
//!   full operation namespace — `(realm, principal, node, operation kind,
//!   idempotency key)` — NOT the idempotency key alone, so the same opaque
//!   caller key reused under a different principal/realm/node/kind can never
//!   collide. A mutating operation MUST carry an idempotency key;
//!   same-key/same-request is a replay (carrying the original
//!   `operation_id` + recorded result), same-key/different-request is a
//!   conflict, and a key reused after the dedup retention window is expired.
//!   Expired keys leave a tombstone for a longer no-reuse horizon so a
//!   post-retention reuse fails closed (`IdempotencyKeyExpired`) instead of
//!   being silently re-executed.
//!
//! **Single-owner / shared scope.** The router is the dedup owner for the
//! node/gateway scope it is constructed at, NOT a per-session object. A peer
//! session binds a *shared* router (e.g. behind `Arc<Mutex<_>>`) so reconnect
//! retries on a fresh session still hit the same dedup state — a fresh
//! per-session router would let reconnect retries bypass dedup and
//! double-dispatch. See `d2bd`'s `PeerOperationRouter` for the wiring.
//!
//! Dependency direction: depends ONLY on `d2b-realm-core` +
//! `d2b-realm-provider`. It MUST NOT depend on `prost`, a codec
//! crate, a transport impl, or any host-only internals (enforced by the
//! constellation dependency-direction CI gate).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use d2b_realm_core::{
    AuthorizationScope, Capability, CapabilitySet, ConstellationError, ErrorKind, IdempotencyKey,
    NodeId, OpaquePayload, OperationId, OperationKind, OperationRequest, PrincipalId, RealmPath,
    StreamKind,
};

pub mod display_transport;
pub mod execution;
pub mod mux_session;
pub mod remote_node;
pub mod secure_session;
pub mod session;
pub mod session_lifecycle;
pub mod target_resolver;

pub use display_transport::{
    DISPLAY_TOKEN_LEN, DISPLAY_VSOCK_PORT, DisplayTransportBinding, DisplayTransportToken,
    encode_display_preface, verify_display_preface,
};
pub use execution::{DEFAULT_MAX_EXECUTIONS, DurableExecTable};
pub use mux_session::MuxSession;
pub use remote_node::{
    DEFAULT_HEARTBEAT_TIMEOUT, DEFAULT_MAX_REMOTE_NODES, RemoteDispatchOutcome,
    RemoteFullHostAdapter, RemoteNodeAuditLabels, RemoteNodeAvailability, RemoteNodeEntry,
    RemoteNodeError, RemoteNodeErrorKind, RemoteNodeRegistration, RemoteNodeRegistry,
    RemotePeerClient, RemotePeerStatus, RemoteRetryAction, RemoteRoute,
    ensure_remote_execution_generation, ensure_remote_shell_generation,
    retry_action_after_disconnect,
};
pub use secure_session::{
    NonceReplayGuard, SecurePeerIdentity, SecurePeerSession, SecureSessionKey,
};
pub use session::{MAX_FRAME_BYTES, PROTOCOL_VERSION, PeerSession};
pub use session_lifecycle::{LifecycleError, SessionLifecycle, SessionPhase};
pub use target_resolver::{DispatchTarget, RealmEntrypoint, RealmEntrypointTable, ResolveError};

/// Default dedup retention window. While a completed key is within this
/// window a same-request retry resolves to `Replay`; past it the key is
/// reported expired rather than silently re-executed.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(15 * 60);

/// Default no-reuse horizon. After a key's [`DEFAULT_RETENTION`] elapses the
/// router keeps an EXPIRED tombstone until this (longer) horizon so a
/// post-retention reuse keeps failing closed; only past this horizon is the
/// record dropped to bound memory.
pub const DEFAULT_NO_REUSE_HORIZON: Duration = Duration::from_secs(60 * 60);

/// Default maximum number of dedup records retained by one router scope. This
/// bounds memory for completed/tombstoned/in-progress records; callers that
/// need a tighter bound can use [`OperationRouter::with_max_dedup_records`].
pub const DEFAULT_MAX_DEDUP_RECORDS: usize = 65_536;

/// The router's decision for one operation. The caller (provider executor)
/// acts on `Accept`/`Replay`; every other variant is a typed refusal that
/// maps to a `ConstellationError` kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    /// Dispatch the operation. Carries the trusted authorization scope.
    Accept {
        /// The authorization scope derived from the operation kind.
        scope: AuthorizationScope,
    },
    /// A completed mutating operation with the same key + request is being
    /// retried; return the recorded prior result instead of re-executing.
    /// Carries the ORIGINAL accepted `operation_id` so a lost-reply retry
    /// can be correlated to its first attempt.
    Replay {
        /// The `operation_id` of the original accepted attempt.
        original_operation_id: OperationId,
        /// The result recorded at completion of the original attempt.
        result: OpaquePayload,
    },
    /// A mutating operation with the same key + request is still running.
    /// Carries the ORIGINAL accepted `operation_id` so the caller can
    /// correlate the in-flight attempt.
    InProgress {
        /// The `operation_id` of the original accepted attempt.
        original_operation_id: OperationId,
    },
    /// Same dedup key (full namespace), different request fingerprint.
    IdempotencyKeyConflict,
    /// Idempotency key reused after the dedup retention window expired.
    IdempotencyKeyExpired,
    /// A mutating operation arrived without the required idempotency key.
    MissingIdempotencyKey,
    /// The operation kind requires an addressed workload but none was supplied.
    MissingWorkload,
    /// The request principal does not match the authenticated session.
    PrincipalMismatch,
    /// The target did not advertise a required capability.
    CapabilityDenied {
        /// Capability missing from the negotiated set.
        capability: Capability,
    },
    /// The operation kind is not supported.
    UnsupportedOperation {
        /// Unsupported operation kind.
        kind: OperationKind,
    },
    /// The router cannot retain another dedup record in this scope; refusing
    /// avoids executing a mutating operation without a durable dedup lease.
    DedupCapacityExceeded,
}

/// Whether this router build implements a semantic operation kind.
///
/// The first routed ADR 0043 surface is intentionally narrow: lifecycle, exec,
/// retained logs, persistent shells, node/session health, and Wayland/display.
/// Other operation families stay fail-closed.
pub fn supported_operation_kind(kind: OperationKind) -> bool {
    operation_route_plan(kind).is_some()
}

/// Whether this router build implements a semantic stream kind.
///
/// Capability advertisement alone is not enough to enable a stream family.
/// Unsupported stream kinds get a typed `UnsupportedFeature` refusal even when a
/// peer advertises the matching capability, so there is no accidental fallback
/// to generic network, clipboard, audio, or device byte paths.
pub fn supported_stream_kind(kind: StreamKind) -> bool {
    matches!(
        kind,
        StreamKind::Control
            | StreamKind::Pty
            | StreamKind::ShellPty
            | StreamKind::Stdio
            | StreamKind::Logs
            | StreamKind::Display
    )
}

/// Pure semantic routing plan for one operation kind.
///
/// This is not a provider invocation API. It is the router-owned contract that
/// tells callers which provider family and stream families a successfully
/// authorized operation may use. Unsupported operations return `None` so callers
/// cannot accidentally translate them to SSH, provider-native shells, generic
/// sockets, or raw byte tunnels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperationRoutePlan {
    /// Operation being planned.
    pub kind: OperationKind,
    /// Trusted authorization scope derived from `kind`.
    pub authorization_scope: AuthorizationScope,
    /// Required capability derived from `kind`, if any.
    pub required_capability: Option<Capability>,
    /// Whether this operation mutates state and therefore needs idempotency.
    pub mutating: bool,
    /// Whether callers must address a workload before dispatch.
    pub requires_workload: bool,
    /// Stream families the operation is allowed to open.
    pub stream_kinds: &'static [StreamKind],
}

impl OperationRoutePlan {
    fn new(kind: OperationKind) -> Self {
        Self {
            kind,
            authorization_scope: kind.authorization_scope(),
            required_capability: kind.required_capability(),
            mutating: kind.is_mutating(),
            requires_workload: kind.requires_workload(),
            stream_kinds: kind.allowed_stream_kinds(),
        }
    }

    /// True iff every stream kind in the plan is implemented by this router.
    pub fn streams_supported(self) -> bool {
        self.stream_kinds.iter().copied().all(supported_stream_kind)
    }
}

/// Return the current semantic route plan for a supported operation kind.
pub fn operation_route_plan(kind: OperationKind) -> Option<OperationRoutePlan> {
    let plan = match kind {
        OperationKind::NodeRegister
        | OperationKind::NodeHeartbeat
        | OperationKind::NodeCapabilities
        | OperationKind::GuestHealth
        | OperationKind::WorkloadList
        | OperationKind::WorkloadStart
        | OperationKind::WorkloadStop
        | OperationKind::ExecStart
        | OperationKind::ExecAttach
        | OperationKind::ExecLogs
        | OperationKind::ExecCancel
        | OperationKind::ShellList
        | OperationKind::ShellAttach
        | OperationKind::ShellDetach
        | OperationKind::ShellKill
        | OperationKind::DisplaySessionOpen => OperationRoutePlan::new(kind),
        OperationKind::FileCopyStart | OperationKind::PortForwardOpen => return None,
    };
    plan.streams_supported().then_some(plan)
}

/// A monotonic clock, injectable so dedup expiry is deterministically
/// testable without real sleeps.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The default wall-clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The ADR 0032 dedup record key: the full operation namespace, NOT the
/// idempotency key alone. Two different principals/realms/nodes/kinds that
/// happen to reuse the same opaque idempotency key get distinct records.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DedupKey {
    realm: RealmPath,
    principal: PrincipalId,
    node: NodeId,
    kind: OperationKind,
    key: IdempotencyKey,
}

impl DedupKey {
    fn for_request(req: &OperationRequest, key: &IdempotencyKey) -> Self {
        Self {
            realm: req.realm.clone(),
            principal: req.principal.clone(),
            node: req.node.clone(),
            kind: req.kind,
            key: key.clone(),
        }
    }
}

#[derive(Debug, Clone)]
enum DedupState {
    /// Accepted and executing; never expires by retention (a long-running
    /// operation must not be dropped out from under an in-flight retry).
    /// `since` records when the lease was taken so a stale lease can be
    /// *surfaced* for provider-side reconciliation — never auto-dropped.
    InProgress { since: Instant },
    /// Completed; the retention clock (for `Replay` vs expiry) runs from
    /// `since`. Carries the recorded result for `Replay`.
    Completed {
        result: OpaquePayload,
        since: Instant,
    },
    /// Tombstone: the key was consumed and its retention elapsed. Reuse
    /// fails closed (`IdempotencyKeyExpired`) until the no-reuse horizon
    /// (measured from `since`) drops the record.
    Expired { since: Instant },
}

#[derive(Debug, Clone)]
struct DedupEntry {
    fingerprint: Vec<u8>,
    original_operation_id: OperationId,
    state: DedupState,
}

/// The codec-neutral operation router + dedup owner for one node/gateway
/// scope. Share a single instance across peer sessions (see the module
/// docs); do not construct one per session.
pub struct OperationRouter<C: Clock = SystemClock> {
    retention: Duration,
    no_reuse_horizon: Duration,
    max_dedup_records: usize,
    clock: C,
    dedup: HashMap<DedupKey, DedupEntry>,
}

impl Default for OperationRouter<SystemClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationRouter<SystemClock> {
    /// A router with the default retention + wall-clock.
    pub fn new() -> Self {
        Self::with_clock(SystemClock)
    }
}

impl<C: Clock> OperationRouter<C> {
    /// A router with an injected clock (and default retention/horizon).
    pub fn with_clock(clock: C) -> Self {
        Self {
            retention: DEFAULT_RETENTION,
            no_reuse_horizon: DEFAULT_NO_REUSE_HORIZON,
            max_dedup_records: DEFAULT_MAX_DEDUP_RECORDS,
            clock,
            dedup: HashMap::new(),
        }
    }

    /// Override the dedup retention window. The no-reuse horizon is kept at
    /// least one retention window beyond `retention` so an expired key always
    /// leaves a tombstone.
    pub fn with_retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        if self.no_reuse_horizon < retention {
            self.no_reuse_horizon = retention.saturating_mul(2);
        }
        self
    }

    /// Override the no-reuse horizon (tombstone lifetime past completion).
    pub fn with_no_reuse_horizon(mut self, horizon: Duration) -> Self {
        self.no_reuse_horizon = horizon;
        self
    }

    /// Override the maximum number of dedup records tracked by this router.
    /// When the bound is reached, a new mutating key fails closed with
    /// [`RouteDecision::DedupCapacityExceeded`] rather than being dispatched
    /// without a retained dedup lease. Existing keys can still replay,
    /// conflict, or report in-progress state because those decisions do not
    /// grow memory.
    pub fn with_max_dedup_records(mut self, max: usize) -> Self {
        self.max_dedup_records = max;
        self
    }

    /// Route one operation after checking the negotiated capability set. A
    /// missing capability fails before dedup state is mutated.
    pub fn route_with_capabilities(
        &mut self,
        req: &OperationRequest,
        session_principal: &PrincipalId,
        capabilities: &CapabilitySet,
    ) -> RouteDecision {
        // 1. Principal binding.
        if &req.principal != session_principal {
            return RouteDecision::PrincipalMismatch;
        }

        let Some(plan) = operation_route_plan(req.kind) else {
            return RouteDecision::UnsupportedOperation { kind: req.kind };
        };

        let scope = plan.authorization_scope;
        if plan.requires_workload && req.workload.is_none() {
            return RouteDecision::MissingWorkload;
        }
        if let Some(capability) = plan.required_capability
            && !capabilities.has(capability)
        {
            return RouteDecision::CapabilityDenied { capability };
        }

        // 2. Non-mutating operations need no dedup.
        if !plan.mutating {
            return RouteDecision::Accept { scope };
        }

        // 3. Mutating operations require an idempotency key.
        let key = match &req.idempotency_key {
            Some(k) => k.clone(),
            None => return RouteDecision::MissingIdempotencyKey,
        };
        let dedup_key = DedupKey::for_request(req, &key);
        let fingerprint = req.dedup_fingerprint_input();
        let now = self.clock.now();

        match self.dedup.get_mut(&dedup_key) {
            None => {
                self.gc_at(now);
                if self.dedup.len() >= self.max_dedup_records {
                    return RouteDecision::DedupCapacityExceeded;
                }
                self.dedup.insert(
                    dedup_key,
                    DedupEntry {
                        fingerprint,
                        original_operation_id: req.operation_id.clone(),
                        state: DedupState::InProgress { since: now },
                    },
                );
                RouteDecision::Accept { scope }
            }
            Some(entry) => {
                match &entry.state {
                    // A still-running attempt never expires; a different
                    // request under the same key is a conflict.
                    DedupState::InProgress { .. } => {
                        if entry.fingerprint != fingerprint {
                            RouteDecision::IdempotencyKeyConflict
                        } else {
                            RouteDecision::InProgress {
                                original_operation_id: entry.original_operation_id.clone(),
                            }
                        }
                    }

                    DedupState::Completed { result, since } => {
                        if now.duration_since(*since) > self.retention {
                            // Retention elapsed: tombstone it now (lazy) and
                            // fail closed so the key cannot be re-executed.
                            entry.state = DedupState::Expired { since: now };
                            RouteDecision::IdempotencyKeyExpired
                        } else if entry.fingerprint != fingerprint {
                            RouteDecision::IdempotencyKeyConflict
                        } else {
                            RouteDecision::Replay {
                                original_operation_id: entry.original_operation_id.clone(),
                                result: result.clone(),
                            }
                        }
                    }
                    // Tombstoned: reuse always fails closed until GC drops it.
                    DedupState::Expired { .. } => RouteDecision::IdempotencyKeyExpired,
                }
            }
        }
    }

    /// Mark a previously-accepted mutating operation complete, recording its
    /// `result` so a same-key + same-request retry resolves to
    /// [`RouteDecision::Replay`]. Returns `true` if a matching in-progress
    /// record was found. The record is identified by the full dedup namespace
    /// AND the request fingerprint, so a same-key/different-request caller
    /// (one that routed as [`RouteDecision::IdempotencyKeyConflict`] and was
    /// never accepted) can never terminalize the accepted operation's record.
    pub fn mark_completed(&mut self, req: &OperationRequest, result: OpaquePayload) -> bool {
        let key = match &req.idempotency_key {
            Some(k) => k.clone(),
            None => return false,
        };
        let dedup_key = DedupKey::for_request(req, &key);
        let fingerprint = req.dedup_fingerprint_input();
        let now = self.clock.now();
        match self.dedup.get_mut(&dedup_key) {
            Some(entry)
                if matches!(entry.state, DedupState::InProgress { .. })
                    && entry.fingerprint == fingerprint =>
            {
                entry.state = DedupState::Completed { result, since: now };
                true
            }
            _ => false,
        }
    }

    /// Mark a previously-accepted mutating operation terminally failed,
    /// removing its in-progress record so a fresh retry is accepted rather
    /// than wedged `InProgress` until expiry. Returns `true` if a matching
    /// in-progress record was removed. Like [`Self::mark_completed`], the
    /// record is matched on the full dedup namespace AND the request
    /// fingerprint, so a same-key/different-request (conflicting) caller
    /// cannot remove the accepted operation's record.
    pub fn mark_failed(&mut self, req: &OperationRequest) -> bool {
        let key = match &req.idempotency_key {
            Some(k) => k.clone(),
            None => return false,
        };
        let dedup_key = DedupKey::for_request(req, &key);
        let fingerprint = req.dedup_fingerprint_input();
        match self.dedup.get(&dedup_key) {
            Some(entry)
                if matches!(entry.state, DedupState::InProgress { .. })
                    && entry.fingerprint == fingerprint =>
            {
                self.dedup.remove(&dedup_key);
                true
            }
            _ => false,
        }
    }

    /// Bounded-memory maintenance. Never drops `InProgress` records (a
    /// long-running op must survive). Transitions `Completed` records past
    /// the retention window into `Expired` tombstones (preserving fail-closed
    /// refusal of reuse), and only removes `Expired` tombstones older than
    /// the no-reuse horizon.
    pub fn gc(&mut self) {
        let now = self.clock.now();
        self.gc_at(now);
    }

    fn gc_at(&mut self, now: Instant) {
        let retention = self.retention;
        let horizon = self.no_reuse_horizon;
        // First, age completed entries into tombstones.
        for entry in self.dedup.values_mut() {
            if let DedupState::Completed { since, .. } = &entry.state
                && now.duration_since(*since) > retention
            {
                entry.state = DedupState::Expired { since: now };
            }
        }
        // Then drop only tombstones older than the no-reuse horizon.
        self.dedup.retain(|_, e| match &e.state {
            DedupState::Expired { since } => now.duration_since(*since) <= horizon,
            _ => true,
        });
    }

    /// The number of tracked dedup records (for diagnostics/tests).
    pub fn tracked(&self) -> usize {
        self.dedup.len()
    }

    /// Convert a refusal for `req` into a typed error carrying the request's
    /// cross-realm correlation id. Accept/replay/in-progress decisions are not
    /// errors and return `None`.
    pub fn error_for_decision(
        &self,
        req: &OperationRequest,
        decision: &RouteDecision,
    ) -> Option<ConstellationError> {
        route_decision_error(req, decision)
    }

    /// List in-progress leases whose age exceeds `older_than`, oldest first,
    /// for **provider-side reconciliation**. This is read-only: it surfaces a
    /// stale lease but never resolves it. An unknown / timed-out operation
    /// stays `InProgress` until the gateway reconciles it against the
    /// provider's durable state — recording the durable id with
    /// [`Self::mark_completed`] if it took effect, or clearing it with
    /// [`Self::mark_failed`] if it provably did not. Auto-dropping or
    /// auto-completing a lease would risk a double side effect or a lost
    /// result, so the router refuses to.
    pub fn reconcilable_leases(&self, older_than: Duration) -> Vec<ReconcilableLease> {
        let now = self.clock.now();
        let mut stale: Vec<ReconcilableLease> = self
            .dedup
            .values()
            .filter_map(|entry| match &entry.state {
                DedupState::InProgress { since } => {
                    let age = now.duration_since(*since);
                    (age > older_than).then(|| ReconcilableLease {
                        original_operation_id: entry.original_operation_id.clone(),
                        age,
                    })
                }
                _ => None,
            })
            .collect();
        stale.sort_by_key(|lease| std::cmp::Reverse(lease.age));
        stale
    }
}

/// Convert a router refusal into an operator-safe typed error that preserves
/// the operation's cross-realm correlation id for route reconstruction.
pub fn route_decision_error(
    req: &OperationRequest,
    decision: &RouteDecision,
) -> Option<ConstellationError> {
    let error = match decision {
        RouteDecision::PrincipalMismatch => {
            ConstellationError::new(ErrorKind::Unauthorized, "principal binding mismatch")
        }
        RouteDecision::CapabilityDenied { capability } => {
            ConstellationError::capability_denied(*capability)
        }
        RouteDecision::MissingIdempotencyKey => ConstellationError::new(
            ErrorKind::MalformedFrame,
            "mutating operation requires an idempotency key",
        ),
        RouteDecision::IdempotencyKeyConflict => ConstellationError::new(
            ErrorKind::IdempotencyKeyConflict,
            "idempotency key conflicts with an existing operation",
        ),
        RouteDecision::IdempotencyKeyExpired => ConstellationError::new(
            ErrorKind::IdempotencyKeyExpired,
            "idempotency key was reused after retention",
        ),
        RouteDecision::DedupCapacityExceeded => {
            ConstellationError::new(ErrorKind::Backpressure, "deduplication capacity exceeded")
        }
        RouteDecision::MissingWorkload => ConstellationError::new(
            ErrorKind::InvalidTarget,
            format!(
                "operation kind '{}' requires a workload target",
                req.kind.code()
            ),
        ),
        RouteDecision::UnsupportedOperation { kind } => ConstellationError::new(
            ErrorKind::UnsupportedFeature,
            format!("operation kind '{}' is not supported", kind.code()),
        ),
        RouteDecision::Accept { .. }
        | RouteDecision::Replay { .. }
        | RouteDecision::InProgress { .. } => return None,
    };
    Some(error.with_correlation_id(req.correlation_id.clone()))
}

/// A stale in-progress lease surfaced by
/// [`OperationRouter::reconcilable_leases`] for provider-side reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilableLease {
    /// The operation id originally accepted for this lease.
    pub original_operation_id: OperationId,
    /// How long the lease has been in progress.
    pub age: Duration,
}

/// Convenience: the capability an operation requires, derived from its kind
/// (never from a caller-supplied field). `None` for node-control / health
/// operations authorized by enrollment/session rather than a capability.
pub fn required_capability(req: &OperationRequest) -> Option<Capability> {
    req.kind.required_capability()
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{
        CorrelationId, NodeId, OpaquePayload, OperationId, OperationKind, RealmPath,
    };
    use std::sync::{Arc, Mutex};

    // A manual, deterministic clock for expiry tests (Send + Sync, no unsafe).
    #[derive(Clone)]
    struct ManualClock(Arc<Mutex<Instant>>);
    impl ManualClock {
        fn new() -> Self {
            ManualClock(Arc::new(Mutex::new(Instant::now())))
        }
        fn advance(&self, d: Duration) {
            let mut t = self.0.lock().unwrap();
            *t += d;
        }
    }
    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    fn principal(s: &str) -> PrincipalId {
        PrincipalId::parse(s).unwrap()
    }

    fn req(kind: OperationKind, key: Option<&str>, body: &[u8], p: &str) -> OperationRequest {
        req_with_op(kind, key, body, p, "op-1")
    }

    fn req_with_op(
        kind: OperationKind,
        key: Option<&str>,
        body: &[u8],
        p: &str,
        op_id: &str,
    ) -> OperationRequest {
        OperationRequest {
            operation_id: OperationId::parse(op_id).unwrap(),
            correlation_id: CorrelationId::parse("corr-1").unwrap(),
            idempotency_key: key.map(|k| IdempotencyKey::parse(k).unwrap()),
            realm: RealmPath::local(),
            node: NodeId::parse("gw").unwrap(),
            workload: None,
            principal: principal(p),
            kind,
            trace: None,
            body: OpaquePayload::new(body.to_vec()).unwrap(),
        }
    }

    fn workload_req(
        kind: OperationKind,
        key: Option<&str>,
        body: &[u8],
        p: &str,
    ) -> OperationRequest {
        with_workload(req(kind, key, body, p), "work")
    }

    fn workload_req_with_op(
        kind: OperationKind,
        key: Option<&str>,
        body: &[u8],
        p: &str,
        op_id: &str,
    ) -> OperationRequest {
        with_workload(req_with_op(kind, key, body, p, op_id), "work")
    }

    fn with_workload(mut req: OperationRequest, workload: &str) -> OperationRequest {
        req.workload = Some(d2b_realm_core::WorkloadId::parse(workload).unwrap());
        req
    }

    fn req_for_kind(
        kind: OperationKind,
        key: Option<&str>,
        body: &[u8],
        p: &str,
        op_id: &str,
    ) -> OperationRequest {
        let req = req_with_op(kind, key, body, p, op_id);
        match operation_route_plan(kind) {
            Some(plan) if plan.requires_workload => with_workload(req, "work"),
            _ => req,
        }
    }

    fn result(bytes: &[u8]) -> OpaquePayload {
        OpaquePayload::new(bytes.to_vec()).unwrap()
    }

    fn caps_for(req: &OperationRequest) -> CapabilitySet {
        match req.required_capability() {
            Some(cap) => CapabilitySet::empty().with(cap),
            None => CapabilitySet::empty(),
        }
    }

    fn route<C: Clock>(
        router: &mut OperationRouter<C>,
        req: &OperationRequest,
        principal: &PrincipalId,
    ) -> RouteDecision {
        router.route_with_capabilities(req, principal, &caps_for(req))
    }

    #[test]
    fn principal_mismatch_is_rejected() {
        let mut r = OperationRouter::new();
        let req = with_workload(
            req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice"),
            "work",
        );
        assert_eq!(
            route(&mut r, &req, &principal("bob")),
            RouteDecision::PrincipalMismatch
        );
        assert_eq!(r.tracked(), 0);
        assert!(matches!(
            route(&mut r, &req, &principal("alice")),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn missing_capability_is_denied_before_dedup_state() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let req = with_workload(
            req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice"),
            "work",
        );
        let decision = r.route_with_capabilities(&req, &p, &CapabilitySet::empty());
        assert_eq!(
            decision,
            RouteDecision::CapabilityDenied {
                capability: Capability::Lifecycle,
            }
        );
        let error = route_decision_error(&req, &decision).unwrap();
        assert_eq!(error.kind(), ErrorKind::CapabilityDenied);
        assert_eq!(error.missing_capability(), Some(Capability::Lifecycle));
        assert_eq!(error.negotiated_capability_fingerprint(), None);
        assert_eq!(
            error.correlation_id().map(CorrelationId::as_str),
            Some("corr-1")
        );
        assert_eq!(r.tracked(), 0);
        assert!(matches!(
            r.route_with_capabilities(
                &req,
                &p,
                &CapabilitySet::empty().with(Capability::Lifecycle)
            ),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn non_mutating_accepts_without_key() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadList, None, b"", "alice");
        match route(&mut r, &req, &principal("alice")) {
            RouteDecision::Accept { scope } => {
                assert_eq!(scope, AuthorizationScope::capability(Capability::Lifecycle))
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[test]
    fn required_capability_and_scope_are_derived_from_kind() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let cases = [
            (
                OperationKind::NodeRegister,
                None,
                AuthorizationScope::Enrollment,
            ),
            (
                OperationKind::NodeHeartbeat,
                None,
                AuthorizationScope::NodeControl,
            ),
            (
                OperationKind::NodeCapabilities,
                None,
                AuthorizationScope::NodeControl,
            ),
            (OperationKind::GuestHealth, None, AuthorizationScope::Health),
            (
                OperationKind::WorkloadList,
                Some(Capability::Lifecycle),
                AuthorizationScope::capability(Capability::Lifecycle),
            ),
            (
                OperationKind::WorkloadStart,
                Some(Capability::Lifecycle),
                AuthorizationScope::capability(Capability::Lifecycle),
            ),
            (
                OperationKind::WorkloadStop,
                Some(Capability::Lifecycle),
                AuthorizationScope::capability(Capability::Lifecycle),
            ),
            (
                OperationKind::ExecStart,
                Some(Capability::Exec),
                AuthorizationScope::capability(Capability::Exec),
            ),
            (
                OperationKind::ExecAttach,
                Some(Capability::Exec),
                AuthorizationScope::capability(Capability::Exec),
            ),
            (
                OperationKind::ExecCancel,
                Some(Capability::Exec),
                AuthorizationScope::capability(Capability::Exec),
            ),
            (
                OperationKind::ExecLogs,
                Some(Capability::Logs),
                AuthorizationScope::capability(Capability::Logs),
            ),
            (
                OperationKind::ShellList,
                Some(Capability::PersistentShell),
                AuthorizationScope::capability(Capability::PersistentShell),
            ),
            (
                OperationKind::ShellAttach,
                Some(Capability::PersistentShell),
                AuthorizationScope::capability(Capability::PersistentShell),
            ),
            (
                OperationKind::ShellDetach,
                Some(Capability::PersistentShell),
                AuthorizationScope::capability(Capability::PersistentShell),
            ),
            (
                OperationKind::ShellKill,
                Some(Capability::PersistentShell),
                AuthorizationScope::capability(Capability::PersistentShell),
            ),
            (
                OperationKind::DisplaySessionOpen,
                Some(Capability::WindowForwarding),
                AuthorizationScope::capability(Capability::WindowForwarding),
            ),
        ];

        for (idx, (kind, expected_capability, expected_scope)) in cases.into_iter().enumerate() {
            let key = format!("scope-key-{idx}");
            let op_id = format!("op-{idx}");
            let req = req_for_kind(
                kind,
                kind.is_mutating().then_some(key.as_str()),
                b"scope",
                "alice",
                &op_id,
            );
            assert_eq!(required_capability(&req), expected_capability);
            match route(&mut r, &req, &p) {
                RouteDecision::Accept { scope } => assert_eq!(scope, expected_scope),
                other => panic!("expected Accept for {kind:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn workload_required_operations_fail_before_capability_or_dedup() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let req = req_with_op(
            OperationKind::DisplaySessionOpen,
            Some("display-key"),
            b"display",
            "alice",
            "op-display",
        );
        let decision = r.route_with_capabilities(
            &req,
            &p,
            &CapabilitySet::empty().with(Capability::WindowForwarding),
        );
        assert_eq!(decision, RouteDecision::MissingWorkload);
        let err = route_decision_error(&req, &decision).unwrap();
        assert_eq!(err.kind(), ErrorKind::InvalidTarget);
        assert!(err.message().contains("display-session-open"));
        assert_eq!(r.tracked(), 0);
    }

    #[test]
    fn unsupported_operation_families_deny_before_capability_or_dedup() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        for kind in [OperationKind::FileCopyStart, OperationKind::PortForwardOpen] {
            let req = req_with_op(kind, Some("unsupported-key"), b"opaque", "alice", "op-1");
            assert_eq!(
                r.route_with_capabilities(&req, &p, &caps_for(&req)),
                RouteDecision::UnsupportedOperation { kind }
            );
            let err =
                route_decision_error(&req, &RouteDecision::UnsupportedOperation { kind }).unwrap();
            assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
            assert_eq!(
                err.correlation_id().map(CorrelationId::as_str),
                Some("corr-1")
            );
            assert!(err.message().contains(kind.code()));
            assert_eq!(r.tracked(), 0);
        }
    }

    #[test]
    fn operation_support_policy_is_explicit() {
        let supported = [
            OperationKind::NodeRegister,
            OperationKind::NodeHeartbeat,
            OperationKind::NodeCapabilities,
            OperationKind::WorkloadList,
            OperationKind::WorkloadStart,
            OperationKind::WorkloadStop,
            OperationKind::GuestHealth,
            OperationKind::ExecStart,
            OperationKind::ExecAttach,
            OperationKind::ExecLogs,
            OperationKind::ExecCancel,
            OperationKind::ShellList,
            OperationKind::ShellAttach,
            OperationKind::ShellDetach,
            OperationKind::ShellKill,
            OperationKind::DisplaySessionOpen,
        ];
        for kind in supported {
            assert!(supported_operation_kind(kind), "{kind:?} should route");
        }
        for kind in [OperationKind::FileCopyStart, OperationKind::PortForwardOpen] {
            assert!(
                !supported_operation_kind(kind),
                "{kind:?} should fail closed"
            );
        }
    }

    #[test]
    fn stream_support_policy_is_explicit() {
        let supported = [
            StreamKind::Control,
            StreamKind::Pty,
            StreamKind::ShellPty,
            StreamKind::Stdio,
            StreamKind::Logs,
            StreamKind::Display,
        ];
        for kind in supported {
            assert!(supported_stream_kind(kind), "{kind:?} should route");
        }
        let unsupported = [
            StreamKind::FileCopy,
            StreamKind::PortForward,
            StreamKind::Clipboard,
            StreamKind::AudioPlayback,
            StreamKind::AudioCapture,
            StreamKind::DeviceHid,
            StreamKind::DeviceUsb,
        ];
        for kind in unsupported {
            assert!(!supported_stream_kind(kind), "{kind:?} should fail closed");
        }
    }

    #[test]
    fn operation_route_plan_maps_supported_operation_families() {
        let cases: &[(OperationKind, Option<Capability>, bool, &[StreamKind])] = &[
            (OperationKind::NodeRegister, None, false, &[]),
            (OperationKind::NodeHeartbeat, None, false, &[]),
            (OperationKind::NodeCapabilities, None, false, &[]),
            (
                OperationKind::WorkloadList,
                Some(Capability::Lifecycle),
                false,
                &[],
            ),
            (
                OperationKind::WorkloadStart,
                Some(Capability::Lifecycle),
                true,
                &[],
            ),
            (
                OperationKind::WorkloadStop,
                Some(Capability::Lifecycle),
                true,
                &[],
            ),
            (OperationKind::GuestHealth, None, true, &[]),
            (
                OperationKind::ExecStart,
                Some(Capability::Exec),
                true,
                &[StreamKind::Stdio, StreamKind::Pty],
            ),
            (
                OperationKind::ExecAttach,
                Some(Capability::Exec),
                true,
                &[StreamKind::Stdio, StreamKind::Pty],
            ),
            (
                OperationKind::ExecLogs,
                Some(Capability::Logs),
                true,
                &[StreamKind::Logs],
            ),
            (OperationKind::ExecCancel, Some(Capability::Exec), true, &[]),
            (
                OperationKind::ShellList,
                Some(Capability::PersistentShell),
                true,
                &[],
            ),
            (
                OperationKind::ShellAttach,
                Some(Capability::PersistentShell),
                true,
                &[StreamKind::ShellPty],
            ),
            (
                OperationKind::ShellDetach,
                Some(Capability::PersistentShell),
                true,
                &[],
            ),
            (
                OperationKind::ShellKill,
                Some(Capability::PersistentShell),
                true,
                &[],
            ),
            (
                OperationKind::DisplaySessionOpen,
                Some(Capability::WindowForwarding),
                true,
                &[StreamKind::Display],
            ),
        ];
        for (kind, required_capability, requires_workload, stream_kinds) in cases {
            let plan = operation_route_plan(*kind).unwrap();
            assert_eq!(plan.kind, *kind);
            assert_eq!(plan.authorization_scope, kind.authorization_scope());
            assert_eq!(plan.required_capability, *required_capability);
            assert_eq!(plan.mutating, kind.is_mutating());
            assert_eq!(plan.requires_workload, *requires_workload);
            assert_eq!(plan.stream_kinds, *stream_kinds);
            assert!(plan.streams_supported());
        }
    }

    #[test]
    fn operation_route_plan_refuses_future_operation_families() {
        for kind in [OperationKind::FileCopyStart, OperationKind::PortForwardOpen] {
            assert_eq!(operation_route_plan(kind), None);
        }
    }

    #[test]
    fn mutating_without_key_is_rejected() {
        let mut r = OperationRouter::new().with_max_dedup_records(0);
        let missing_key = workload_req(OperationKind::WorkloadStart, None, b"x", "alice");
        assert_eq!(
            route(&mut r, &missing_key, &principal("alice")),
            RouteDecision::MissingIdempotencyKey
        );
        assert_eq!(r.tracked(), 0);

        let keyed = workload_req(OperationKind::WorkloadStart, Some("k1"), b"x", "alice");
        assert_eq!(
            route(&mut r, &keyed, &principal("alice")),
            RouteDecision::DedupCapacityExceeded
        );
    }

    #[test]
    fn accept_then_in_progress_then_replay_carries_original_op_and_result() {
        let mut r = OperationRouter::new();
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        let p = principal("alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        // Still in progress before completion; carries the original op id.
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::InProgress {
                original_operation_id: OperationId::parse("op-1").unwrap(),
            }
        );
        assert!(r.mark_completed(&req, result(b"started-ok")));
        // After completion, the same key+request replays the recorded result.
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"started-ok"),
            }
        );
    }

    #[test]
    fn lost_reply_retry_replays_recorded_result_without_new_accept() {
        let mut shared_router = OperationRouter::new();
        let p = principal("alice");
        let first = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("lost-reply-key"),
            b"start-once",
            "alice",
            "op-original",
        );
        assert!(matches!(
            route(&mut shared_router, &first, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(shared_router.mark_completed(&first, result(b"started")));

        let retry_after_disconnect = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("lost-reply-key"),
            b"start-once",
            "alice",
            "op-retry",
        );
        assert_eq!(
            route(&mut shared_router, &retry_after_disconnect, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-original").unwrap(),
                result: result(b"started"),
            }
        );
        assert_eq!(shared_router.tracked(), 1);
    }

    #[test]
    fn same_key_different_request_conflicts() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let a = workload_req(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-A",
            "alice",
        );
        let b = workload_req(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-B",
            "alice",
        );
        assert!(matches!(
            route(&mut r, &a, &p),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(route(&mut r, &b, &p), RouteDecision::IdempotencyKeyConflict);
    }

    #[test]
    fn dedup_key_is_full_namespace_not_idempotency_key_alone() {
        // The same opaque idempotency key under a different principal must NOT
        // collide (ADR 0032 keys dedup by realm+principal+node+kind+key).
        let mut r = OperationRouter::new();
        let alice = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        let bob = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "bob");
        assert!(matches!(
            route(&mut r, &alice, &principal("alice")),
            RouteDecision::Accept { .. }
        ));
        // Same opaque key, different principal -> independent record, not a
        // false conflict and not a replay of alice's op.
        assert!(matches!(
            route(&mut r, &bob, &principal("bob")),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(r.tracked(), 2);
    }

    #[test]
    fn per_attempt_fields_excluded_from_dedup_fingerprint() {
        // A retry that changes only per-attempt fields (operation_id, trace)
        // but keeps the same key + request content must dedup as the SAME
        // request (Replay after completion), never a conflict.
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let first = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start",
            "alice",
            "op-1",
        );
        let retry = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start",
            "alice",
            "op-2",
        );
        assert!(matches!(
            route(&mut r, &first, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(r.mark_completed(&first, result(b"ok")));
        // Different operation_id, same fingerprint -> Replay of the original.
        assert_eq!(
            route(&mut r, &retry, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"ok"),
            }
        );
    }

    #[test]
    fn in_progress_key_does_not_expire() {
        // A never-completed (still running) op must not be expired/dropped
        // just because the retention window elapsed.
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        clock.advance(Duration::from_secs(61));
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::InProgress {
                original_operation_id: OperationId::parse("op-1").unwrap(),
            }
        );
    }

    #[test]
    fn completed_key_reused_after_retention_is_expired() {
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(r.mark_completed(&req, result(b"ok")));
        clock.advance(Duration::from_secs(61));
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::IdempotencyKeyExpired
        );
    }

    #[test]
    fn capacity_exhaustion_fails_closed_for_new_keys() {
        let mut r = OperationRouter::new().with_max_dedup_records(1);
        let p = principal("alice");
        let first = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-one",
            "alice",
            "op-1",
        );
        let second = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k2"),
            b"start-two",
            "alice",
            "op-2",
        );

        assert!(matches!(
            route(&mut r, &first, &p),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(
            route(&mut r, &second, &p),
            RouteDecision::DedupCapacityExceeded
        );
        assert_eq!(r.tracked(), 1);

        assert!(r.mark_completed(&first, result(b"first-result")));
        assert_eq!(
            route(&mut r, &first, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"first-result"),
            }
        );
    }

    #[test]
    fn capacity_reclaims_expired_tombstones_before_refusing_new_keys() {
        let clock = ManualClock::new();
        let mut r = OperationRouter::with_clock(clock.clone())
            .with_retention(Duration::from_secs(10))
            .with_no_reuse_horizon(Duration::from_secs(10))
            .with_max_dedup_records(1);
        let p = principal("alice");
        let first = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-one",
            "alice",
            "op-1",
        );
        let second = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k2"),
            b"start-two",
            "alice",
            "op-2",
        );

        assert!(matches!(
            route(&mut r, &first, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(r.mark_completed(&first, result(b"first-result")));
        clock.advance(Duration::from_secs(11));
        assert_eq!(
            route(&mut r, &first, &p),
            RouteDecision::IdempotencyKeyExpired
        );
        clock.advance(Duration::from_secs(11));

        assert!(matches!(
            route(&mut r, &second, &p),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(r.tracked(), 1);
    }

    #[test]
    fn expired_tombstone_survives_gc_until_no_reuse_horizon() {
        // route -> complete -> expire -> gc -> same key must STILL fail closed
        // (IdempotencyKeyExpired), not silently re-accept, until the longer
        // no-reuse horizon drops the tombstone.
        let clock = ManualClock::new();
        let mut r = OperationRouter::with_clock(clock.clone())
            .with_retention(Duration::from_secs(60))
            .with_no_reuse_horizon(Duration::from_secs(600));
        let p = principal("alice");
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(r.mark_completed(&req, result(b"ok")));
        clock.advance(Duration::from_secs(61));
        // Retention elapsed: expired (and tombstoned).
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::IdempotencyKeyExpired
        );
        r.gc();
        // Tombstone kept; reuse still fails closed.
        assert_eq!(r.tracked(), 1);
        assert_eq!(
            route(&mut r, &req, &p),
            RouteDecision::IdempotencyKeyExpired
        );
        // Past the no-reuse horizon the tombstone is dropped.
        clock.advance(Duration::from_secs(601));
        r.gc();
        assert_eq!(r.tracked(), 0);
        // Only now (after the full no-reuse horizon) is the key fresh again.
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn gc_keeps_in_progress_records() {
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        clock.advance(Duration::from_secs(61));
        r.gc();
        // In-progress record is never dropped.
        assert_eq!(r.tracked(), 1);
    }

    #[test]
    fn mark_failed_allows_a_fresh_retry() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let req = workload_req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
        assert!(r.mark_failed(&req));
        // After terminal failure the key is fresh, not wedged InProgress.
        assert!(matches!(
            route(&mut r, &req, &p),
            RouteDecision::Accept { .. }
        ));
    }

    #[test]
    fn conflicting_request_cannot_terminalize_accepted_record() {
        // A same-key/different-body request routes as a conflict and is never
        // accepted; it must NOT be able to complete or fail the genuinely
        // accepted operation's record (which would corrupt replay/dedup).
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let accepted = workload_req(OperationKind::WorkloadStart, Some("k1"), b"real", "alice");
        let conflicting =
            workload_req(OperationKind::WorkloadStart, Some("k1"), b"forged", "alice");
        assert!(matches!(
            route(&mut r, &accepted, &p),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(
            route(&mut r, &conflicting, &p),
            RouteDecision::IdempotencyKeyConflict
        );
        // The conflicting caller cannot terminalize the accepted record.
        assert!(!r.mark_completed(&conflicting, result(b"forged-result")));
        assert!(!r.mark_failed(&conflicting));
        // The accepted op is still in progress and still owns its record.
        assert_eq!(
            route(&mut r, &accepted, &p),
            RouteDecision::InProgress {
                original_operation_id: OperationId::parse("op-1").unwrap(),
            }
        );
        // The legitimate completer (matching fingerprint) still works.
        assert!(r.mark_completed(&accepted, result(b"real-result")));
        assert_eq!(
            route(&mut r, &accepted, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"real-result"),
            }
        );
    }

    #[test]
    fn reconcilable_leases_surfaces_stale_in_progress_without_resolving() {
        let clock = ManualClock::new();
        let mut r = OperationRouter::with_clock(clock.clone());
        let p = "alice";
        let op = workload_req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start",
            p,
            "op-1",
        );
        assert!(matches!(
            route(&mut r, &op, &principal(p)),
            RouteDecision::Accept { .. }
        ));

        // Fresh lease: not yet stale at a 30s threshold.
        assert!(r.reconcilable_leases(Duration::from_secs(30)).is_empty());

        // Advance past the threshold: the lease is surfaced, still in-progress.
        clock.advance(Duration::from_secs(31));
        let stale = r.reconcilable_leases(Duration::from_secs(30));
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].original_operation_id,
            OperationId::parse("op-1").unwrap()
        );
        // Surfacing did NOT resolve it: it is still InProgress to a retry.
        assert!(matches!(
            route(&mut r, &op, &principal(p)),
            RouteDecision::InProgress { .. }
        ));

        // Reconciling it (mark_completed) clears it from the stale list.
        assert!(r.mark_completed(&op, result(b"done")));
        assert!(r.reconcilable_leases(Duration::from_secs(30)).is_empty());
    }
}
