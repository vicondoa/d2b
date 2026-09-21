//! The provider service contract: the real envelope payload and the
//! capability object built from a method's declared facets (R6, R7, R8).
//!
//! A provider declares the services it serves on its driver descriptors
//! ([`d2b_resource_types::ServiceDecl`]); the daemon hosts one actor per
//! declared service and dispatches the envelope's real payload to the
//! provider's implementation through the capability object
//! ([`ServiceInvocation`]). The implementation lives in the declaring
//! provider crate, which links this toolkit - the hosting machinery in the
//! daemon consumes the same contract from here.
//!
//! The request payload is the envelope's canonical object
//! ([`EffectRequest`]), not a fixture byte vector (R8): the broker
//! validated the payload against the operation row's payload schema before
//! it forwarded the call, and the service receives exactly that canonical
//! object. The capability object carries the facets the method declared -
//! the driver context for resource-state reads, the declared state cells,
//! the per-zone kernel seam, and the declared descriptor legs - so a
//! service reaches resource state through the generic driver context and
//! daemon-structural state only through its declared state cells (R7).

use std::os::fd::{OwnedFd, RawFd};
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_resource_runtime::context::ServiceResourceContext;
use d2b_resource_types::{KernelCaller, MethodFdContract};
use thiserror::Error;

/// The canonical request payload of one service invocation: the real
/// envelope contract (R8) - the same canonical object the broker validated
/// against the operation row's payload schema before it forwarded the call.
pub type EffectRequest = CanonicalJsonObject;

/// The result of one service invocation: the canonical response object the
/// envelope carries back to the caller, plus the descriptors the service
/// minted for its declared response leg.
///
/// The descriptors travel with the result over the forward carrier's fd
/// leg; the caller owns them once the reply frame has gone. A service that
/// mints no descriptor leaves the vector empty.
#[derive(Debug)]
pub struct EffectResponse {
    /// The canonical response payload.
    pub payload: CanonicalJsonObject,
    /// The descriptors the service minted for this invocation, in frame
    /// order. Empty when the method's response leg declares no carriage.
    pub fds: Vec<OwnedFd>,
}

impl EffectResponse {
    /// Wrap a canonical response payload object with no descriptors.
    pub fn new(payload: CanonicalJsonObject) -> Self {
        Self {
            payload,
            fds: Vec::new(),
        }
    }

    /// Wrap a canonical response payload object plus the descriptors the
    /// service minted for its declared response leg.
    pub fn with_fds(payload: CanonicalJsonObject, fds: Vec<OwnedFd>) -> Self {
        Self { payload, fds }
    }
}

/// The refusal set for effect-service calls (KTD7 shape: a
/// machine-actionable closed code set, never a hang and never flattened to a
/// generic `unregistered` outcome).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectServiceError {
    /// No service of this identity is published in the zone.
    #[error("no service `{service}` is published in zone `{zone}`")]
    UnboundService {
        /// The zone that holds no such service.
        zone: String,
        /// The service identity that is not published.
        service: String,
    },
    /// No hosted service declares the operation.
    #[error("no hosted effect service declares an operation `{operation}`")]
    OperationUnserved {
        /// The operation no declared method names.
        operation: String,
    },
    /// The durable row belongs to a different zone than the one hosting it.
    #[error("row for `{service}` belongs to zone `{row_zone}`, not `{zone}`")]
    WrongZone {
        /// The zone that refused the row.
        zone: String,
        /// The service identity the row declares.
        service: String,
        /// The zone the row belongs to.
        row_zone: String,
    },
    /// The binding's generational revision moved past the caller's capture.
    #[error("binding for `{service}` is stale: revision {current} != expected {expected} (KTD5)")]
    StaleRevision {
        /// The service identity whose generation moved.
        service: String,
        /// The revision the caller captured.
        expected: u64,
        /// The binding's current revision.
        current: u64,
    },
    /// The binding targets an actor that is no longer running.
    #[error("service `{service}` is not running; its binding targets a dead actor (KTD5)")]
    ServiceUnavailable {
        /// The service identity whose actor is gone.
        service: String,
    },
    /// The actor died while the call was in flight.
    #[error("service `{service}` died mid-call; the caller's revision is stale (KTD5)")]
    InFlightStale {
        /// The service identity whose actor died mid-call.
        service: String,
    },
    /// The service answered with its own refusal.
    #[error("`{service}` declined: {reason}")]
    Declined {
        /// The service identity that declined.
        service: String,
        /// The reason the service named.
        reason: String,
    },
}

/// The capability object of one service invocation (R6, R7): built from the
/// method's declared facets - the driver context for resource-state reads,
/// the declared state cells, the per-zone kernel seam, and the declared
/// descriptor legs - beside the invocation's own envelope contract.
///
/// The service reaches resource state through [`Self::resources`] (the
/// generic driver context against the resource store) and daemon-structural
/// state only through the cells [`Self::state_cells`] names; the per-zone
/// kernel seam ([`Self::kernel`]) is how a service invokes broker-generic
/// kernels as nested calls, and the descriptor legs ([`Self::request_fds`],
/// [`Self::response_fds`]) are the method's declared fd carriage.
pub struct ServiceInvocation<'a> {
    /// The zone the invocation runs in.
    pub zone: &'a str,
    /// The declared method being served, by its declared name.
    ///
    /// The actor builds the capability object from the call's declared
    /// method facets, so the method identity rides with the invocation:
    /// a service that serves several methods distinguishes them here rather
    /// than guessing from the payload.
    pub method: &'a str,
    /// The invocation identifier the audit record carries.
    pub invocation_id: &'a str,
    /// The canonical request payload the envelope validated (R8).
    pub payload: &'a CanonicalJsonObject,
    /// The generic driver context for resource-state reads (R7): the
    /// manager-plane surface, never the spec store directly.
    pub resources: &'a mut ServiceResourceContext,
    /// The declared state cells of the method being served (R7): the
    /// daemon-structural state this method may reach, by name.
    pub state_cells: &'a [&'a str],
    /// The per-zone kernel seam (U10), when the composition point wired
    /// one. Absent, a service that needs a kernel leg refuses when it
    /// invokes one.
    pub kernel: Option<&'a KernelCaller>,
    /// The descriptors the caller attached on the request leg, admitted
    /// against the method's declared request-fd contract before dispatch.
    pub request_fds: &'a [RawFd],
    /// The declared response-leg fd contract the returned descriptors must
    /// satisfy.
    pub response_fds: MethodFdContract,
    /// The declared payload schema reference of the method being served:
    /// the row schema the envelope validated the payload against.
    pub payload_schema: Option<&'a str>,
}

/// A provider service: one handle-loop shape plus an optional timer-driven
/// poll tick. Providers implement this over carrier-delivered invocations;
/// the daemon hosts one actor per declared service and dispatches the
/// envelope's real payload through the capability object.
#[async_trait]
pub trait EffectService: Send + Sync + 'static {
    /// Handle one invocation.
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError>;

    /// Timer-driven poll tick (default: nothing). The actor's requeue timer
    /// drives this on the declared interval - never a thread.
    async fn poll(&self) {}
}

/// Rebuilds a service from a durable row; a respawn calls `build` again,
/// exactly like `ResourceManager` re-creates its drivers from the committed
/// spec row.
pub trait EffectServiceFactory: Send + Sync + 'static {
    /// Build a fresh service instance.
    fn build(&self) -> Arc<dyn EffectService>;
}