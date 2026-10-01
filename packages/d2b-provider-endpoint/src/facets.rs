//! The declared facets the provider-owned Endpoint effects service reaches
//! daemon state through (U6).
//!
//! The Endpoint family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the preserved
//! endpoint realization. The realization splits into three daemon-owned
//! surfaces, and each crosses the provider boundary as a declared facet
//! rather than a daemon call: the host socket effect for the binding-owned
//! virtiofsd socket (resolve the producer's private socket target and probe
//! or mutate the bound socket on the host target), and the two
//! row-evidence probes (the guest-runtime control endpoints read the
//! guest's committed VMM Process row; the device-worker endpoints read the
//! producer worker Process row). The purpose derivations that classify one
//! purpose onto those surfaces are this crate's own knowledge (see
//! [`crate::effects_service`]), so the facets never see a purpose decision.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    EndpointAccessRequest, EndpointAccessResponse, EndpointAccessVerb,
};
use d2b_contracts_resource::v3::ResourceRef;

/// The daemon-supplied facet set the provider-owned Endpoint effects are
/// built from (U6).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The three facets are the host socket surface and
/// the two row-evidence probes the preserved realization owns; every other
/// input (the purpose derivations and the dispatch onto the surfaces) is
/// this crate's own knowledge.
#[derive(Clone)]
pub struct EndpointEffectFacets {
    /// The host socket surface: the binding-owned virtiofsd socket's
    /// presence, ensure, and removal over the daemon's socket target
    /// registry and runtime directory.
    pub socket: Arc<dyn EndpointSocketSource>,
    /// The guest-runtime control evidence: whether the producer Guest's
    /// committed VMM Process row reports `Ready` at its current generation.
    pub guest_vmm: Arc<dyn GuestVmmEvidenceSource>,
    /// The device-worker evidence: whether the producer worker Process row
    /// reports `Ready` at its current generation.
    pub device_worker: Arc<dyn DeviceWorkerEvidenceSource>,
}

/// The daemon-supplied host socket surface (U6): the binding-owned
/// virtiofsd socket's presence, ensure, and removal over the daemon's
/// socket target registry and runtime directory.
#[async_trait]
pub trait EndpointSocketSource: Send + Sync + 'static {
    /// Whether the producer's socket is resolved and bound on the host
    /// target.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool;

    /// Realize the producer's socket: wait a bounded budget for the worker
    /// Process child's bind and report a retryable failure otherwise (the
    /// actor owns the retry, R13).
    async fn ensure(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String>;

    /// Remove the producer's socket - endpoint-first teardown, idempotent
    /// under retry (R10).
    async fn remove(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String>;
}

/// The daemon-supplied guest-runtime control evidence (U6): whether the
/// producer Guest's committed VMM Process row reports `Ready` at its
/// current generation.
///
/// The guest's nested VMM carries both private rendezvous - the Cloud
/// Hypervisor API socket and the authenticated guest-control session - and
/// the guest's committed VMM Process row (`Process/<guest>-vmm`) reports
/// `Ready` exactly while the launch that carries them is live.
#[async_trait]
pub trait GuestVmmEvidenceSource: Send + Sync + 'static {
    /// Whether the producer's evidence row reports `Ready`.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool;
}

/// The daemon-supplied device-worker evidence (U6): whether the producer
/// worker Process row reports `Ready` at its current generation.
///
/// One swtpm launch composes both sockets (`--server` and `--ctrl` of the
/// same argv) and the declaring Device TPM Provider's worker Process row
/// reports `Ready` exactly while that launch is live, so the producer row
/// is the evidence row.
#[async_trait]
pub trait DeviceWorkerEvidenceSource: Send + Sync + 'static {
    /// Whether the producer's evidence row reports `Ready`.
    async fn present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool;
}

/// Why one exact-endpoint dispatch did not answer.
///
/// The closed split is the one the driver needs: a broker refusal names a
/// condition the committed row cannot fix, while a transport that never
/// answered says nothing about the relationship and is retried. Neither
/// carries a host path, a socket name, or a numerical principal, so a refusal
/// reads the same wherever it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointAccessDispatchError {
    /// The broker refused the request before or instead of applying it. The
    /// slug is the wire's own closed refusal class.
    Refused(String),
    /// The privileged leg did not answer at all.
    ///
    /// An absent or unreadable answer is never an absence of effect: the
    /// relationship stays outstanding and the pass retries (R13).
    Unavailable(String),
}

impl EndpointAccessDispatchError {
    /// The closed, path-free slug a refusal reports under.
    pub fn code(&self) -> &str {
        match self {
            Self::Refused(code) => code,
            Self::Unavailable(_) => "endpoint-access-dispatch-unavailable",
        }
    }
}

impl core::fmt::Display for EndpointAccessDispatchError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for EndpointAccessDispatchError {}

/// The privileged exact-endpoint ACL dispatch one committed relationship's
/// delivery rides (U18, R23).
///
/// The daemon implements this facet over its authenticated broker socket; this
/// crate's serving driver builds the typed request and reconciles the answer
/// and holds no socket, no path, and no numerical host principal. The broker
/// re-derives the consumer principal from the verified Zone bundle and
/// recomputes the authority binding before it touches an ACL entry, so a
/// request that arrives here is a claim to be checked rather than a grant.
#[async_trait]
pub trait EndpointAccessDispatch: Send + Sync + 'static {
    /// Send one exact-endpoint request and return the broker's own answer.
    async fn dispatch(
        &self,
        verb: EndpointAccessVerb,
        request: EndpointAccessRequest,
    ) -> Result<EndpointAccessResponse, EndpointAccessDispatchError>;
}

/// The dispatch a plane uses when it carries no daemon to reach a broker.
///
/// The privileged leg is a daemon-only capability: only the daemon holds the
/// broker socket and the caller role the request is dispatched under. A plane
/// built without one therefore cannot deliver anything, and says so by name
/// rather than pretending a delivery happened. This grants no authority - it
/// refuses every verb, and the driver reports the relationship undelivered.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredEndpointAccess;

#[async_trait::async_trait]
impl EndpointAccessDispatch for UnwiredEndpointAccess {
    async fn dispatch(
        &self,
        verb: EndpointAccessVerb,
        _request: EndpointAccessRequest,
    ) -> Result<EndpointAccessResponse, EndpointAccessDispatchError> {
        Err(EndpointAccessDispatchError::Unavailable(format!(
            "the exact-endpoint dispatch is daemon-only and this plane has no broker socket to \
             reach it for {}",
            verb.as_str()
        )))
    }
}