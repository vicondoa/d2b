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

// ---------------------------------------------------------------------------
// The `EndpointBinding` family's effects (U18)
// ---------------------------------------------------------------------------

/// The daemon-supplied facet set the provider-owned `EndpointBinding` effects
/// are built from (U18).
///
/// Four surfaces, each one a live-host fact this crate cannot read for
/// itself: the endpoint owner's PRIVATE resolution of the exact endpoint's
/// host path, the effective-access observation for one consumer principal,
/// the grant of the admitted right on the exact inode, and the revoke of
/// that grant. The composition root supplies the objects; the driver never
/// holds a daemon state type (R2) and never sees a host path or a numerical
/// principal.
#[derive(Clone)]
pub struct EndpointBindingEffectFacets {
    /// The endpoint owner's private locator resolution.
    pub locator: Arc<dyn EndpointLocatorSource>,
    /// The effective-access observation the exact-endpoint path re-reads.
    pub access: Arc<dyn EndpointAccessSource>,
    /// The grant of the admitted right on the exact endpoint inode.
    pub grant: Arc<dyn EndpointGrantSource>,
    /// The revoke of that grant: the fence against NEW use, and the release.
    pub revoke: Arc<dyn EndpointRevokeSource>,
}

/// The endpoint owner's PRIVATE resolution of one exact endpoint (U18).
///
/// `EndpointSpec` is locator-free by contract, so the exact socket's location
/// is not a committed fact anywhere in the graph: it is a property of the
/// producing launch, and only the daemon that launched it knows it. This
/// facet is that knowledge, asked as a verdict so the answer rather than the
/// path is what crosses the provider boundary - and so the driver holds no
/// derivation of its own, because a second derivation here would be a second
/// locator that could disagree with the producer's.
///
/// Every facet below takes the committed `Endpoint` reference together with
/// the bounded purpose its own spec publishes, and the consumer's reference.
/// Those three are what the daemon resolves into a host path and a host
/// principal, and they are all the driver ever holds.
#[async_trait]
pub trait EndpointLocatorSource: Send + Sync + 'static {
    /// Whether this daemon has privately realized the exact endpoint.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the resolution itself failed. An endpoint this
    /// daemon never realized answers `Ok(false)`: the producing launch has
    /// not bound it yet, which is not an error and is not a refusal.
    async fn realized(&self, endpoint: &ResourceRef, purpose: &str) -> Result<bool, String>;
}

/// What the kernel actually applies to one consumer principal for one exact
/// endpoint (U18).
///
/// The observation is EFFECTIVE, not a presence check: a named ACL entry the
/// mask has nullified contributes nothing, and every ancestor's traverse bit
/// is folded in. A presence check cannot see either failure.
#[async_trait]
pub trait EndpointAccessSource: Send + Sync + 'static {
    /// Re-read what the consumer principal has on the exact endpoint.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the observation could not be completed. An answer
    /// of `Ok(None)` means the path no longer names a socket at all, so the
    /// caller must prepare the NEW exact endpoint rather than reuse this one.
    async fn observe(
        &self,
        endpoint: &ResourceRef,
        purpose: &str,
        consumer: &ResourceRef,
    ) -> Result<Option<crate::binding::EndpointAccessObservation>, String>;
}

/// The grant of one admitted right on one exact endpoint (U18).
#[async_trait]
pub trait EndpointGrantSource: Send + Sync + 'static {
    /// Grant the consumer principal exactly the admitted right on the exact
    /// endpoint, plus traverse on every ancestor directory and never listing
    /// on the containing directory, and report what is EFFECTIVE afterwards.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the grant could not be applied or could not be
    /// proven effective. A grant that lands but is not effective is an error,
    /// never a success: the mask-nullification failure is exactly the one a
    /// presence check cannot see.
    async fn grant(
        &self,
        endpoint: &ResourceRef,
        purpose: &str,
        consumer: &ResourceRef,
        socket_right_bits: u32,
    ) -> Result<crate::binding::EndpointAccessObservation, String>;
}

/// The revoke of one consumer principal's grant on one exact endpoint (U18).
#[async_trait]
pub trait EndpointRevokeSource: Send + Sync + 'static {
    /// Drop the consumer principal's entry on the exact endpoint inode.
    ///
    /// This is the fence against NEW use and the release, and they are one
    /// effect because the kernel enforces the entry at `connect(2)`: a
    /// principal whose entry is gone cannot newly reach the inode, while a
    /// connection it already holds is unaffected. Revoking never touches the
    /// ancestor traverse grants, which sibling endpoints and the producer's
    /// own helpers also depend on.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the removal could not be completed. A socket that
    /// is already gone, or was never granted, answers `Ok(())` - the removal
    /// is idempotent under retry (R10).
    async fn revoke(
        &self,
        endpoint: &ResourceRef,
        purpose: &str,
        consumer: &ResourceRef,
    ) -> Result<(), String>;
}