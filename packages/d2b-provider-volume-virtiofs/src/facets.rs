//! The declared facet one committed `VolumeBinding` row's virtiofs delivery
//! rides (U15).
//!
//! The serving pass itself is this crate's own: [`crate::VirtiofsBindingController`]
//! derives the worker plan and the private socket from the admitted binding,
//! and drives the privileged leg through the typed [`VirtiofsBindingEffectPort`]
//! that [`crate::VirtiofsBindingPort`] builds out of the dispatch declared
//! here. The dispatch is what crosses the provider boundary: only the daemon
//! holds the broker socket, the host runtime root, and the live target
//! directory, so every privileged fact the pass needs arrives as the typed
//! answer this trait returns.
//!
//! # The answer is the evidence, never the request
//!
//! Every request carries the relationship's own KTD3 fence, its derived
//! socket identity, and (for a launch) the exact plan the controller derived
//! FROM the committed row. The answer carries only what the privileged leg
//! actually observed, and [`crate::VirtiofsBindingPort`] reconciles it against
//! the derived facts before the pass believes it: a worker that came back
//! under another identity, or a socket probe that answered about another
//! relationship, is refused by name rather than folded into a verdict. A
//! delivery the privileged leg could not observe is never reported as one.
//!
//! # A plane with no daemon grants nothing
//!
//! The privileged leg is daemon-only. [`UnwiredVirtiofsServing`] is the value
//! a plane built without a broker socket uses: it refuses every verb BY NAME
//! and grants none, so an undelivered relationship says why instead of
//! looking healthy.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume_binding::{
    VolumeBindingReadinessFence, VolumeBindingStatusResource,
};

use crate::bindings::{SocketIdentity, StoredBinding};
use crate::port::{LaunchedWorker, MountObservation, ServingWorkerLaunch};

/// The closed verb set one committed relationship's delivery uses.
///
/// The set is closed because each verb crosses a different privileged
/// boundary and each answer is a different kind of evidence: a launch
/// realizes the worker, an observation reads live state, a removal tears
/// the worker down, and a publication writes the fenced status under the
/// virtiofs controller identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VirtiofsServingVerb {
    /// Realize the binding-owned virtiofsd worker from the derived launch.
    Launch,
    /// Report what the privileged leg currently observes for this
    /// relationship: the private socket, the closure store-view marker, and
    /// the consumer's mount point.
    Observe,
    /// Delete the binding-owned worker and its endpoint realization.
    Remove,
    /// Write the fenced binding status projection (KTD3).
    Publish,
}

impl VirtiofsServingVerb {
    /// The closed, path-free slug a verb is named under.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Observe => "observe",
            Self::Remove => "remove",
            Self::Publish => "publish",
        }
    }
}

impl fmt::Display for VirtiofsServingVerb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why one privileged virtiofs dispatch did not answer.
///
/// The closed split is the one the serving pass needs: a refusal names a
/// condition the committed row cannot fix, while a transport that never
/// answered says nothing about the relationship and is retried. Neither
/// carries a socket path, a shared directory, argv, or a numerical
/// principal, so a refusal reads the same wherever it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtiofsServingError {
    /// The privileged leg refused the request before or instead of
    /// applying it. The slug is the closed refusal class it reported.
    Refused(String),
    /// The privileged leg did not answer at all.
    ///
    /// An absent answer is never an absence of effect: the relationship
    /// stays outstanding and the pass retries.
    Unavailable(String),
}

impl VirtiofsServingError {
    /// The closed, path-free slug a refusal reports under.
    pub fn code(&self) -> &str {
        match self {
            Self::Refused(code) => code,
            Self::Unavailable(_) => "virtiofs-serving-dispatch-unavailable",
        }
    }
}

impl fmt::Display for VirtiofsServingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for VirtiofsServingError {}

/// What one privileged observation actually saw.
///
/// All three facts come back from a single probe of the same relationship,
/// so a pass never mixes evidence from two different sockets or two
/// different consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtiofsServingObservation {
    socket: SocketIdentity,
    socket_listening: bool,
    marker_present: bool,
    consumer_mount: MountObservation,
}

