//! The gateway session ledger: the accepted-op idempotency/dedup table, the
//! per-realm/principal quotas, and the display-session lifecycle state machine
//! (ADR 0032 P0, design §3). Pure logic — no I/O — so it is exhaustively
//! unit-testable without live providers.

use crate::error::GatewayError;
use crate::handshake::DisplaySessionId;
use std::collections::HashMap;

/// The display-session lifecycle states (design §3 state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    /// Minting the one-shot session credential.
    Minting,
    /// Spawning the in-sandbox agent (detached ACA exec).
    AgentSpawning,
    /// Arming the host relay listener task.
    ListenerArming,
    /// Listener up; awaiting the agent's verified handshake.
    AwaitingHandshake,
    /// Handshake verified; display bytes flowing.
    Running,
    /// Listener/relay dropped; eligible for reopen.
    Degraded,
    /// Tearing down (compensating cleanup running).
    Stopping,
    /// Terminal: cleaned up.
    Closed,
    /// Terminal: failed before Running.
    Failed,
}

impl SessionState {
    /// Whether a transition `self -> next` is legal. Fail-closed: any edge not
    /// listed is rejected.
    pub fn can_transition(self, next: SessionState) -> bool {
        use SessionState::*;
        matches!(
            (self, next),
            (Minting, ListenerArming)
                | (ListenerArming, AgentSpawning)
                | (AgentSpawning, AwaitingHandshake)
                | (AwaitingHandshake, Running)
                | (Running, Degraded)
                | (Degraded, ListenerArming)
                // close path from any non-terminal state
                | (Minting, Stopping)
                | (AgentSpawning, Stopping)
                | (ListenerArming, Stopping)
                | (AwaitingHandshake, Stopping)
                | (Running, Stopping)
                | (Degraded, Stopping)
                | (Stopping, Closed)
                // failure path from any pre-Running state
                | (Minting, Failed)
                | (AgentSpawning, Failed)
                | (ListenerArming, Failed)
                | (AwaitingHandshake, Failed)
        )
    }

    /// Whether this is a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(self, SessionState::Closed | SessionState::Failed)
    }
}

/// The identity of a display target: a session is unique per `(realm,
/// workload)` (the single-session cap).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetKey {
    /// Realm target form (most-specific-first).
    pub realm: String,
    /// Workload id.
    pub workload: String,
}

/// Quota limits enforced by the ledger (the in-process accounting; the
/// gateway-daemon cgroup is the hard backstop — design §3).
#[derive(Debug, Clone)]
pub struct LedgerLimits {
    /// Max concurrent (non-terminal) sessions per principal.
    pub max_sessions_per_principal: usize,
    /// Max concurrent (non-terminal) sessions per realm.
    pub max_sessions_per_realm: usize,
    /// Max total tracked records (records bound — old terminal records are
    /// GC'd first; if still over, opens fail closed).
    pub max_records: usize,
}

impl Default for LedgerLimits {
    fn default() -> Self {
        Self {
            max_sessions_per_principal: 8,
            max_sessions_per_realm: 16,
            max_records: 256,
        }
    }
}

/// A tracked session record (redacted; carries no secret).
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Opaque session id.
    pub id: DisplaySessionId,
    /// Target.
    pub target: TargetKey,
    /// Authorizing principal.
    pub principal: String,
    /// Authorizing operation id (idempotency key).
    pub operation_id: String,
    /// A hash of the authorizing request (to distinguish replay from conflict).
    pub request_hash: u64,
    /// Current state.
    pub state: SessionState,
    /// Gateway generation that owns this record.
    pub generation: u64,
}

/// The outcome of an `open` admission decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpOutcome {
    /// A fresh session was admitted with this id.
    Accepted(DisplaySessionId),
    /// An idempotent replay of the same operation: the original session id.
    Replay(DisplaySessionId),
}

