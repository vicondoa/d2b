//! The async display-session orchestrator (ADR 0032 P0, design §3). It
//! composes the proven pieces — a detached in-sandbox agent (ACA exec), the
//! host relay listener, and the operator display endpoint — driving the
//! [`SessionLedger`] state machine and minting + binding the one-shot
//! [`SessionBinding`] so display bytes are admitted only by the verified
//! handshake.
//!
//! Every side effect is an **injected dependency** ([`GatewayDeps`]), so the
//! orchestrator is exhaustively unit-testable with mocks and never needs live
//! Azure in tests. The daemon supplies real implementations (the ACA
//! `WorkloadProvider`, the relay `TransportProvider`, the operator
//! display runner).

use async_trait::async_trait;
use std::sync::{Mutex, MutexGuard};

use crate::audit::{GatewayAudit, GatewayAuditEvent, GatewayAuditKind, display_envelope};
use crate::error::GatewayError;
use crate::handshake::{DisplaySessionId, SessionBinding, SessionSecret};
use crate::ledger::{LedgerLimits, OpOutcome, SessionLedger, SessionState, TargetKey};
use crate::types::{AppCommand, DisplaySessionContext};
use d2b_realm_core::{AuthzDecision, OperationId, PrincipalId, RealmId, RealmPath, WorkloadId};

/// How long a minted session credential is valid (seconds) before
/// `not_after`. The agent must complete its handshake within this window.
pub const DEFAULT_SESSION_TTL_SECS: u64 = 120;

/// The relay coordinates + secret the gateway hands the in-sandbox agent over
/// the provider control plane (never over the relay, never logged).
#[derive(Clone)]
pub struct AgentSpawnRequest {
    /// The session being established.
    pub ctx: DisplaySessionContext,
    /// The bound binding the agent must MAC and send as its handshake.
    pub binding: SessionBinding,
    /// The one-shot secret `S` (delivered over the ACA control plane only).
    pub secret: SessionSecret,
    /// The app to launch in the sandbox once the channel is up.
    pub app: AppCommand,
}

impl core::fmt::Debug for AgentSpawnRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never leak the secret or argv.
        f.debug_struct("AgentSpawnRequest")
            .field("session_id", &self.ctx.session_id)
            .field("program", &self.app.program())
            .finish_non_exhaustive()
    }
}

/// An opaque handle to a spawned in-sandbox agent, used for cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHandle(pub String);

/// An opaque handle to an armed host relay listener, used for teardown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerHandle(pub String);

/// The detached workload-agent contract. ACA `exec` is synchronous and has no
/// logs/cancel/status, so the gateway models the in-sandbox agent as a
/// detached execution with an explicit cleanup path (design §3
/// `exec-lifecycle-dto`).
#[async_trait]
pub trait GatewayWorkload: Send + Sync {
    /// Spawn the in-sandbox agent (detached): deliver the relay coords +
    /// binding + secret over the provider control plane and start
    /// `d2b-relay sender` + `waypipe server` + the app. Returns a durable
    /// handle for cleanup/reconciliation.
    async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError>;
    /// Best-effort teardown of a spawned agent (kill its in-sandbox process
    /// group). Idempotent; safe to call on a partially-spawned agent.
    async fn cleanup(&self, handle: &AgentHandle) -> Result<(), GatewayError>;
}

