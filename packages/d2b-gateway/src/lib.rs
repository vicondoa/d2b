//! `d2b-gateway` — the gateway-mode display-session orchestrator
//! (ADR 0032, P0; panel-approved design `gw-display-design-r2`).
//!
//! The crate composes the proven providers into a **session-credential-bound**
//! display session. Control-plane authentication is exclusively supplied by
//! ComponentSession; Relay identity is reachability-only and never local
//! authorization.
//!
//! This module exposes the pure-logic, exhaustively-tested cores — the
//! The [`ledger`] (idempotency/quotas/state machine), redacted [`types`], and
//! [`error`] mapping are driven by the gateway-mode
//! `d2bd` op handler drives. The async orchestration (detached ACA exec,
//! the in-process relay-listener task, the operator display runner) is layered
//! on these in the daemon via injected dependencies, so this crate stays
//! unit-testable with no live Azure.

pub mod audit;
pub mod error;
pub mod ledger;
pub mod orchestrator;
pub mod types;

pub use audit::{
    GatewayAudit, GatewayAuditEvent, GatewayAuditKind, NoopGatewayAudit, display_envelope,
};
pub use error::GatewayError;
pub use ledger::{LedgerLimits, OpOutcome, SessionLedger, SessionRecord, SessionState, TargetKey};
pub use orchestrator::{
    AgentHandle, AgentSpawnRequest, Clock, ContextSeed, DEFAULT_SESSION_TTL_SECS, DisplayListener,
    DisplaySessionSummary, GatewayDeps, GatewayOrchestrator, GatewayWorkload, IdSource,
    ListenerHandle, OpenSession,
};
pub use types::{
    AppCommand, DisplaySessionContext, DisplaySessionId, DisplaySocket, SECRET_LEN, SessionBinding,
    SessionSecret,
};