/// The gateway session ledger.
#[derive(Debug)]
pub struct SessionLedger {
    limits: LedgerLimits,
    generation: u64,
    records: HashMap<String, SessionRecord>, // by session id
}

impl SessionLedger {
    /// A ledger owned by gateway `generation`.
    pub fn new(generation: u64, limits: LedgerLimits) -> Self {
        Self {
            limits,
            generation,
            records: HashMap::new(),
        }
    }

    /// The owning generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn active_for_principal(&self, principal: &str) -> usize {
        self.records
            .values()
            .filter(|r| r.principal == principal && !r.state.is_terminal())
            .count()
    }

    fn active_for_realm(&self, realm: &str) -> usize {
        self.records
            .values()
            .filter(|r| r.target.realm == realm && !r.state.is_terminal())
            .count()
    }

    /// Admit (or idempotently replay) an `open`. `new_id` is the id to assign
    /// if a fresh session is accepted. Fail-closed on quota/busy/conflict.
    pub fn open(
        &mut self,
        target: TargetKey,
        principal: &str,
        operation_id: &str,
        request_hash: u64,
        new_id: DisplaySessionId,
    ) -> Result<OpOutcome, GatewayError> {
        // Idempotency: same operation id already tracked.
        if let Some(existing) = self
            .records
            .values()
            .find(|r| r.operation_id == operation_id && r.principal == principal)
        {
            if existing.request_hash == request_hash {
                return Ok(OpOutcome::Replay(existing.id.clone()));
            }
            // Same key, different request = non-retryable conflict (fail closed).
            return Err(GatewayError::Conflict);
        }
        // Single-session cap: one live session per (realm, workload).
        if self
            .records
            .values()
            .any(|r| r.target == target && !r.state.is_terminal())
        {
            return Err(GatewayError::Busy);
        }
        // GC terminal records if we're at the record ceiling.
        if self.records.len() >= self.limits.max_records {
            self.gc_terminal();
            if self.records.len() >= self.limits.max_records {
                return Err(GatewayError::QuotaExceeded);
            }
        }
        // Concurrency quotas.
        if self.active_for_principal(principal) >= self.limits.max_sessions_per_principal
            || self.active_for_realm(&target.realm) >= self.limits.max_sessions_per_realm
        {
            return Err(GatewayError::QuotaExceeded);
        }
        // Fail closed on an id-source collision: never overwrite a tracked
        // record (which would silently drop its state + quota accounting).
        if self.records.contains_key(new_id.as_str()) {
            return Err(GatewayError::Internal);
        }
        let rec = SessionRecord {
            id: new_id.clone(),
            target,
            principal: principal.to_owned(),
            operation_id: operation_id.to_owned(),
            request_hash,
            state: SessionState::Minting,
            generation: self.generation,
        };
        self.records.insert(new_id.as_str().to_owned(), rec);
        Ok(OpOutcome::Accepted(new_id))
    }

    /// Transition a session to `next`; fail-closed on an illegal edge or an
    /// unknown session.
    pub fn transition(
        &mut self,
        id: &DisplaySessionId,
        next: SessionState,
    ) -> Result<(), GatewayError> {
        let rec = self
            .records
            .get_mut(id.as_str())
            .ok_or(GatewayError::Cancelled)?;
        if !rec.state.can_transition(next) {
            return Err(GatewayError::Cancelled);
        }
        rec.state = next;
        Ok(())
    }

    /// The current state of a session, if tracked.
    pub fn state(&self, id: &DisplaySessionId) -> Option<SessionState> {
        self.records.get(id.as_str()).map(|r| r.state)
    }

    /// All non-terminal records (for `vm display list`).
    pub fn active(&self) -> Vec<&SessionRecord> {
        self.records
            .values()
            .filter(|r| !r.state.is_terminal())
            .collect()
    }

    /// Drop terminal records to bound memory.
    pub fn gc_terminal(&mut self) {
        self.records.retain(|_, r| !r.state.is_terminal());
    }

    /// On gateway restart: bump the generation and mark every prior-generation
    /// record `Closed` (its in-process state is gone; the §2 generation check
    /// rejects any sandbox survivor). Returns the reconciled records so the
    /// caller can run compensating cleanup (ACA agent kill, socket unlink).
    pub fn reconcile_restart(&mut self) -> Vec<SessionRecord> {
        self.generation += 1;
        let mut reconciled = Vec::new();
        for rec in self.records.values_mut() {
            if !rec.state.is_terminal() {
                rec.state = SessionState::Closed;
                reconciled.push(rec.clone());
            }
        }
        reconciled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(realm: &str, wl: &str) -> TargetKey {
        TargetKey {
            realm: realm.into(),
            workload: wl.into(),
        }
    }
    fn id(s: &str) -> DisplaySessionId {
        DisplaySessionId::new(s)
    }

    #[test]
    fn fresh_open_is_accepted() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        let out = l
            .open(key("work", "demo"), "alice", "op-1", 42, id("s1"))
            .unwrap();
        assert_eq!(out, OpOutcome::Accepted(id("s1")));
        assert_eq!(l.state(&id("s1")), Some(SessionState::Minting));
    }

    #[test]
    fn same_op_same_request_is_idempotent_replay() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 42, id("s1"))
            .unwrap();
        let out = l
            .open(key("work", "demo"), "alice", "op-1", 42, id("s2"))
            .unwrap();
        assert_eq!(out, OpOutcome::Replay(id("s1"))); // original id, not s2
    }

    #[test]
    fn same_op_different_request_is_conflict() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 42, id("s1"))
            .unwrap();
        let err = l
            .open(key("work", "demo"), "alice", "op-1", 99, id("s2"))
            .unwrap_err();
        assert_eq!(err, GatewayError::Conflict);
    }

    #[test]
    fn id_source_collision_fails_closed() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "a"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        // A different op/target but a colliding session id must fail closed,
        // not overwrite the live record.
        let err = l
            .open(key("work", "b"), "bob", "op-2", 2, id("s1"))
            .unwrap_err();
        assert_eq!(err, GatewayError::Internal);
        assert_eq!(l.state(&id("s1")), Some(SessionState::Minting)); // original intact
    }

    #[test]
    fn per_realm_quota_fails_closed() {
        let limits = LedgerLimits {
            max_sessions_per_realm: 1,
            ..LedgerLimits::default()
        };
        let mut l = SessionLedger::new(1, limits);
        l.open(key("work", "a"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        // Different principal + workload, same realm: still capped.
        let err = l
            .open(key("work", "b"), "bob", "op-2", 2, id("s2"))
            .unwrap_err();
        assert_eq!(err, GatewayError::QuotaExceeded);
    }

    #[test]
    fn record_ceiling_gcs_terminal_then_fails_closed_when_all_active() {
        let limits = LedgerLimits {
            max_records: 1,
            max_sessions_per_principal: 8,
            max_sessions_per_realm: 16,
        };
        let mut l = SessionLedger::new(1, limits);
        l.open(key("work", "a"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        // Ceiling reached with an ACTIVE record: a new open fails closed.
        assert_eq!(
            l.open(key("work", "b"), "alice", "op-2", 2, id("s2")),
            Err(GatewayError::QuotaExceeded)
        );
        // Close s1 -> it becomes terminal; the next open GCs it and succeeds.
        l.transition(&id("s1"), SessionState::Stopping).unwrap();
        l.transition(&id("s1"), SessionState::Closed).unwrap();
        let out = l
            .open(key("work", "b"), "alice", "op-3", 3, id("s3"))
            .unwrap();
        assert_eq!(out, OpOutcome::Accepted(id("s3")));
    }

    #[test]
    fn transition_matrix_is_fail_closed() {
        use SessionState::*;
        let legal = [
            (Minting, ListenerArming),
            (ListenerArming, AgentSpawning),
            (AgentSpawning, AwaitingHandshake),
            (AwaitingHandshake, Running),
            (Running, Degraded),
            (Degraded, ListenerArming),
            (Minting, Stopping),
            (AgentSpawning, Stopping),
            (ListenerArming, Stopping),
            (AwaitingHandshake, Stopping),
            (Running, Stopping),
            (Degraded, Stopping),
            (Stopping, Closed),
            (Minting, Failed),
            (AgentSpawning, Failed),
            (ListenerArming, Failed),
            (AwaitingHandshake, Failed),
        ];
        let all = [
            Minting,
            AgentSpawning,
            ListenerArming,
            AwaitingHandshake,
            Running,
            Degraded,
            Stopping,
            Closed,
            Failed,
        ];
        for &from in &all {
            for &to in &all {
                let expect = legal.contains(&(from, to));
                assert_eq!(
                    from.can_transition(to),
                    expect,
                    "edge {from:?} -> {to:?} legality"
                );
            }
            // Terminal states never transition anywhere.
            if from.is_terminal() {
                for &to in &all {
                    assert!(!from.can_transition(to), "terminal {from:?} must not move");
                }
            }
        }
    }

    #[test]
    fn second_session_for_same_target_is_busy() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        let err = l
            .open(key("work", "demo"), "bob", "op-2", 2, id("s2"))
            .unwrap_err();
        assert_eq!(err, GatewayError::Busy);
    }

    #[test]
    fn closing_frees_the_target_slot() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        l.transition(&id("s1"), SessionState::Stopping).unwrap();
        l.transition(&id("s1"), SessionState::Closed).unwrap();
        // Target free again.
        let out = l
            .open(key("work", "demo"), "bob", "op-2", 2, id("s2"))
            .unwrap();
        assert_eq!(out, OpOutcome::Accepted(id("s2")));
    }

    #[test]
    fn per_principal_quota_fails_closed() {
        let limits = LedgerLimits {
            max_sessions_per_principal: 1,
            ..LedgerLimits::default()
        };
        let mut l = SessionLedger::new(1, limits);
        l.open(key("work", "a"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        let err = l
            .open(key("work", "b"), "alice", "op-2", 2, id("s2"))
            .unwrap_err();
        assert_eq!(err, GatewayError::QuotaExceeded);
    }

    #[test]
    fn happy_path_transitions_legal() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        for s in [
            SessionState::ListenerArming,
            SessionState::AgentSpawning,
            SessionState::AwaitingHandshake,
            SessionState::Running,
        ] {
            l.transition(&id("s1"), s).unwrap();
        }
        assert_eq!(l.state(&id("s1")), Some(SessionState::Running));
    }

    #[test]
    fn illegal_transition_fails_closed() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        // Minting -> Running is not a legal edge.
        assert!(l.transition(&id("s1"), SessionState::Running).is_err());
    }

    #[test]
    fn degraded_can_reopen_then_run() {
        let mut l = SessionLedger::new(1, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        for s in [
            SessionState::ListenerArming,
            SessionState::AgentSpawning,
            SessionState::AwaitingHandshake,
            SessionState::Running,
            SessionState::Degraded,
            SessionState::ListenerArming,
            SessionState::AgentSpawning,
            SessionState::AwaitingHandshake,
            SessionState::Running,
        ] {
            l.transition(&id("s1"), s).unwrap();
        }
        assert_eq!(l.state(&id("s1")), Some(SessionState::Running));
    }

    #[test]
    fn reconcile_restart_bumps_generation_and_closes_active() {
        let mut l = SessionLedger::new(5, LedgerLimits::default());
        l.open(key("work", "demo"), "alice", "op-1", 1, id("s1"))
            .unwrap();
        l.transition(&id("s1"), SessionState::ListenerArming)
            .unwrap();
        let reconciled = l.reconcile_restart();
        assert_eq!(l.generation(), 6);
        assert_eq!(reconciled.len(), 1);
        assert_eq!(l.state(&id("s1")), Some(SessionState::Closed));
    }
}