/// Arming the host relay listener that verifies the session handshake before
/// bridging any byte to the operator display endpoint. The implementation
/// owns the verification (it has the secret + the authorizing binding); the
/// orchestrator only sequences it.
#[async_trait]
pub trait DisplayListener: Send + Sync {
    /// Arm the listener for `ctx`, verifying a handshake bound to `binding`
    /// under `secret` before forwarding bytes. Resolves once the listener is
    /// registered and ready to accept the sender rendezvous.
    async fn arm(
        &self,
        ctx: &DisplaySessionContext,
        binding: &SessionBinding,
        secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError>;
    /// Wait for the agent's verified handshake to complete (bytes are now
    /// flowing). Returns `Ok(())` on a verified handshake, or a typed error
    /// (e.g. timeout / auth failure) otherwise.
    async fn await_handshake(&self, handle: &ListenerHandle) -> Result<(), GatewayError>;
    /// Tear down an armed listener. Idempotent.
    async fn close(&self, handle: &ListenerHandle) -> Result<(), GatewayError>;
}

/// A monotonic clock seam (so tests are deterministic).
pub trait Clock: Send + Sync {
    /// Unix seconds now.
    fn now_unix(&self) -> u64;
}

/// A source of session ids + secrets (a CSPRNG in production; deterministic in
/// tests).
pub trait IdSource: Send + Sync {
    /// A fresh opaque (non-secret) session id.
    fn new_session_id(&self) -> DisplaySessionId;
    /// A fresh 32-byte session secret.
    fn new_secret(&self) -> SessionSecret;
}

/// The injected side-effect dependencies. The orchestrator owns no I/O itself.
pub struct GatewayDeps {
    /// The in-sandbox agent driver (ACA provider in production).
    pub workload: Box<dyn GatewayWorkload>,
    /// The host relay listener / display endpoint driver.
    pub listener: Box<dyn DisplayListener>,
    /// Clock seam.
    pub clock: Box<dyn Clock>,
    /// Id/secret source.
    pub ids: Box<dyn IdSource>,
    /// Gateway-local audit sink.
    pub audit: Box<dyn GatewayAudit>,
}

/// The orchestrator: owns the ledger (one generation) + the injected deps, and
/// drives `open`/`close` for display sessions.
pub struct GatewayOrchestrator {
    deps: GatewayDeps,
    ledger: Mutex<SessionLedger>,
    ttl_secs: u64,
}

/// A live (opened) display session: the ledger id + the handles needed to
/// tear it down.
#[derive(Debug, Clone)]
pub struct OpenSession {
    /// The session id.
    pub session_id: DisplaySessionId,
    /// The in-sandbox agent handle.
    pub agent: AgentHandle,
    /// The host listener handle.
    pub listener: ListenerHandle,
}

/// Non-secret summary for `display list` style inspection. It contains only
/// bounded identifiers and state; handles, secrets, argv, sockets, and paths
/// stay out of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySessionSummary {
    /// The session id.
    pub session_id: DisplaySessionId,
    /// The current lifecycle state.
    pub state: SessionState,
    /// Gateway generation that owns the record.
    pub generation: u64,
    /// Target realm.
    pub realm: RealmPath,
    /// Target workload.
    pub workload: WorkloadId,
    /// Authorizing operation id.
    pub operation_id: OperationId,
    /// Authorizing principal.
    pub peer_principal: PrincipalId,
}

impl DisplaySessionSummary {
    fn from_record(record: crate::ledger::SessionRecord) -> Result<Self, GatewayError> {
        let realm = parse_target_realm(&record.target.realm)?;
        let workload =
            WorkloadId::parse(record.target.workload).map_err(|_| GatewayError::Internal)?;
        let operation_id =
            OperationId::parse(record.operation_id).map_err(|_| GatewayError::Internal)?;
        let peer_principal =
            PrincipalId::parse(record.principal).map_err(|_| GatewayError::Internal)?;
        Ok(Self {
            session_id: record.id,
            state: record.state,
            generation: record.generation,
            realm,
            workload,
            operation_id,
            peer_principal,
        })
    }
}

fn parse_target_realm(target: &str) -> Result<RealmPath, GatewayError> {
    let labels = target
        .split('.')
        .map(|label| RealmId::parse(label.to_owned()).ok())
        .collect::<Option<Vec<_>>>()
        .ok_or(GatewayError::Internal)?;
    RealmPath::new(labels).ok_or(GatewayError::Internal)
}

