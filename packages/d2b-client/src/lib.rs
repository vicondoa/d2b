//! Generic v3 client and retry layer.
//!
//! The client carries only bounded metadata and delegates transport and
//! service resolution to injected implementations. It does not hold a store
//! handle or a Zone authority.

#![forbid(unsafe_code)]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use d2b_contracts::v3::{ResourceName, ResourceRef, ZoneId};

/// Maximum bytes in one metadata token.
pub const MAX_METADATA_TOKEN_BYTES: usize = 128;

/// A v3 client target. Resource addressing is always Zone-scoped; there is no
/// Realm or Workload spelling and no caller-supplied host path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetInput {
    /// The Zone service itself.
    Zone(ZoneId),
    /// A same-Zone resource service.
    Resource {
        /// Zone owning the resource.
        zone: ZoneId,
        /// Canonical resource identity within that Zone.
        resource: ResourceRef,
    },
}

impl TargetInput {
    /// Construct a Zone target.
    pub fn zone(zone: ZoneId) -> Self {
        Self::Zone(zone)
    }

    /// Construct a same-Zone resource target.
    pub fn resource(zone: ZoneId, resource: ResourceRef) -> Self {
        Self::Resource { zone, resource }
    }

    /// Borrow the target Zone.
    pub const fn zone_id(&self) -> &ZoneId {
        match self {
            Self::Zone(zone) | Self::Resource { zone, .. } => zone,
        }
    }

    /// Borrow the ResourceRef when this target is not the Zone service.
    pub const fn resource_ref(&self) -> Option<&ResourceRef> {
        match self {
            Self::Zone(_) => None,
            Self::Resource { resource, .. } => Some(resource),
        }
    }

    /// Parse `Zone/<name>` or `<ResourceType>/<name>` with an explicit Zone
    /// context. A ResourceRef never carries a cross-Zone component.
    pub fn parse(zone: ZoneId, value: &str) -> Result<Self, ClientError> {
        if let Some(name) = value.strip_prefix("Zone/") {
            let target = ZoneId::parse(name.to_owned()).map_err(|_| ClientError::InvalidTarget)?;
            if target != zone {
                return Err(ClientError::InvalidTarget);
            }
            return Ok(Self::Zone(zone));
        }
        let resource = ResourceRef::parse(value).map_err(|_| ClientError::InvalidTarget)?;
        if resource.resource_type().as_str() == "Zone" {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self::Resource { zone, resource })
    }
}

/// The semantic owner of a service endpoint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServiceOwner {
    /// The Zone runtime owns the service.
    Zone(ZoneId),
    /// A resource in one Zone owns the service.
    Resource {
        /// Owning Zone.
        zone: ZoneId,
        /// Owning ResourceName.
        resource: ResourceName,
    },
}

impl ServiceOwner {
    /// Construct a resource owner without accepting a Realm/Workload pair.
    pub const fn resource(zone: ZoneId, resource: ResourceName) -> Self {
        Self::Resource { zone, resource }
    }

    /// Borrow the owning Zone.
    pub const fn zone_id(&self) -> &ZoneId {
        match self {
            Self::Zone(zone) | Self::Resource { zone, .. } => zone,
        }
    }
}

/// Transport kinds selected by the Zone route table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransportKind {
    /// Unix seqpacket bootstrap transport.
    UnixSeqpacket,
    /// Unix stream transport.
    UnixStream,
    /// Vsock transport selected by an authenticated ZoneLink.
    Vsock,
}

/// A bounded transport selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TransportSelection {
    kind: TransportKind,
    generation: u64,
}

impl TransportSelection {
    /// Construct a transport selection with a nonzero route generation.
    pub const fn new(kind: TransportKind, generation: u64) -> Result<Self, ClientError> {
        if generation == 0 {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self { kind, generation })
    }

    /// Return the selected transport kind.
    pub const fn kind(self) -> TransportKind {
        self.kind
    }

    /// Return the route generation.
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// One route-table record resolved by the Zone bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRecord {
    target: TargetInput,
    owner: ServiceOwner,
    transport: TransportSelection,
}

