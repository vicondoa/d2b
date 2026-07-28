//! Zone message bus: router, registry, authorization, streams, operations.
//!
//! The crate deliberately has no adapter registration in another service.
//! A Zone runtime must own the single registration authority and provide
//! authenticated session claims before any route becomes reachable.

pub mod authorization;
pub mod operations;
pub mod registry;
pub mod router;
pub mod streams;

pub use authorization::{
    AuthorizationError, AuthorizationErrorClass, BusAuthorizer, session_verb_name,
};
pub use d2b_resource_api::authz::SessionVerb;
pub use operations::{Cancellation, OperationId, OperationSpec};
pub use registry::{
    BusEndpoint, BusResponse, EndpointError, EndpointFailureClass, EndpointSessionFailure,
    RouteGenerations, RouteKey, RouteMember, RouteTarget,
};
pub use router::{
    BusClock, BusConfig, BusError, BusEvent, BusFailureReason, BusIngress, BusObserver, BusStream,
    ComponentSessionAdmission, DeliveredInvocation, DeliveredStream, ManualClock, NoopBusObserver,
    ResourceCall, ResourceFilter, ResourceQuery, ZoneBus, ZoneRegistrar,
};
pub use streams::{IncomingStream, ReceivedFrame, StreamError, StreamLimits, StreamName};