impl GatewayOrchestrator {
    /// Build an orchestrator owned by gateway `generation`.
    pub fn new(deps: GatewayDeps, generation: u64, limits: LedgerLimits) -> Self {
        Self {
            deps,
            ledger: Mutex::new(SessionLedger::new(generation, limits)),
            ttl_secs: DEFAULT_SESSION_TTL_SECS,
        }
    }

    fn ledger(&self) -> Result<MutexGuard<'_, SessionLedger>, GatewayError> {
        self.ledger.lock().map_err(|_| GatewayError::Internal)
    }

    /// The owning gateway generation.
    pub fn generation(&self) -> u64 {
        self.ledger()
            .map(|ledger| ledger.generation())
            .unwrap_or_default()
    }

    /// The current state of a session, if tracked.
    pub fn state(&self, id: &DisplaySessionId) -> Option<SessionState> {
        self.ledger().ok().and_then(|ledger| ledger.state(id))
    }

    /// Active display sessions known to this gateway generation.
    pub fn list_sessions(&self) -> Result<Vec<DisplaySessionSummary>, GatewayError> {
        self.ledger()?
            .active_records()
            .into_iter()
            .map(DisplaySessionSummary::from_record)
            .collect()
    }

    /// Open a display session for `ctx_seed` (which carries the realm,
    /// operation id, and principal) presenting `app`. Drives the full state
    /// machine; on any failure it runs compensating cleanup and surfaces a
    /// typed [`GatewayError`]. An idempotent replay of the same operation
    /// returns the original session without re-spawning.
    pub async fn open(
        &self,
        realm_target: TargetKey,
        ctx_seed: ContextSeed,
        app: AppCommand,
        request_hash: u64,
    ) -> Result<OpenSession, GatewayError> {
        let new_id = self.deps.ids.new_session_id();
        // 1. Ledger admission (idempotency / single-session cap / quotas).
        let outcome = {
            let mut ledger = self.ledger()?;
            match ledger.open(
                realm_target,
                ctx_seed.principal.as_str(),
                ctx_seed.operation_id.as_str(),
                request_hash,
                new_id,
            ) {
                Ok(outcome) => outcome,
                Err(err) => {
                    drop(ledger);
                    self.audit_open_denied(&ctx_seed, err.clone())?;
                    return Err(err);
                }
            }
        };
        let session_id = match outcome {
            OpOutcome::Replay(id) => {
                // Idempotent replay: the caller already has a live session for
                // this operation. Return the original id without re-spawning.
                // (Empty handles signal "no new resources were created".)
                return Ok(OpenSession {
                    session_id: id,
                    agent: AgentHandle(String::new()),
                    listener: ListenerHandle(String::new()),
                });
            }
            OpOutcome::Accepted(id) => id,
        };

        // 2. Mint the one-shot credential bound to the authorizing operation.
        let now = self.deps.clock.now_unix();
        let not_after = now.saturating_add(self.ttl_secs);
        let generation = self.ledger()?.generation();
        let ctx = DisplaySessionContext {
            session_id: session_id.clone(),
            operation_id: ctx_seed.operation_id.clone(),
            realm: ctx_seed.realm.clone(),
            generation,
            peer_principal: ctx_seed.principal.clone(),
        };
        let binding = SessionBinding::new(
            &ctx.realm,
            generation,
            &session_id,
            0,
            &ctx.operation_id,
            &ctx.peer_principal,
            &ctx_seed.workload,
            not_after,
        );
        let secret = self.deps.ids.new_secret();
        self.audit_open_admitted(&ctx_seed, &session_id, SessionState::Minting)?;

        // 3. Drive the state machine with compensating cleanup on any failure.
        let result = self.drive_open(&ctx, &binding, &secret, &app).await;
        match result {
            Ok(open) => {
                self.audit_running(&ctx_seed, &open.session_id)?;
                Ok(open)
            }
            Err(e) => {
                // Compensate: best-effort cleanup + ledger Failed/Closed.
                let _ = self.fail_session(&session_id).await;
                Err(e)
            }
        }
    }

    async fn drive_open(
        &self,
        ctx: &DisplaySessionContext,
        binding: &SessionBinding,
        secret: &SessionSecret,
        app: &AppCommand,
    ) -> Result<OpenSession, GatewayError> {
        let id = &ctx.session_id;
        // Minting -> ListenerArming. The relay listener must be registered
        // before the sandbox sender dials; Azure Relay resets sender
        // rendezvous attempts when no listener control channel is armed.
        self.ledger()?
            .transition(id, SessionState::ListenerArming)?;
        let listener = self.deps.listener.arm(ctx, binding, secret).await?;

        // ListenerArming -> AgentSpawning.
        self.ledger()?.transition(id, SessionState::AgentSpawning)?;
        let spawn = AgentSpawnRequest {
            ctx: ctx.clone(),
            binding: binding.clone(),
            secret: secret.clone(),
            app: app.clone(),
        };
        let agent = match self.deps.workload.spawn_agent(&spawn).await {
            Ok(agent) => agent,
            Err(err) => {
                let _ = self.deps.listener.close(&listener).await;
                return Err(err);
            }
        };

        // AgentSpawning -> AwaitingHandshake
        self.ledger()?
            .transition(id, SessionState::AwaitingHandshake)?;
        if let Err(err) = self.deps.listener.await_handshake(&listener).await {
            let _ = self.deps.workload.cleanup(&agent).await;
            let _ = self.deps.listener.close(&listener).await;
            return Err(err);
        }

        // AwaitingHandshake -> Running
        self.ledger()?.transition(id, SessionState::Running)?;
        Ok(OpenSession {
            session_id: id.clone(),
            agent,
            listener,
        })
    }

    async fn fail_session(&self, id: &DisplaySessionId) -> Result<(), GatewayError> {
        // Best-effort: the handles may not exist yet; cleanup is idempotent.
        let _ = self
            .ledger
            .lock()
            .map(|mut ledger| ledger.transition(id, SessionState::Failed));
        Ok(())
    }

    /// Close a live session: tear down the listener + agent, then mark Closed.
    /// Idempotent and cleanup-complete even on partial failure.
    pub async fn close(&self, open: &OpenSession) -> Result<(), GatewayError> {
        let id = &open.session_id;
        let _ = self
            .ledger
            .lock()
            .map(|mut ledger| ledger.transition(id, SessionState::Stopping));
        // Tear down both sides regardless of individual errors.
        let l = self.deps.listener.close(&open.listener).await;
        let a = self.deps.workload.cleanup(&open.agent).await;
        let _ = self
            .ledger
            .lock()
            .map(|mut ledger| ledger.transition(id, SessionState::Closed));
        l.and(a)
    }

    fn audit_open_admitted(
        &self,
        seed: &ContextSeed,
        session_id: &DisplaySessionId,
        state: SessionState,
    ) -> Result<(), GatewayError> {
        let envelope = display_envelope(
            seed.operation_id.clone(),
            seed.correlation_id.clone(),
            seed.realm.clone(),
            seed.principal.clone(),
            seed.node.clone(),
            seed.workload.clone(),
            AuthzDecision::Allow,
        );
        self.deps.audit.record(GatewayAuditEvent {
            kind: GatewayAuditKind::DisplaySessionOpenAdmitted,
            envelope,
            session_id: Some(session_id.clone()),
            state: Some(state),
            error_slug: None,
        })
    }

    fn audit_open_denied(&self, seed: &ContextSeed, err: GatewayError) -> Result<(), GatewayError> {
        let envelope = display_envelope(
            seed.operation_id.clone(),
            seed.correlation_id.clone(),
            seed.realm.clone(),
            seed.principal.clone(),
            seed.node.clone(),
            seed.workload.clone(),
            AuthzDecision::Deny,
        );
        self.deps.audit.record(GatewayAuditEvent {
            kind: GatewayAuditKind::DisplaySessionOpenDenied,
            envelope,
            session_id: None,
            state: None,
            error_slug: Some(err.slug()),
        })
    }

    fn audit_running(
        &self,
        seed: &ContextSeed,
        session_id: &DisplaySessionId,
    ) -> Result<(), GatewayError> {
        let envelope = display_envelope(
            seed.operation_id.clone(),
            seed.correlation_id.clone(),
            seed.realm.clone(),
            seed.principal.clone(),
            seed.node.clone(),
            seed.workload.clone(),
            AuthzDecision::Allow,
        );
        self.deps.audit.record(GatewayAuditEvent {
            kind: GatewayAuditKind::DisplaySessionRunning,
            envelope,
            session_id: Some(session_id.clone()),
            state: Some(SessionState::Running),
            error_slug: None,
        })
    }
}

