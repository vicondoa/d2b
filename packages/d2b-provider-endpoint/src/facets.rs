//! The declared facets the provider-owned Endpoint effects service reaches
//! daemon state through (U6).
//!
//! The Endpoint family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the preserved
//! endpoint realization. The realization splits into daemon-owned surfaces,
//! and each crosses the provider boundary as a declared facet rather than a
//! daemon call: the host socket effect for the binding-owned
//! virtiofsd socket (resolve the producer's private socket target and probe
//! or mutate the bound socket on the host target), the two row-evidence
//! probes (the guest-runtime control endpoints read the guest's committed
//! VMM Process row; the device-worker endpoints read the producer worker
//! Process row), the private host observation a Provider-committed socket
//! shape is realized behind, and the Provider vocabularies that admit the
//! shapes a declaring Provider commits. The purpose derivations that classify
//! one purpose onto those surfaces are this crate's own knowledge (see
//! [`crate::effects_service`]), so the facets never see a purpose decision,
//! and a host observation answers with an opaque minted handle rather than
//! with the socket it looked at (KTD8).

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    EndpointAccessRequest, EndpointAccessResponse, EndpointAccessVerb,
};
use d2b_contracts_resource::v3::execution_policy::{BoundedToken, redacted_debug};
use d2b_contracts_resource::v3::ResourceRef;

use crate::driver::CommittedEndpointShape;
use crate::endpoint::EndpointSpec;

/// The daemon-supplied facet set the provider-owned Endpoint effects are
/// built from (U6).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). Five seams cross the provider boundary: the host
/// socket surface and the two row-evidence probes this crate realizes
/// itself, the Provider vocabularies that admit the shapes a declaring
/// Provider commits, and the daemon's private host observation. Every other
/// input (the purpose derivations and the dispatch onto the surfaces) is
/// this crate's own knowledge.
///
/// The last two default to their closed answers. A composition that injects
/// no Provider vocabulary and carries no daemon admits nothing on their
/// account, which is the honest prior rather than a missing answer (KTD5).
#[derive(Clone)]
pub struct EndpointEffectFacets {
    /// The host socket surface: the binding-owned virtiofsd socket's
    /// presence, ensure, and removal over the daemon's socket target
    /// registry and runtime directory.
    pub(crate) socket: Arc<dyn EndpointSocketSource>,
    /// The guest-runtime control evidence: whether the producer Guest's
    /// committed VMM Process row reports `Ready` at its current generation.
    pub(crate) guest_vmm: Arc<dyn GuestVmmEvidenceSource>,
    /// The device-worker evidence: whether the producer worker Process row
    /// reports `Ready` at its current generation.
    pub(crate) device_worker: Arc<dyn DeviceWorkerEvidenceSource>,
    /// The Provider vocabularies of this Zone: which committed rows are the
    /// shapes their declaring Providers commit.
    pub(crate) committed: Arc<dyn CommittedEndpointShapeSource>,
    /// The daemon's private host observation, for the shapes realized behind
    /// a socket the daemon owns.
    pub(crate) host_socket: Arc<dyn HostSocketEvidenceSource>,
}

impl EndpointEffectFacets {
    /// Build the three daemon surfaces this crate realizes itself over.
    ///
    /// The two Provider-owned seams take their closed answers: nothing is
    /// admitted from a shape no declaring Provider has committed, and no host
    /// socket is observed. A composition that supplies the two installs them
    /// with [`Self::with_committed_shapes`] and
    /// [`Self::with_host_socket_observation`].
    pub fn new(
        socket: Arc<dyn EndpointSocketSource>,
        guest_vmm: Arc<dyn GuestVmmEvidenceSource>,
        device_worker: Arc<dyn DeviceWorkerEvidenceSource>,
    ) -> Self {
        Self {
            socket,
            guest_vmm,
            device_worker,
            committed: Arc::new(UnwiredCommittedShapes),
            host_socket: Arc::new(UnwiredHostSocketEvidence),
        }
    }

    /// Install the Provider vocabularies that admit this Zone's
    /// Provider-committed shapes (KTD5).
    ///
    /// The vocabulary is a declaring Provider's own object, matched in full
    /// against the shape that Provider committed; this crate asks the
    /// question and takes the one exact verdict. Passing no vocabulary is the
    /// closed answer and is what [`Self::new`] installs.
    #[must_use]
    pub fn with_committed_shapes(
        mut self,
        committed: Arc<dyn CommittedEndpointShapeSource>,
    ) -> Self {
        self.committed = committed;
        self
    }

    /// Install the daemon's private host observation (KTD5, KTD8).
    ///
    /// The observation is daemon state: it knows the locator the endpoint
    /// owner committed, the exact socket standing there, and whether that
    /// socket accepts a connection, and none of that crosses. A composition
    /// that installs none observes no exact socket, which is what
    /// [`Self::new`] installs.
    #[must_use]
    pub fn with_host_socket_observation(
        mut self,
        host_socket: Arc<dyn HostSocketEvidenceSource>,
    ) -> Self {
        self.host_socket = host_socket;
        self
    }
}

