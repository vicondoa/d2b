//! Daemon-side Credential family effects.
//!
//! The Credential family crate owns the driver and its effect port; this
//! module implements that port over the daemon's preserved provider reads and
//! the ProviderSupervisor session handoff registry. The composition unit
//! supplies the closures (the inputs the old `start_u10_controller_runners`
//! assembled) and the daemon's session registry, so the family never reaches
//! the daemon's runtime and the daemon keeps the host state.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_credential::{
    CredentialDependencyFacts, CredentialDriverEffects, CredentialLeaseFacts, CredentialSession,
};

/// Boxed future returned by one production dependency probe: resolving the
/// Provider and target rows is store-backed (the old dependency snapshots
/// were assembled from the same reads), so the port cannot be a sync
/// closure.
pub(crate) type DependencyFactsFuture<'a> =
    Pin<Box<dyn Future<Output = Option<CredentialDependencyFacts>> + Send + 'a>>;

/// Boxed credential dependency-facts probe closure (one Provider/target
/// pair).
pub(crate) type DependencyFactsEffect = Arc<
    dyn for<'a> Fn(&'a ResourceRef, &'a ResourceRef) -> DependencyFactsFuture<'a> + Send + Sync,
>;

/// Boxed future of one production lease-fact read.
pub(crate) type LeaseFactsFuture<'a> =
    Pin<Box<dyn Future<Output = Option<CredentialLeaseFacts>> + Send + 'a>>;

/// Boxed future of one production agent-readiness probe.
pub(crate) type AgentReadyFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// The production effects over the preserved provider reads and the
/// ProviderSupervisor handoff registry.
pub(crate) struct ProductionCredentialDriverEffects {
    facts: DependencyFactsEffect,
    lease: Arc<dyn for<'a> Fn(&'a ResourceRef) -> LeaseFactsFuture<'a> + Send + Sync>,
    agent: Arc<dyn for<'a> Fn(&'a ResourceRef) -> AgentReadyFuture<'a> + Send + Sync>,
    sessions: crate::credential_resource_runtime::CredentialSessionRegistry,
}

impl ProductionCredentialDriverEffects {
    pub(crate) fn new(
        facts: DependencyFactsEffect,
        lease: Arc<dyn for<'a> Fn(&'a ResourceRef) -> LeaseFactsFuture<'a> + Send + Sync>,
        agent: Arc<dyn for<'a> Fn(&'a ResourceRef) -> AgentReadyFuture<'a> + Send + Sync>,
        sessions: crate::credential_resource_runtime::CredentialSessionRegistry,
    ) -> Self {
        Self {
            facts,
            lease,
            agent,
            sessions,
        }
    }
}

#[async_trait::async_trait]
impl CredentialDriverEffects for ProductionCredentialDriverEffects {
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts> {
        (self.facts)(provider_ref, execution_ref).await
    }

    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        (self.lease)(credential_ref).await
    }

    async fn agent_ready(&self, agent_ref: &ResourceRef) -> bool {
        (self.agent)(agent_ref).await
    }

    fn session(&self, provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
        Some(self.sessions.for_provider(provider_ref.clone()))
    }
}