/// The non-secret seed the caller supplies to [`GatewayOrchestrator::open`]
/// (the authorizing operation's realm/op/principal/workload).
#[derive(Debug, Clone)]
pub struct ContextSeed {
    /// Realm of the authorizing operation.
    pub realm: d2b_realm_core::RealmPath,
    /// Authorizing operation id.
    pub operation_id: d2b_realm_core::OperationId,
    /// Cross-realm correlation id shared across route and audit hops.
    pub correlation_id: d2b_realm_core::CorrelationId,
    /// Authorizing caller principal.
    pub principal: d2b_realm_core::PrincipalId,
    /// Gateway node handling the operation.
    pub node: d2b_realm_core::NodeId,
    /// Workload presenting the UI.
    pub workload: d2b_realm_core::WorkloadId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::SECRET_LEN;
    use d2b_realm_core::{
        CorrelationId, NodeId, OperationId, PrincipalId, RealmId, RealmPath, WorkloadId,
    };
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // ---- mock deps ----

    #[derive(Default)]
    struct MockWorkload {
        spawns: AtomicUsize,
        cleanups: AtomicUsize,
        fail_spawn: bool,
    }
    #[async_trait]
    impl GatewayWorkload for MockWorkload {
        async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail_spawn {
                return Err(GatewayError::ProviderAllocationFailed);
            }
            Ok(AgentHandle(format!("agent-{}", req.ctx.session_id)))
        }
        async fn cleanup(&self, _h: &AgentHandle) -> Result<(), GatewayError> {
            self.cleanups.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockListener {
        arms: AtomicUsize,
        closes: AtomicUsize,
        fail_handshake: bool,
    }
    #[async_trait]
    impl DisplayListener for MockListener {
        async fn arm(
            &self,
            ctx: &DisplaySessionContext,
            _b: &SessionBinding,
            _s: &SessionSecret,
        ) -> Result<ListenerHandle, GatewayError> {
            self.arms.fetch_add(1, Ordering::SeqCst);
            Ok(ListenerHandle(format!("lst-{}", ctx.session_id)))
        }
        async fn await_handshake(&self, _h: &ListenerHandle) -> Result<(), GatewayError> {
            if self.fail_handshake {
                return Err(GatewayError::DisplayAuthFailed(
                    crate::handshake::HandshakeError::BadMac,
                ));
            }
            Ok(())
        }
        async fn close(&self, _h: &ListenerHandle) -> Result<(), GatewayError> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FixedClock(u64);
    impl Clock for FixedClock {
        fn now_unix(&self) -> u64 {
            self.0
        }
    }

    struct SeqIds {
        n: AtomicU64,
    }
    impl IdSource for SeqIds {
        fn new_session_id(&self) -> DisplaySessionId {
            DisplaySessionId::new(format!("s{}", self.n.fetch_add(1, Ordering::SeqCst)))
        }
        fn new_secret(&self) -> SessionSecret {
            SessionSecret::from_bytes([3u8; SECRET_LEN])
        }
    }

    #[derive(Default)]
    struct RecordingAudit {
        events: Mutex<Vec<crate::GatewayAuditEvent>>,
    }
    impl GatewayAudit for RecordingAudit {
        fn record(&self, event: crate::GatewayAuditEvent) -> Result<(), GatewayError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    struct AuditRef(Arc<RecordingAudit>);
    impl GatewayAudit for AuditRef {
        fn record(&self, event: crate::GatewayAuditEvent) -> Result<(), GatewayError> {
            self.0.record(event)
        }
    }

    fn deps(workload: Arc<MockWorkload>, listener: Arc<MockListener>) -> GatewayDeps {
        deps_with_audit(workload, listener, None)
    }

    fn deps_with_audit(
        workload: Arc<MockWorkload>,
        listener: Arc<MockListener>,
        audit: Option<Arc<RecordingAudit>>,
    ) -> GatewayDeps {
        struct W(Arc<MockWorkload>);
        struct L(Arc<MockListener>);
        #[async_trait]
        impl GatewayWorkload for W {
            async fn spawn_agent(
                &self,
                req: &AgentSpawnRequest,
            ) -> Result<AgentHandle, GatewayError> {
                self.0.spawn_agent(req).await
            }
            async fn cleanup(&self, h: &AgentHandle) -> Result<(), GatewayError> {
                self.0.cleanup(h).await
            }
        }
        #[async_trait]
        impl DisplayListener for L {
            async fn arm(
                &self,
                ctx: &DisplaySessionContext,
                b: &SessionBinding,
                s: &SessionSecret,
            ) -> Result<ListenerHandle, GatewayError> {
                self.0.arm(ctx, b, s).await
            }
            async fn await_handshake(&self, h: &ListenerHandle) -> Result<(), GatewayError> {
                self.0.await_handshake(h).await
            }
            async fn close(&self, h: &ListenerHandle) -> Result<(), GatewayError> {
                self.0.close(h).await
            }
        }
        GatewayDeps {
            workload: Box::new(W(workload)),
            listener: Box::new(L(listener)),
            clock: Box::new(FixedClock(1000)),
            ids: Box::new(SeqIds {
                n: AtomicU64::new(0),
            }),
            audit: match audit {
                Some(a) => Box::new(AuditRef(a)) as Box<dyn GatewayAudit>,
                None => Box::new(crate::NoopGatewayAudit),
            },
        }
    }

    fn seed() -> (TargetKey, ContextSeed, AppCommand) {
        let realm = RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap();
        (
            TargetKey {
                realm: realm.target_form(),
                workload: "demo".into(),
            },
            ContextSeed {
                realm,
                operation_id: OperationId::parse("op-1").unwrap(),
                correlation_id: CorrelationId::parse("corr-1").unwrap(),
                principal: PrincipalId::parse("alice").unwrap(),
                node: NodeId::parse("gateway").unwrap(),
                workload: WorkloadId::parse("demo").unwrap(),
            },
            AppCommand::new(vec!["foot".into()]).unwrap(),
        )
    }

    #[tokio::test]
    async fn happy_path_opens_to_running() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w.clone(), l.clone()), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let open = orch.open(tk, cs, app, 42).await.unwrap();
        assert_eq!(orch.state(&open.session_id), Some(SessionState::Running));
        assert_eq!(w.spawns.load(Ordering::SeqCst), 1);
        assert_eq!(l.arms.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn list_sessions_returns_bounded_active_summaries() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w, l), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let open = orch.open(tk, cs, app, 42).await.unwrap();

        let sessions = orch.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let summary = &sessions[0];
        assert_eq!(summary.session_id, open.session_id);
        assert_eq!(summary.state, SessionState::Running);
        assert_eq!(summary.realm.target_form(), "work");
        assert_eq!(summary.workload.as_str(), "demo");
        assert_eq!(summary.operation_id.as_str(), "op-1");
        assert_eq!(summary.peer_principal.as_str(), "alice");
        let rendered = format!("{summary:?}");
        for forbidden in ["foot", "secret", "socket", "/run/", "waypipe"] {
            assert!(
                !rendered.contains(forbidden),
                "display session summary leaked {forbidden}: {rendered}"
            );
        }
    }

    #[tokio::test]
    async fn closed_sessions_drop_out_of_list() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w, l), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let open = orch.open(tk, cs, app, 42).await.unwrap();
        assert_eq!(orch.list_sessions().unwrap().len(), 1);
        orch.close(&open).await.unwrap();
        assert!(orch.list_sessions().unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_emits_redacted_typed_audit_events() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let audit = Arc::new(RecordingAudit::default());
        let orch = GatewayOrchestrator::new(
            deps_with_audit(w, l, Some(audit.clone())),
            1,
            LedgerLimits::default(),
        );
        let (tk, cs, app) = seed();
        let open = orch.open(tk, cs, app, 42).await.unwrap();
        let events = audit.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].kind,
            crate::GatewayAuditKind::DisplaySessionOpenAdmitted
        );
        assert_eq!(events[0].session_id.as_ref(), Some(&open.session_id));
        assert!(events[0].envelope.is_principal_consistent());
        assert_eq!(events[0].envelope.realm.target_form(), "work");
        assert_eq!(events[0].envelope.principal.as_str(), "alice");
        assert_eq!(
            events[1].kind,
            crate::GatewayAuditKind::DisplaySessionRunning
        );
        let rendered = format!("{events:?}");
        assert!(!rendered.contains("SharedAccessKey"));
        assert!(!rendered.contains("/run/"));
        assert!(!rendered.contains("foot"));
        assert!(!rendered.contains("wayland-bytes"));
    }

    #[tokio::test]
    async fn spawn_failure_compensates_and_fails_closed() {
        let w = Arc::new(MockWorkload {
            fail_spawn: true,
            ..Default::default()
        });
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w.clone(), l.clone()), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let err = orch.open(tk, cs, app, 42).await.unwrap_err();
        assert_eq!(err, GatewayError::ProviderAllocationFailed);
        // Listener is armed before the sender is spawned, then closed during
        // compensation when spawn fails.
        assert_eq!(l.arms.load(Ordering::SeqCst), 1);
        assert_eq!(l.closes.load(Ordering::SeqCst), 1);
        let id = DisplaySessionId::new("s0");
        assert_eq!(orch.state(&id), Some(SessionState::Failed));
    }

    #[tokio::test]
    async fn handshake_failure_fails_closed_no_bytes() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener {
            fail_handshake: true,
            ..Default::default()
        });
        let orch = GatewayOrchestrator::new(deps(w.clone(), l.clone()), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let err = orch.open(tk, cs, app, 42).await.unwrap_err();
        assert!(matches!(err, GatewayError::DisplayAuthFailed(_)));
        // The agent was spawned + listener armed, but never reached Running.
        let id = DisplaySessionId::new("s0");
        assert_eq!(orch.state(&id), Some(SessionState::Failed));
    }

    #[tokio::test]
    async fn close_tears_down_both_sides() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w.clone(), l.clone()), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let open = orch.open(tk, cs, app, 42).await.unwrap();
        orch.close(&open).await.unwrap();
        assert_eq!(orch.state(&open.session_id), Some(SessionState::Closed));
        assert_eq!(l.closes.load(Ordering::SeqCst), 1);
        assert_eq!(w.cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn idempotent_replay_returns_original_without_respawn() {
        let w = Arc::new(MockWorkload::default());
        let l = Arc::new(MockListener::default());
        let orch = GatewayOrchestrator::new(deps(w.clone(), l.clone()), 1, LedgerLimits::default());
        let (tk, cs, app) = seed();
        let first = orch
            .open(tk.clone(), cs.clone(), app.clone(), 42)
            .await
            .unwrap();
        // Same op + request hash: replay returns the original id, no new spawn.
        let again = orch.open(tk, cs, app, 42).await.unwrap();
        assert_eq!(again.session_id, first.session_id);
        assert_eq!(w.spawns.load(Ordering::SeqCst), 1); // not re-spawned
    }
}