impl RouteRecord {
    /// Construct a route record and require matching Zone custody.
    pub fn new(
        target: TargetInput,
        owner: ServiceOwner,
        transport: TransportSelection,
    ) -> Result<Self, ClientError> {
        if target.zone_id() != owner.zone_id() {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self {
            target,
            owner,
            transport,
        })
    }

    /// Borrow the target.
    pub const fn target(&self) -> &TargetInput {
        &self.target
    }

    /// Borrow the service owner.
    pub const fn owner(&self) -> &ServiceOwner {
        &self.owner
    }

    /// Return the selected transport.
    pub const fn transport(&self) -> TransportSelection {
        self.transport
    }
}

/// A deterministic route table with no mutable authority surface.
#[derive(Debug, Default)]
pub struct RouteTable {
    routes: Vec<RouteRecord>,
}

impl RouteTable {
    /// Add a route, rejecting duplicate target identities.
    pub fn insert(&mut self, route: RouteRecord) -> Result<(), ClientError> {
        if self
            .routes
            .iter()
            .any(|candidate| candidate.target == route.target)
        {
            return Err(ClientError::InvalidTarget);
        }
        self.routes.push(route);
        self.routes
            .sort_by(|left, right| left.target.cmp(&right.target));
        Ok(())
    }

    /// Resolve a route by target.
    pub fn resolve(&self, target: &TargetInput) -> Result<&RouteRecord, ClientError> {
        self.routes
            .iter()
            .find(|route| &route.target == target)
            .ok_or(ClientError::ServiceUnavailable)
    }
}

/// Connector abstraction for an established ComponentSession.
pub trait ComponentSessionConnector: Send + Sync {
    /// Establish a session for one already-resolved route.
    fn connect(&self, route: &RouteRecord) -> Result<ConnectedSession, ClientError>;
}

/// An established client-side session handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSession {
    transport: TransportSelection,
}

impl ConnectedSession {
    /// Construct a connected session from a selected transport.
    pub const fn new(transport: TransportSelection) -> Self {
        Self { transport }
    }

    /// Return the transport selection.
    pub const fn transport(self) -> TransportSelection {
        self.transport
    }
}

/// Precisely classified session failures used by retry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFailure {
    /// The request was rejected before dispatch.
    BeforeDispatch,
    /// The transport can safely retry.
    Retryable,
    /// Dispatch may have happened and retry is ambiguous.
    Ambiguous,
    /// The session disconnected after dispatch.
    Disconnected,
    /// The request deadline elapsed.
    Deadline,
    /// Cancellation was observed.
    Cancelled,
    /// The peer violated the protocol.
    Protocol,
}

impl SessionFailure {
    /// Whether retry policy may automatically retry this failure.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::BeforeDispatch | Self::Retryable)
    }
}

/// Named stream identity returned by a connected client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedStream {
    name: String,
}

impl NamedStream {
    /// Construct a bounded stream name.
    pub fn new(name: impl Into<String>) -> Result<Self, ClientError> {
        let name = bounded_token(name.into())?;
        Ok(Self { name })
    }

    /// Borrow the stream name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The identity of the daemon-owned public socket bootstrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpointIdentity {
    zone: ZoneId,
    bootstrap: String,
}

impl DaemonEndpointIdentity {
    /// Construct an identity from Zone bootstrap metadata, not a socket path.
    pub fn new(zone: ZoneId, bootstrap: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self {
            zone,
            bootstrap: bounded_token(bootstrap.into())?,
        })
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the bounded bootstrap identity.
    pub fn bootstrap(&self) -> &str {
        &self.bootstrap
    }
}

/// Host-socket connector seam. Binding the actual socket is owned by the
/// host integration; this type carries only its authenticated identity.
pub trait HostSocketConnector: Send + Sync {
    /// Return the daemon endpoint identity learned from bootstrap config.
    fn local_daemon_endpoint_identity(&self) -> Result<DaemonEndpointIdentity, ClientError>;
}

/// Validated request metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataInput {
    trace: Option<d2b_telemetry::TraceContext>,
    correlation_id: String,
    idempotency_key: Option<String>,
    deadline: Option<Duration>,
}