/// The Provider vocabularies that decide which committed rows this Zone
/// admits (KTD5).
///
/// A shape a declaring Provider commits is named by that Provider and by
/// nobody else: the Provider owns its constants and every field of its own
/// exact match, and answers with one verdict. This crate never names a
/// Provider, a Provider type, or a Provider's vocabulary - it asks, and the
/// composition root installs whichever vocabularies the Zone's Providers
/// published.
///
/// The default admits nothing, which is what a composition that installs no
/// Provider vocabulary gets.
pub trait CommittedEndpointShapeSource: Send + Sync + 'static {
    /// The exact shape ONE declaring Provider commits for `spec`, or `None`
    /// for any spec this Provider does not commit.
    ///
    /// `None` is a terminal refusal and never a near miss: a row that differs
    /// from the committed shape on any structural field is not a shape this
    /// Provider commits, and this crate refuses it rather than repairing it
    /// into an admission.
    fn committed_endpoint_shape(&self, _spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        None
    }
}

/// The committed-shape source a composition that installed no Provider
/// vocabulary answers with.
///
/// A Zone whose Providers published no vocabulary admits no Provider-committed
/// shape: the closed answer is what a composition that supplied nothing gets,
/// and it is the same answer a Provider that commits no shape of this crate's
/// families gives (KTD5).
#[derive(Clone, Copy, Default)]
pub struct UnwiredCommittedShapes;

impl CommittedEndpointShapeSource for UnwiredCommittedShapes {
    fn committed_endpoint_shape(&self, _spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        None
    }
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

/// The narrowest nonce a realization-handle source may offer (KTD8).
///
/// KTD8 sets the floor for an incarnation token at 128 bits of
/// unpredictability. The handle takes its nonce from the source that mints
/// it, so this is the width that floor names. The width is a FLOOR on the
/// source's claim and never a proof of its entropy; it exists so a source
/// that cannot state 128 bits cannot be installed as the fence a launch
/// gate compares.
pub const MIN_REALIZATION_NONCE_CHARS: usize = 32;

/// The daemon-minted opaque handle of one realization that is CURRENT.
///
/// A handle is minted when the realization it names becomes current and
/// rotated when that realization is replaced or when the daemon restarts:
/// the daemon is the only party that knows both, so the daemon mints it.
///
/// Nothing about WHERE the realization lives takes part. No path, no
/// `(dev, ino)` pair, and no socket name enters a handle, so it cannot be
/// turned back into a locator by anyone who reads one; `Debug` renders the
/// redacted marker rather than the value, so a log line built from a handle
/// leaks neither the token nor the thing it names (KTD8, R15).
#[derive(Clone, PartialEq, Eq)]
pub struct RealizationHandle {
    nonce: BoundedToken,
    rotation: u64,
}

impl RealizationHandle {
    /// Bind one minted handle.
    ///
    /// # Errors
    ///
    /// Returns `None` when the nonce is narrower than
    /// [`MIN_REALIZATION_NONCE_CHARS`]: a source that cannot state the
    /// KTD8 floor has not minted an incarnation fence, and installing one
    /// anyway would make a guessable token the thing a launch gate trusts.
    pub fn mint(nonce: BoundedToken, rotation: u64) -> Option<Self> {
        if nonce.as_str().len() < MIN_REALIZATION_NONCE_CHARS {
            return None;
        }
        Some(Self { nonce, rotation })
    }

    /// Borrow the opaque minted nonce.
    ///
    /// Only a realization-incarnation derivation reads this value: it is an
    /// input to a digest, never something a projection or a log carries.
    pub const fn nonce(&self) -> &BoundedToken {
        &self.nonce
    }

    /// The rotation counter this handle was minted at.
    ///
    /// The counter moves when the realization is replaced and when the
    /// daemon restarts, so it is one of the facts a replacement cannot
    /// reproduce.
    pub const fn rotation(&self) -> u64 {
        self.rotation
    }
}

redacted_debug!(RealizationHandle);

/// The daemon-supplied private host observation (U5, KTD5, KTD8).
///
/// A host socket realization is resolved and owned by the daemon: it knows
/// the locator the endpoint owner committed, the exact socket that locator
/// currently stands for, and whether that socket accepts a connection. None
/// of that crosses this facet - only the minted [`RealizationHandle`] does,
/// and only for an observation that proved it (KTD8).
#[async_trait]
pub trait HostSocketEvidenceSource: Send + Sync + 'static {
    /// The realization standing behind `endpoint_ref` for `purpose`, or
    /// `None` when nothing is standing there.
    ///
    /// The three ways a socket can fail to be the one this endpoint named -
    /// nothing bound at the locator, something bound that does not accept a
    /// connection, and a socket replaced under the same locator - are ONE
    /// answer here. They read the same way to a consumer, so the facet
    /// answers the same way, and the exact socket identity stays inside the
    /// daemon that compared it.
    async fn observe(&self, endpoint_ref: &ResourceRef, purpose: &str) -> Option<RealizationHandle>;
}

/// The host observation a plane built without a daemon can answer with.
///
/// A host socket realization belongs to the daemon that owns it (KTD5), so a
/// plane carrying no daemon has observed no exact socket and answers none:
/// the endpoint behind it stays unrealized instead of reporting readiness
/// nothing proved.
#[derive(Clone, Copy, Default)]
pub struct UnwiredHostSocketEvidence;

#[async_trait]
impl HostSocketEvidenceSource for UnwiredHostSocketEvidence {
    async fn observe(&self, _endpoint_ref: &ResourceRef, _purpose: &str) -> Option<RealizationHandle> {
        None
    }
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
