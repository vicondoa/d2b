//! `nixling-gateway` — the gateway-mode display-session orchestrator
//! (ADR 0032, P0; panel-approved design `gw-display-design-r2`).
//!
//! The crate composes the proven providers into a **session-credential-bound**
//! display session: a Wayland app inside an ACA sandbox is rendered on the
//! operator's compositor over Azure Relay, where Relay and the sandbox managed
//! identity are **reachability-only** and the display bytes are admitted only
//! by a gateway-minted one-shot HMAC handshake ([`handshake`]) verified before
//! a single byte reaches Waypipe.
//!
//! This module exposes the pure-logic, exhaustively-tested cores — the
//! [`handshake`] credential, the [`ledger`] (idempotency/quotas/state machine),
//! the redacted [`types`], and the [`error`] mapping — that the gateway-mode
//! `nixlingd` op handler drives. The async orchestration (detached ACA exec,
//! the in-process relay-listener task, the operator display runner) is layered
//! on these in the daemon via injected dependencies, so this crate stays
//! unit-testable with no live Azure.

pub mod error;
pub mod handshake;
pub mod ledger;
pub mod types;

pub use error::GatewayError;
pub use handshake::{
    DisplaySessionId, Handshake, HandshakeError, ReplayGuard, SessionBinding, SessionSecret,
    SetReplayGuard,
};
pub use ledger::{LedgerLimits, OpOutcome, SessionLedger, SessionRecord, SessionState, TargetKey};
pub use types::{AppCommand, DisplaySessionContext, DisplaySocket};