impl MetadataInput {
    /// Construct bounded metadata.
    pub fn new(
        correlation_id: impl Into<String>,
        idempotency_key: Option<String>,
        deadline: Option<Duration>,
    ) -> Result<Self, ClientError> {
        let correlation_id = bounded_token(correlation_id.into())?;
        if let Some(key) = &idempotency_key {
            bounded_token(key.clone())?;
        }
        if deadline.is_some_and(|value| value.is_zero()) {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(Self {
            trace: None,
            correlation_id,
            idempotency_key,
            deadline,
        })
    }

    /// Attach a validated trace context.
    pub fn with_trace(mut self, trace: d2b_telemetry::TraceContext) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Borrow the trace context.
    pub const fn trace(&self) -> Option<&d2b_telemetry::TraceContext> {
        self.trace.as_ref()
    }

    /// Borrow the correlation identity.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// Borrow the optional idempotency key.
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// Return a bounded deadline.
    pub const fn deadline(&self) -> Option<Duration> {
        self.deadline
    }
}

/// Retry configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum attempts including the first request.
    pub max_attempts: u8,
    /// Initial retry delay.
    pub initial_delay: Duration,
    /// Maximum retry delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    /// Validate policy bounds.
    pub fn validate(self) -> Result<Self, ClientError> {
        if self.max_attempts == 0
            || self.initial_delay.is_zero()
            || self.max_delay < self.initial_delay
            || self.max_delay > Duration::from_secs(60)
        {
            return Err(ClientError::InvalidRetryPolicy);
        }
        Ok(self)
    }

    /// Return the bounded delay for a one-based retry number.
    pub fn delay_for(self, retry_number: u8) -> Duration {
        let shift = u32::from(retry_number.saturating_sub(1)).min(16);
        let factor = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
        self.initial_delay
            .checked_mul(factor)
            .unwrap_or(self.max_delay)
            .min(self.max_delay)
    }
}

/// Transport resolver abstraction.
pub trait Resolver: Send + Sync {
    /// Resolve a v3 service package to an opaque endpoint handle.
    fn resolve(&self, service: &str) -> Result<Box<dyn Connector>, ClientError>;
}

/// Transport connector abstraction.
pub trait Connector: Send + Sync {
    /// Send one request and return a transport response.
    fn call(
        &self,
        service: &str,
        method: &str,
        metadata: &MetadataInput,
    ) -> Result<Vec<u8>, ClientError>;
}

/// Clock abstraction for cancellation and retry tests.
pub trait Clock: Send + Sync {
    /// Current monotonic instant.
    fn now(&self) -> Instant;
}

/// Native monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Cancellation token.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    /// Construct a token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel all operations observing this token.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Generic v3 client.
#[derive(Debug)]
pub struct Client<R, C = MonotonicClock> {
    resolver: Arc<R>,
    clock: C,
    retry: RetryPolicy,
}

impl<R, C> Client<R, C>
where
    R: Resolver,
    C: Clock,
{
    /// Construct a client.
    pub fn new(resolver: Arc<R>, clock: C, retry: RetryPolicy) -> Result<Self, ClientError> {
        Ok(Self {
            resolver,
            clock,
            retry: retry.validate()?,
        })
    }

    /// Call a typed service package/method with bounded retry behavior.
    pub fn call(
        &self,
        service: &str,
        method: &str,
        metadata: &MetadataInput,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, ClientError> {
        if service.is_empty() || method.is_empty() {
            return Err(ClientError::InvalidRoute);
        }
        let connector = self.resolver.resolve(service)?;
        let started = self.clock.now();
        for attempt in 0..self.retry.max_attempts {
            if cancellation.is_cancelled() {
                return Err(ClientError::Cancelled);
            }
            if metadata
                .deadline()
                .is_some_and(|deadline| self.clock.now().duration_since(started) >= deadline)
            {
                return Err(ClientError::DeadlineExpired);
            }
            match connector.call(service, method, metadata) {
                Ok(response) => return Ok(response),
                Err(error) if error.retryable() && attempt + 1 < self.retry.max_attempts => {
                    let _delay = self.retry.delay_for(attempt + 1);
                }
                Err(error) => return Err(error),
            }
        }
        Err(ClientError::RetryExhausted)
    }
}

/// Daemon-local typed client facade.
pub type DaemonClient<R, C = MonotonicClock> = Client<R, C>;
/// Guest-side typed client facade.
pub type GuestClient<R, C = MonotonicClock> = Client<R, C>;

/// Client failures with stable, identity-free codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// Metadata was malformed.
    InvalidMetadata,
    /// A Zone/resource target or route identity was malformed.
    InvalidTarget,
    /// A daemon endpoint identity was malformed.
    InvalidEndpointIdentity,
    /// Retry configuration was malformed.
    InvalidRetryPolicy,
    /// Service or method was not in the closed catalog.
    InvalidRoute,
    /// Connector could not be resolved.
    ServiceUnavailable,
    /// Request can be retried.
    TransportRetryable,
    /// Request failed permanently.
    TransportFailed,
    /// Request was cancelled.
    Cancelled,
    /// Request deadline elapsed.
    DeadlineExpired,
    /// All attempts were exhausted.
    RetryExhausted,
}

