//! `nixling-constellation-router` (ADR 0032): the **codec-neutral**
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
//! double-dispatch. See `nixlingd`'s `PeerOperationRouter` for the wiring.
//!
//! Dependency direction: depends ONLY on `nixling-constellation-core` +
//! `nixling-constellation-provider`. It MUST NOT depend on `prost`, a codec
//! crate, a transport impl, or any host-only internals (enforced by the
//! constellation dependency-direction CI gate).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nixling_constellation_core::{
    AuthorizationScope, Capability, IdempotencyKey, NodeId, OpaquePayload, OperationId,
    OperationKind, OperationRequest, PrincipalId, RealmPath,
};

pub mod session;

pub use session::{MAX_FRAME_BYTES, PROTOCOL_VERSION, PeerSession};

/// Default dedup retention window. While a completed key is within this
/// window a same-request retry resolves to `Replay`; past it the key is
/// reported expired rather than silently re-executed.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(15 * 60);

/// Default no-reuse horizon. After a key's [`DEFAULT_RETENTION`] elapses the
/// router keeps an EXPIRED tombstone until this (longer) horizon so a
/// post-retention reuse keeps failing closed; only past this horizon is the
/// record dropped to bound memory.
pub const DEFAULT_NO_REUSE_HORIZON: Duration = Duration::from_secs(60 * 60);

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
    /// The request principal does not match the authenticated session.
    PrincipalMismatch,
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
    InProgress,
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

    /// Route one operation against the authenticated `session_principal`.
    ///
    /// On `Accept` for a mutating operation the key is recorded as
    /// in-progress; the caller MUST later call [`Self::mark_completed`]
    /// (success) or [`Self::mark_failed`] (terminal failure) so the dedup
    /// record reaches a terminal state instead of wedging `InProgress`.
    pub fn route(
        &mut self,
        req: &OperationRequest,
        session_principal: &PrincipalId,
    ) -> RouteDecision {
        // 1. Principal binding.
        if &req.principal != session_principal {
            return RouteDecision::PrincipalMismatch;
        }

        let scope = req.kind.authorization_scope();

        // 2. Non-mutating operations need no dedup.
        if !req.kind.is_mutating() {
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
                self.dedup.insert(
                    dedup_key,
                    DedupEntry {
                        fingerprint,
                        original_operation_id: req.operation_id.clone(),
                        state: DedupState::InProgress,
                    },
                );
                RouteDecision::Accept { scope }
            }
            Some(entry) => {
                match &entry.state {
                    // A still-running attempt never expires; a different
                    // request under the same key is a conflict.
                    DedupState::InProgress => {
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
                if matches!(entry.state, DedupState::InProgress)
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
                if matches!(entry.state, DedupState::InProgress)
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
    use nixling_constellation_core::{
        NodeId, OpaquePayload, OperationId, OperationKind, RealmPath,
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

    fn result(bytes: &[u8]) -> OpaquePayload {
        OpaquePayload::new(bytes.to_vec()).unwrap()
    }

    #[test]
    fn principal_mismatch_is_rejected() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadList, None, b"", "alice");
        assert_eq!(
            r.route(&req, &principal("bob")),
            RouteDecision::PrincipalMismatch
        );
    }

    #[test]
    fn non_mutating_accepts_without_key() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadList, None, b"", "alice");
        match r.route(&req, &principal("alice")) {
            RouteDecision::Accept { scope } => {
                assert_eq!(scope, AuthorizationScope::capability(Capability::Lifecycle))
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[test]
    fn mutating_without_key_is_rejected() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadStart, None, b"x", "alice");
        assert_eq!(
            r.route(&req, &principal("alice")),
            RouteDecision::MissingIdempotencyKey
        );
    }

    #[test]
    fn accept_then_in_progress_then_replay_carries_original_op_and_result() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        let p = principal("alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        // Still in progress before completion; carries the original op id.
        assert_eq!(
            r.route(&req, &p),
            RouteDecision::InProgress {
                original_operation_id: OperationId::parse("op-1").unwrap(),
            }
        );
        assert!(r.mark_completed(&req, result(b"started-ok")));
        // After completion, the same key+request replays the recorded result.
        assert_eq!(
            r.route(&req, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"started-ok"),
            }
        );
    }

    #[test]
    fn same_key_different_request_conflicts() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let a = req(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-A",
            "alice",
        );
        let b = req(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start-B",
            "alice",
        );
        assert!(matches!(r.route(&a, &p), RouteDecision::Accept { .. }));
        assert_eq!(r.route(&b, &p), RouteDecision::IdempotencyKeyConflict);
    }

    #[test]
    fn dedup_key_is_full_namespace_not_idempotency_key_alone() {
        // The same opaque idempotency key under a different principal must NOT
        // collide (ADR 0032 keys dedup by realm+principal+node+kind+key).
        let mut r = OperationRouter::new();
        let alice = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        let bob = req(OperationKind::WorkloadStart, Some("k1"), b"start", "bob");
        assert!(matches!(
            r.route(&alice, &principal("alice")),
            RouteDecision::Accept { .. }
        ));
        // Same opaque key, different principal -> independent record, not a
        // false conflict and not a replay of alice's op.
        assert!(matches!(
            r.route(&bob, &principal("bob")),
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
        let first = req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start",
            "alice",
            "op-1",
        );
        let retry = req_with_op(
            OperationKind::WorkloadStart,
            Some("k1"),
            b"start",
            "alice",
            "op-2",
        );
        assert!(matches!(r.route(&first, &p), RouteDecision::Accept { .. }));
        assert!(r.mark_completed(&first, result(b"ok")));
        // Different operation_id, same fingerprint -> Replay of the original.
        assert_eq!(
            r.route(&retry, &p),
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
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        clock.advance(Duration::from_secs(61));
        assert_eq!(
            r.route(&req, &p),
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
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        assert!(r.mark_completed(&req, result(b"ok")));
        clock.advance(Duration::from_secs(61));
        assert_eq!(r.route(&req, &p), RouteDecision::IdempotencyKeyExpired);
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
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        assert!(r.mark_completed(&req, result(b"ok")));
        clock.advance(Duration::from_secs(61));
        // Retention elapsed: expired (and tombstoned).
        assert_eq!(r.route(&req, &p), RouteDecision::IdempotencyKeyExpired);
        r.gc();
        // Tombstone kept; reuse still fails closed.
        assert_eq!(r.tracked(), 1);
        assert_eq!(r.route(&req, &p), RouteDecision::IdempotencyKeyExpired);
        // Past the no-reuse horizon the tombstone is dropped.
        clock.advance(Duration::from_secs(601));
        r.gc();
        assert_eq!(r.tracked(), 0);
        // Only now (after the full no-reuse horizon) is the key fresh again.
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
    }

    #[test]
    fn gc_keeps_in_progress_records() {
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        clock.advance(Duration::from_secs(61));
        r.gc();
        // In-progress record is never dropped.
        assert_eq!(r.tracked(), 1);
    }

    #[test]
    fn mark_failed_allows_a_fresh_retry() {
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        assert!(r.mark_failed(&req));
        // After terminal failure the key is fresh, not wedged InProgress.
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
    }

    #[test]
    fn conflicting_request_cannot_terminalize_accepted_record() {
        // A same-key/different-body request routes as a conflict and is never
        // accepted; it must NOT be able to complete or fail the genuinely
        // accepted operation's record (which would corrupt replay/dedup).
        let mut r = OperationRouter::new();
        let p = principal("alice");
        let accepted = req(OperationKind::WorkloadStart, Some("k1"), b"real", "alice");
        let conflicting = req(OperationKind::WorkloadStart, Some("k1"), b"forged", "alice");
        assert!(matches!(
            r.route(&accepted, &p),
            RouteDecision::Accept { .. }
        ));
        assert_eq!(
            r.route(&conflicting, &p),
            RouteDecision::IdempotencyKeyConflict
        );
        // The conflicting caller cannot terminalize the accepted record.
        assert!(!r.mark_completed(&conflicting, result(b"forged-result")));
        assert!(!r.mark_failed(&conflicting));
        // The accepted op is still in progress and still owns its record.
        assert_eq!(
            r.route(&accepted, &p),
            RouteDecision::InProgress {
                original_operation_id: OperationId::parse("op-1").unwrap(),
            }
        );
        // The legitimate completer (matching fingerprint) still works.
        assert!(r.mark_completed(&accepted, result(b"real-result")));
        assert_eq!(
            r.route(&accepted, &p),
            RouteDecision::Replay {
                original_operation_id: OperationId::parse("op-1").unwrap(),
                result: result(b"real-result"),
            }
        );
    }
}
