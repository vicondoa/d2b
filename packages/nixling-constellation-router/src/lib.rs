//! `nixling-constellation-router` (ADR 0032): the **codec-neutral**
//! operation router. It reasons only over the semantic
//! [`OperationRequest`] envelope (never a wire encoding), so it owns the
//! three cross-cutting invariants the design panel made load-bearing:
//!
//! - **Principal binding**: the request principal MUST match the
//!   authenticated session principal.
//! - **Required-capability derivation**: the capability/scope is derived
//!   from [`OperationKind`] in trusted code, never from a caller-supplied
//!   field.
//! - **Idempotency + dedup ownership**: the router is the single dedup
//!   owner. A mutating operation MUST carry an idempotency key;
//!   same-key/same-request is a replay, same-key/different-request is a
//!   conflict, and a key reused after the dedup retention window is expired.
//!
//! Dependency direction: depends ONLY on `nixling-constellation-core` +
//! `nixling-constellation-provider`. It MUST NOT depend on `prost`, a codec
//! crate, a transport impl, or any host-only internals (enforced by
//! `tests/unit/meta/w0-dep-direction.sh`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nixling_constellation_core::{
    AuthorizationScope, Capability, IdempotencyKey, OperationRequest, PrincipalId,
};

/// Default dedup retention window. A key reused after this window is
/// reported as expired rather than silently re-executed.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(15 * 60);

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
    /// retried; return the prior result instead of re-executing.
    Replay,
    /// A mutating operation with the same key + request is still running.
    InProgress,
    /// Same idempotency key, different request fingerprint (conflict).
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum DedupState {
    InProgress,
    Completed,
}

#[derive(Debug, Clone)]
struct DedupEntry {
    fingerprint: Vec<u8>,
    state: DedupState,
    recorded: Instant,
}

/// The codec-neutral operation router + dedup owner.
pub struct OperationRouter<C: Clock = SystemClock> {
    retention: Duration,
    clock: C,
    dedup: HashMap<IdempotencyKey, DedupEntry>,
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
    /// A router with an injected clock (and default retention).
    pub fn with_clock(clock: C) -> Self {
        Self {
            retention: DEFAULT_RETENTION,
            clock,
            dedup: HashMap::new(),
        }
    }

    /// Override the dedup retention window.
    pub fn with_retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        self
    }

    /// Route one operation against the authenticated `session_principal`.
    ///
    /// On `Accept` for a mutating operation the key is recorded as
    /// in-progress; the caller MUST later call [`Self::mark_completed`] so a
    /// subsequent same-key/same-request retry resolves to `Replay`.
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
        let fingerprint = req.dedup_fingerprint_input();
        let now = self.clock.now();

        match self.dedup.get(&key) {
            None => {
                self.dedup.insert(
                    key,
                    DedupEntry {
                        fingerprint,
                        state: DedupState::InProgress,
                        recorded: now,
                    },
                );
                RouteDecision::Accept { scope }
            }
            Some(entry) => {
                if now.duration_since(entry.recorded) > self.retention {
                    // Key reused after its dedup window expired.
                    RouteDecision::IdempotencyKeyExpired
                } else if entry.fingerprint != fingerprint {
                    RouteDecision::IdempotencyKeyConflict
                } else {
                    match entry.state {
                        DedupState::InProgress => RouteDecision::InProgress,
                        DedupState::Completed => RouteDecision::Replay,
                    }
                }
            }
        }
    }

    /// Mark a previously-accepted mutating operation complete so a same-key
    /// + same-request retry resolves to [`RouteDecision::Replay`].
    pub fn mark_completed(&mut self, key: &IdempotencyKey) {
        if let Some(entry) = self.dedup.get_mut(key) {
            entry.state = DedupState::Completed;
        }
    }

    /// Drop dedup entries older than the retention window (bounded memory).
    pub fn gc(&mut self) {
        let now = self.clock.now();
        let retention = self.retention;
        self.dedup
            .retain(|_, e| now.duration_since(e.recorded) <= retention);
    }

    /// The number of tracked dedup entries (for diagnostics/tests).
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
        OperationRequest {
            operation_id: OperationId::parse("op-1").unwrap(),
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
    fn same_key_same_request_replays_after_completion() {
        let mut r = OperationRouter::new();
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        let p = principal("alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        // Still in progress before completion.
        assert_eq!(r.route(&req, &p), RouteDecision::InProgress);
        r.mark_completed(&IdempotencyKey::parse("k1").unwrap());
        // After completion, the same key+request is a replay.
        assert_eq!(r.route(&req, &p), RouteDecision::Replay);
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
    fn key_reused_after_retention_is_expired() {
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        clock.advance(Duration::from_secs(61));
        assert_eq!(r.route(&req, &p), RouteDecision::IdempotencyKeyExpired);
    }

    #[test]
    fn gc_drops_expired_entries() {
        let clock = ManualClock::new();
        let mut r =
            OperationRouter::with_clock(clock.clone()).with_retention(Duration::from_secs(60));
        let p = principal("alice");
        let req = req(OperationKind::WorkloadStart, Some("k1"), b"start", "alice");
        assert!(matches!(r.route(&req, &p), RouteDecision::Accept { .. }));
        assert_eq!(r.tracked(), 1);
        clock.advance(Duration::from_secs(61));
        r.gc();
        assert_eq!(r.tracked(), 0);
    }
}