impl ClientError {
    /// Whether retrying may succeed.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::TransportRetryable | Self::ServiceUnavailable)
    }
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMetadata => "client-metadata-invalid",
            Self::InvalidTarget => "client-target-invalid",
            Self::InvalidEndpointIdentity => "client-endpoint-identity-invalid",
            Self::InvalidRetryPolicy => "client-retry-policy-invalid",
            Self::InvalidRoute => "client-route-invalid",
            Self::ServiceUnavailable => "client-service-unavailable",
            Self::TransportRetryable => "client-transport-retryable",
            Self::TransportFailed => "client-transport-failed",
            Self::Cancelled => "client-cancelled",
            Self::DeadlineExpired => "client-deadline-expired",
            Self::RetryExhausted => "client-retry-exhausted",
        })
    }
}

impl std::error::Error for ClientError {}

fn bounded_token(value: String) -> Result<String, ClientError> {
    if value.is_empty()
        || value.len() > MAX_METADATA_TOKEN_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(ClientError::InvalidMetadata);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ResolverImpl;

    impl Resolver for ResolverImpl {
        fn resolve(&self, service: &str) -> Result<Box<dyn Connector>, ClientError> {
            if service == "d2b.resource.v3" {
                Ok(Box::new(ConnectorImpl))
            } else {
                Err(ClientError::ServiceUnavailable)
            }
        }
    }

    struct ConnectorImpl;

    impl Connector for ConnectorImpl {
        fn call(
            &self,
            _service: &str,
            _method: &str,
            _metadata: &MetadataInput,
        ) -> Result<Vec<u8>, ClientError> {
            Ok(b"ok".to_vec())
        }
    }

    #[test]
    fn trace_metadata_and_typed_route_are_bounded() {
        let metadata = MetadataInput::new("correlation", None, None)
            .unwrap()
            .with_trace(d2b_telemetry::TraceContext::new("trace", "span").unwrap());
        let client = Client::new(
            Arc::new(ResolverImpl),
            MonotonicClock,
            RetryPolicy::default(),
        )
        .unwrap();
        let result = client.call(
            "d2b.resource.v3",
            "Get",
            &metadata,
            &CancellationToken::new(),
        );
        assert_eq!(result.unwrap(), b"ok");
        assert!(MetadataInput::new("bad id", None, None).is_err());
    }

    #[test]
    fn target_input_uses_zone_and_resource_refs() {
        let zone = ZoneId::parse("work").unwrap();
        let target = TargetInput::parse(zone.clone(), "Guest/workstation").unwrap();
        assert_eq!(target.zone_id(), &zone);
        assert_eq!(
            target.resource_ref().unwrap().to_canonical_string(),
            "Guest/workstation"
        );
        assert!(TargetInput::parse(zone.clone(), "Zone/other").is_err());
        assert!(TargetInput::parse(zone, "Realm/workload").is_err());
    }

    #[test]
    fn route_owner_and_target_must_share_zone() {
        let work = ZoneId::parse("work").unwrap();
        let other = ZoneId::parse("other").unwrap();
        let target = TargetInput::resource(
            work.clone(),
            ResourceRef::parse("Guest/workstation").unwrap(),
        );
        let owner = ServiceOwner::resource(other, ResourceName::parse("runtime").unwrap());
        let transport = TransportSelection::new(TransportKind::UnixSeqpacket, 1).unwrap();
        assert!(RouteRecord::new(target, owner, transport).is_err());
    }
}