impl VirtiofsServingObservation {
    /// Bind one probe's answer to the relationship it was asked about.
    pub const fn new(
        socket: SocketIdentity,
        socket_listening: bool,
        marker_present: bool,
        consumer_mount: MountObservation,
    ) -> Self {
        Self {
            socket,
            socket_listening,
            marker_present,
            consumer_mount,
        }
    }

    /// The socket identity the privileged leg answered about.
    ///
    /// The serving side compares this against the identity it derived from
    /// the committed row; an answer about another socket is not evidence
    /// about this relationship.
    pub const fn socket(&self) -> SocketIdentity {
        self.socket
    }

    /// Whether the worker's private socket is listening right now.
    pub const fn socket_listening(&self) -> bool {
        self.socket_listening
    }

    /// Whether the closure store-view's zero-length readiness marker is
    /// present.
    pub const fn marker_present(&self) -> bool {
        self.marker_present
    }

    /// What the consumer currently observes at its mount point.
    pub const fn consumer_mount(&self) -> MountObservation {
        self.consumer_mount
    }
}

/// What one privileged dispatch returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtiofsServingAnswer {
    /// The worker the privileged leg realized for this launch.
    Launched(LaunchedWorker),
    /// The live state the privileged leg observed.
    Observed(VirtiofsServingObservation),
    /// The worker and its endpoint realization are gone.
    Removed,
    /// The fenced status projection was accepted under the virtiofs
    /// controller identity.
    Published,
}

/// One privileged dispatch request, carrying only what the serving side
/// derived FROM the committed row.
///
/// A request never carries a caller-supplied destination, a view root, an
/// argv, or a Guest row: the launch it carries is the plan the controller
/// derived from the admitted binding, and the fence it carries is the
/// committed row's own identity (KTD3).
pub struct VirtiofsServingRequest {
    verb: VirtiofsServingVerb,
    fence: VolumeBindingReadinessFence,
    zone: BoundedToken,
    socket: SocketIdentity,
    launch: Option<ServingWorkerLaunch>,
    worker: Option<LaunchedWorker>,
    writer: Option<BoundedToken>,
    projection: Option<VolumeBindingStatusResource>,
    /// The worker `Process` reference the committed row derives.
    ///
    /// The privileged leg realizes exactly this worker and the answer
    /// carries it back: a worker that came back under another reference is
    /// evidence about a different row, not about this one.
    worker_ref: ResourceRef,
}

impl VirtiofsServingRequest {
    /// Build a launch request from the binding and the derived launch.
    pub fn for_launch(
        binding: &StoredBinding,
        zone: BoundedToken,
        worker: ResourceRef,
        launch: ServingWorkerLaunch,
    ) -> Self {
        Self {
            verb: VirtiofsServingVerb::Launch,
            fence: binding.fence(),
            socket: binding.serving_socket(&zone),
            zone,
            launch: Some(launch),
            worker: None,
            writer: None,
            projection: None,
            worker_ref: worker,
        }
    }

    /// Build an observation request for one relationship.
    ///
    /// `worker` is the launched worker whose socket is being probed, or
    /// `None` for a probe that runs before the worker exists (the closure
    /// store-view marker check).
    pub fn for_observe(
        binding: &StoredBinding,
        zone: BoundedToken,
        worker_ref: ResourceRef,
        worker: Option<LaunchedWorker>,
    ) -> Self {
        Self {
            verb: VirtiofsServingVerb::Observe,
            fence: binding.fence(),
            socket: binding.serving_socket(&zone),
            zone,
            launch: None,
            worker,
            writer: None,
            projection: None,
            worker_ref,
        }
    }

    /// Build a removal request for one relationship's worker.
    pub fn for_remove(
        binding: &StoredBinding,
        zone: BoundedToken,
        worker_ref: ResourceRef,
        worker: LaunchedWorker,
    ) -> Self {
        Self {
            verb: VirtiofsServingVerb::Remove,
            fence: binding.fence(),
            socket: binding.serving_socket(&zone),
            zone,
            launch: None,
            worker: Some(worker),
            writer: None,
            projection: None,
            worker_ref,
        }
    }

