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