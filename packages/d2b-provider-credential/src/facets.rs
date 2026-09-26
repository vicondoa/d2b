//! The declared facets the provider-owned Credential effects reach daemon
//! state through (U8).
//!
//! The Credential family's driver effects are served by this crate's own
//! implementation ([`crate::effects_service`]). The daemon state that
//! implementation holds - the preserved Provider and execution-target
//! reads, the lease-facts read, the managed-identity agent probe, and the
//! authenticated Provider session handoff registry - crosses the provider
//! boundary as declared facets rather than as a daemon handle: every facet
//! here is a type this crate declares, an implementation of it is supplied
//! by the daemon host through the composition root (never derived from
//! caller input), and the family crate holds no daemon state type.
//!
//! Credential material never crosses this boundary: the runtime facet
//! answers typed facts (Provider readiness, lease presence, agent
//! liveness) and hands back the same [`CredentialSession`] objects the
//! daemon's ProviderSupervisor handoff registry holds, so the revocation
//! call stays on the daemon-supplied authenticated session surface. No
//! secret, lease handle, or raw credential byte moves to a new surface.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::{CredentialDependencyFacts, CredentialLeaseFacts};
use crate::session::{CredentialResourceRuntimeError, CredentialSession};

/// Boxed future returned by one production dependency probe: resolving the
/// Provider and target rows is store-backed, so the facet cannot be a sync
/// closure. The read fails as [`CredentialResourceRuntimeError`] rather
/// than reporting absence when the manager RPC itself failed.
pub type DependencyFactsFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<Option<CredentialDependencyFacts>, CredentialResourceRuntimeError>>
            + Send
            + 'a,
    >,
>;

/// Boxed future of one production lease-fact read.
pub type LeaseFactsFuture<'a> =
    Pin<Box<dyn Future<Output = Option<CredentialLeaseFacts>> + Send + 'a>>;

/// Boxed future of one production agent-readiness probe.
pub type AgentReadyFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// The daemon-supplied facet set the provider-owned Credential effects are
/// built from (U8).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct CredentialEffectFacets {
    /// The daemon's Credential runtime: the preserved Provider and
    /// execution-target reads, the lease-facts read, the managed-identity
    /// agent probe, and the authenticated Provider session handoff
    /// registry, supplied through the composition root.
    pub runtime: Arc<dyn CredentialRuntime>,
}

/// The daemon-hosted Credential runtime one zone's effects run over (U8).
///
/// The daemon implements this trait in its composition root (the same
/// preserved reads and session registry the retired adapter held), and the
/// family crate's effects service delegates every driver seam to it. The
/// runtime answers typed facts only; credential material stays on the
/// daemon-supplied authenticated session surface.
#[async_trait]
pub trait CredentialRuntime: Send + Sync + 'static {
    /// Provider + execution-target facts. `Ok(None)` when the Provider row
    /// is not observable to this daemon (deletion still fails closed rather
    /// than guessing); `Err` when the dependency read itself failed (a
    /// manager RPC failure), so absence is never answered for a failed
    /// read.
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<Option<CredentialDependencyFacts>, CredentialResourceRuntimeError>;

    /// Provider-side lease facts for one Credential row.
    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts>;

    /// Whether the managed-identity agent Process is live (target-local
    /// evidence behind the facet, like the binding socket probe).
    async fn agent_ready(&self, agent_ref: &ResourceRef) -> bool;

    /// The authenticated Provider session for one Credential Provider
    /// (R28). The session owns the exact generation binding; `None` means
    /// no session surface exists at all and revocation fails closed.
    fn session(&self, provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>>;
}