    /// Build a fenced status publication request.
    pub fn for_publish(
        binding: &StoredBinding,
        zone: BoundedToken,
        worker_ref: ResourceRef,
        writer: BoundedToken,
        projection: VolumeBindingStatusResource,
    ) -> Self {
        Self {
            verb: VirtiofsServingVerb::Publish,
            fence: binding.fence(),
            socket: binding.serving_socket(&zone),
            zone,
            launch: None,
            worker: None,
            writer: Some(writer),
            projection: Some(projection),
            worker_ref,
        }
    }

    /// The verb this request travels under.
    pub const fn verb(&self) -> VirtiofsServingVerb {
        self.verb
    }

    /// The committed row's KTD3 fence.
    pub const fn fence(&self) -> &VolumeBindingReadinessFence {
        &self.fence
    }

    /// The worker `Process` reference this relationship derives.
    pub const fn worker_ref(&self) -> &ResourceRef {
        &self.worker_ref
    }

    /// The Zone the relationship lives in.
    pub const fn zone(&self) -> &BoundedToken {
        &self.zone
    }

    /// The socket identity the serving side derived from the committed row.
    pub const fn socket(&self) -> SocketIdentity {
        self.socket
    }

    /// The derived launch, on a [`VirtiofsServingVerb::Launch`] request.
    pub const fn launch(&self) -> Option<&ServingWorkerLaunch> {
        self.launch.as_ref()
    }

    /// The worker identity the request is about.
    pub const fn worker(&self) -> Option<&LaunchedWorker> {
        self.worker.as_ref()
    }

    /// The controller identity publishing a status projection.
    pub const fn writer(&self) -> Option<&BoundedToken> {
        self.writer.as_ref()
    }

    /// The fenced status projection being published.
    pub const fn projection(&self) -> Option<&VolumeBindingStatusResource> {
        self.projection.as_ref()
    }
}

impl fmt::Debug for VirtiofsServingRequest {
    /// Renders the verb, the fence, and the derived identity, and never the
    /// socket path, the plan, or the projection payload: a debug line is a
    /// log line, and a log line never carries a private path.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VirtiofsServingRequest")
            .field("verb", &self.verb.as_str())
            .field("fence", &self.fence)
            .field("zone", &self.zone.as_str())
            .field("socket", &self.socket)
            .field("worker", &self.worker.as_ref().map(|worker| &worker.process_ref))
            .field("launch", &self.launch.as_ref().map(|launch| &launch.plan))
            .field("writer", &self.writer.as_ref().map(BoundedToken::as_str))
            .field("publishes_projection", &self.projection.is_some())
            .finish()
    }
}

/// The privileged virtiofs serving dispatch one committed relationship's
/// delivery rides (U15).
///
/// The daemon implements this facet over its own broker socket, host
/// runtime root, and live Zone target directory. This crate's serving pass
/// builds the typed request and reconciles the answer and holds no socket,
/// no host path, and no numerical principal of its own, so a request that
/// arrives here is a claim to be checked rather than a grant.
pub trait VirtiofsServingDispatch: Send + Sync + 'static {
    /// Send one request and return the privileged leg's own answer.
    ///
    /// The boxed future is what keeps the facet object-safe: the daemon
    /// supplies one implementation and the family holds it erased, so a
    /// daemon dispatch and an unwired refusal are the same type at the
    /// composition site.
    fn dispatch<'life>(
        &'life self,
        request: VirtiofsServingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<VirtiofsServingAnswer, VirtiofsServingError>> + Send + 'life>>;
}

/// The dispatch a plane uses when it carries no daemon to reach a broker.
///
/// The privileged leg is a daemon-only capability: only the daemon holds the
/// broker socket, the runtime root, and the live target directory. A plane
/// built without one therefore delivers nothing, and says so BY NAME rather
/// than pretending a delivery happened. This grants no authority - it
/// refuses every verb, and the serving pass reports the relationship
/// undelivered.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredVirtiofsServing;

impl VirtiofsServingDispatch for UnwiredVirtiofsServing {
    fn dispatch<'life>(
        &'life self,
        request: VirtiofsServingRequest,
    ) -> Pin<Box<dyn Future<Output = Result<VirtiofsServingAnswer, VirtiofsServingError>> + Send + 'life>>
    {
        Box::pin(async move {
            Err(VirtiofsServingError::Unavailable(format!(
                "the virtiofs serving dispatch is daemon-only and this plane has no broker socket to \
                 reach it for {}",
                request.verb()
            )))
        })
    }
}
