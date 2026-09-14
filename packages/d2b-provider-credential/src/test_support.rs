//! Re-usable recording test double for the Credential driver effect port.
//!
//! The daemon (`d2bd`) and this crate's own tests share one canonical
//! recording implementation of [`CredentialDriverEffects`] plus the
//! [`CredentialSession`] recording double, so the scripted provider facts,
//! lease facts, agent probe, and session generation binding cannot drift
//! between the owner crate and the plane.

use std::sync::Arc;

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::ResourceRef;
use parking_lot::Mutex;

use crate::driver::{CredentialDependencyFacts, CredentialDriverEffects, CredentialLeaseFacts};
use crate::session::{
    CredentialResourceRuntimeError, CredentialRevocationOutcome, CredentialRevocationRequest,
    CredentialSession,
};

/// The shared ordered log: every call the effect port, the manager endpoint,
/// and the session double perform lands here, so
/// revocation-before-child-deletion is observable as it happens.
pub type Log = Arc<Mutex<Vec<String>>>;

/// A fresh, empty shared log.
pub fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

/// Scripted effect port. Every call lands in the shared ordered log so
/// revocation-before-child-deletion is observable.
pub struct FakeEffects {
    log: Log,
    facts: Mutex<Option<CredentialDependencyFacts>>,
    lease: Mutex<Option<CredentialLeaseFacts>>,
    agent_ready: Mutex<bool>,
    /// The bound session (called-into directly by tests that assert on the
    /// field itself; script through [`FakeEffects::set_session`] otherwise).
    pub session: Mutex<Option<Arc<dyn CredentialSession>>>,
}

impl FakeEffects {
    /// A fresh scripted double sharing `log`, with the defaults the tests'
    /// fixtures rely on: provider and execution ready, no lease facts, agent
    /// ready, and a live session bound to generation 7.
    pub fn new(log: Log) -> Arc<Self> {
        Arc::new(Self {
            log,
            facts: Mutex::new(Some(facts(true, true))),
            lease: Mutex::new(None),
            agent_ready: Mutex::new(true),
            session: Mutex::new(Some(Arc::new(RecordingSession::new(Some(7))))),
        })
    }

    /// The calls recorded so far, in order (the shared log also carries the
    /// manager endpoint's `ensure`/`get`/`delete`/`list-owned` entries).
    pub fn call_order(&self) -> Vec<String> {
        self.log.lock().clone()
    }

    /// Script the Provider + execution-target facts.
    pub fn set_facts(&self, value: Option<CredentialDependencyFacts>) {
        *self.facts.lock() = value;
    }

    /// Script the provider-side lease facts.
    pub fn set_lease(&self, value: Option<CredentialLeaseFacts>) {
        *self.lease.lock() = value;
    }

    /// Script whether the managed-identity agent Process is live.
    pub fn set_agent_ready(&self, value: bool) {
        *self.agent_ready.lock() = value;
    }

    /// Script the session the delete path binds for the revocation call.
    pub fn set_session(&self, value: Option<Arc<dyn CredentialSession>>) {
        *self.session.lock() = value;
    }
}

#[async_trait::async_trait]
impl CredentialDriverEffects for FakeEffects {
    async fn dependency_facts(
        &self,
        _provider_ref: &ResourceRef,
        _execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts> {
        self.log.lock().push("dependency-facts".to_owned());
        self.facts.lock().clone()
    }

    async fn lease_facts(&self, _credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        self.log.lock().push("lease-facts".to_owned());
        *self.lease.lock()
    }

    async fn agent_ready(&self, _agent_ref: &ResourceRef) -> bool {
        self.log.lock().push("agent-ready".to_owned());
        *self.agent_ready.lock()
    }

    fn session(&self, _provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
        self.log.lock().push("session".to_owned());
        self.session.lock().clone()
    }
}

/// The old runner's dependency snapshot facts for one Credential, scripted
/// with the desired Provider and execution-target readiness.
pub fn facts(provider_ready: bool, execution_ready: bool) -> CredentialDependencyFacts {
    CredentialDependencyFacts {
        provider_uid: "223e4567-e89b-42d3-a456-426614174000".to_owned(),
        provider_generation: 1,
        provider_ready,
        execution_ready,
    }
}

/// Session double that binds the generation exactly like the real
/// `ComponentCredentialSession`: a request carrying a different
/// generation is `Uncertain`, never `Revoked`.
pub struct RecordingSession {
    generation: Option<ReconnectGeneration>,
    operations: Mutex<Vec<String>>,
}

impl RecordingSession {
    /// A session bound to `generation` (or unbound when `None`).
    pub fn new(generation: Option<u64>) -> Self {
        Self {
            generation: generation.map(|value| ReconnectGeneration::new(value).unwrap()),
            operations: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl CredentialSession for RecordingSession {
    fn session_generation(&self) -> Option<ReconnectGeneration> {
        self.generation
    }

    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
        if Some(request.session_generation()) != self.generation {
            return Ok(CredentialRevocationOutcome::Uncertain);
        }
        let mut operations = self.operations.lock();
        let operation_id = request.operation_id().to_owned();
        if operations.contains(&operation_id) {
            return Ok(CredentialRevocationOutcome::AlreadyRevoked);
        }
        operations.push(operation_id);
        Ok(CredentialRevocationOutcome::Revoked)
    }
}
