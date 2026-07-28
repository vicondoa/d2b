//! Exact Zone router and the single-owner registration surface.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts::v3::{ResourceName, ResourceRef, ResourceTypeName, ZoneId};
use d2b_resource_api::authz::{
    ApiMethod, AuthorizationRequest, AuthorizationState, AuthorizationTarget, PolicySet,
    ResourceVerb, SessionVerb,
};
use d2b_session::{
    AuthenticatedComponentSession, AuthenticatedSessionRouteBinding, GENERATED_OPERATION_CATALOG,
    OperationKind, SessionAcceptor, SessionAuthority, SessionAuthorizationRequest,
    SessionCancellationHandle, SessionOperation, SessionRegistrationCapability,
    contract::EndpointPolicy, resource_operation, ttrpc_request_id,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    authorization::{AuthorizationError, BusAuthorizer},
    operations::{
        Cancellation, OperationError, OperationId, OperationSpec, OperationTable, SessionId,
    },
    registry::{
        BusResponse, EndpointError, Registry, RegistryError, RouteKey, RouteTarget,
        SessionRegistration,
    },
    streams::{
        IncomingStream, OutgoingStream, StreamBridge, StreamError, StreamLimits, StreamName,
    },
};

/// Default maximum bytes in one method payload.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const DEFAULT_MAX_ROUTES_PER_SESSION: usize = 128;
pub const DEFAULT_MAX_TOTAL_ROUTES: usize = 4096;

/// Monotonic clock used for operation deadlines.
pub trait BusClock: Send + Sync + 'static {
    /// Return the current monotonic tick.
    fn now_tick(&self) -> u64;
}

struct SystemClock(Instant);

impl SystemClock {
    fn new() -> Self {
        Self(Instant::now())
    }
}

impl BusClock for SystemClock {
    fn now_tick(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Deterministic monotonic clock for tests and embedded runtimes.
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// Construct a clock at one tick.
    pub const fn new(tick: u64) -> Self {
        Self(AtomicU64::new(tick))
    }

    /// Advance to an equal or later tick.
    pub fn advance_to(&self, tick: u64) {
        self.0.fetch_max(tick, Ordering::AcqRel);
    }
}

impl BusClock for ManualClock {
    fn now_tick(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

impl core::fmt::Debug for ManualClock {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ManualClock(<redacted>)")
    }
}

/// Frozen bounds for one Zone bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    pub max_payload_bytes: usize,
    pub max_operations: usize,
    pub max_operations_per_session: usize,
    pub max_routes_per_session: usize,
    pub max_total_routes: usize,
    pub stream_limits: StreamLimits,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_operations: crate::operations::DEFAULT_MAX_OPERATIONS,
            max_operations_per_session: crate::operations::DEFAULT_MAX_OPERATIONS_PER_SESSION,
            max_routes_per_session: DEFAULT_MAX_ROUTES_PER_SESSION,
            max_total_routes: DEFAULT_MAX_TOTAL_ROUTES,
            stream_limits: StreamLimits::default(),
        }
    }
}

/// Exact indexed filter preserved in List and Watch calls.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceFilter {
    field: String,
    values: Vec<String>,
}

impl ResourceFilter {
    /// Construct a bounded exact-match filter.
    pub fn new(field: impl Into<String>, values: Vec<String>) -> Result<Self, BusError> {
        let field = field.into();
        if field.is_empty()
            || field.len() > 64
            || !field
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || values.is_empty()
            || values.len() > 64
            || values.iter().any(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            })
        {
            return Err(BusError::InvalidResourceCall);
        }
        Ok(Self { field, values })
    }

    /// Borrow the exact indexed field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Borrow the exact accepted values.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

impl core::fmt::Debug for ResourceFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceFilter")
            .field("value_count", &self.values.len())
            .finish()
    }
}

/// Named or nameless List/Watch selector.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceQuery {
    resource_types: Vec<ResourceTypeName>,
    resource_names: Vec<ResourceName>,
    filters: Vec<ResourceFilter>,
}

impl ResourceQuery {
    /// Construct a bounded query without rewriting selector order or filters.
    pub fn new(
        resource_types: Vec<ResourceTypeName>,
        resource_names: Vec<ResourceName>,
        filters: Vec<ResourceFilter>,
    ) -> Result<Self, BusError> {
        if resource_types.is_empty()
            || resource_types.len() > 64
            || resource_names.len() > 64
            || filters.len() > 64
        {
            return Err(BusError::InvalidResourceCall);
        }
        Ok(Self {
            resource_types,
            resource_names,
            filters,
        })
    }

    /// Borrow the ResourceType selector in its exact received order.
    pub fn resource_types(&self) -> &[ResourceTypeName] {
        &self.resource_types
    }

    /// Borrow the optional name selector in its exact received order.
    pub fn resource_names(&self) -> &[ResourceName] {
        &self.resource_names
    }

    /// Borrow the exact filters.
    pub fn filters(&self) -> &[ResourceFilter] {
        &self.filters
    }
}

impl core::fmt::Debug for ResourceQuery {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceQuery")
            .field("resource_type_count", &self.resource_types.len())
            .field("resource_name_count", &self.resource_names.len())
            .field("filter_count", &self.filters.len())
            .finish()
    }
}

/// Closed resource-service call. Authorization targets are derived from this value.
#[derive(Clone, PartialEq, Eq)]
pub enum ResourceCall {
    Get(ResourceRef),
    List(ResourceQuery),
    Watch(ResourceQuery),
    Create(ResourceRef),
    UpdateSpec(ResourceRef),
    UpdateStatus(ResourceRef),
    UpdateMetadata(ResourceRef),
    UpdateFinalizers(ResourceRef),
    Delete(ResourceRef),
    CommitBatch(Vec<(ResourceRef, ResourceVerb)>),
    ResolveRef(ResourceRef),
    InspectSchema(ResourceTypeName),
    Upgrade(ResourceRef),
}

impl ResourceCall {
    pub(crate) fn authorization_request(
        &self,
        zone: ZoneId,
    ) -> Result<AuthorizationRequest, BusError> {
        let (method, targets) = match self {
            Self::Get(target) => (
                ApiMethod::Get,
                vec![exact_target(target, ResourceVerb::Get, None)],
            ),
            Self::List(query) => (ApiMethod::List, query_targets(query, ResourceVerb::List)),
            Self::Watch(query) => (ApiMethod::Watch, query_targets(query, ResourceVerb::Watch)),
            Self::Create(target) => (
                ApiMethod::Create,
                vec![exact_target(target, ResourceVerb::Create, None)],
            ),
            Self::UpdateSpec(target) => (
                ApiMethod::UpdateSpec,
                vec![exact_target(target, ResourceVerb::UpdateSpec, None)],
            ),
            Self::UpdateStatus(target) => (
                ApiMethod::UpdateStatus,
                vec![exact_target(
                    target,
                    ResourceVerb::UpdateStatus,
                    Some("status"),
                )],
            ),
            Self::UpdateMetadata(target) => (
                ApiMethod::UpdateMetadata,
                vec![exact_target(target, ResourceVerb::UpdateMetadata, None)],
            ),
            Self::UpdateFinalizers(target) => (
                ApiMethod::UpdateFinalizers,
                vec![exact_target(
                    target,
                    ResourceVerb::UpdateFinalizers,
                    Some("finalizers"),
                )],
            ),
            Self::Delete(target) => (
                ApiMethod::Delete,
                vec![exact_target(target, ResourceVerb::Delete, None)],
            ),
            Self::CommitBatch(mutations) => {
                if mutations.is_empty()
                    || mutations.len() > 128
                    || mutations.iter().any(|(_, verb)| {
                        !matches!(
                            verb,
                            ResourceVerb::Create
                                | ResourceVerb::UpdateSpec
                                | ResourceVerb::UpdateStatus
                                | ResourceVerb::UpdateMetadata
                                | ResourceVerb::UpdateFinalizers
                                | ResourceVerb::Delete
                        )
                    })
                {
                    return Err(BusError::InvalidResourceCall);
                }
                (
                    ApiMethod::CommitBatch,
                    mutations
                        .iter()
                        .map(|(target, verb)| {
                            let subresource = match verb {
                                ResourceVerb::UpdateStatus => Some("status"),
                                ResourceVerb::UpdateFinalizers => Some("finalizers"),
                                _ => None,
                            };
                            exact_target(target, *verb, subresource)
                        })
                        .collect(),
                )
            }
            Self::ResolveRef(target) => (
                ApiMethod::ResolveRef,
                vec![exact_target(target, ResourceVerb::Get, None)],
            ),
            Self::InspectSchema(resource_type) => (
                ApiMethod::InspectSchema,
                vec![AuthorizationTarget {
                    resource_type: resource_type.clone(),
                    resource_name: None,
                    verb: ResourceVerb::Get,
                    subresource: Some("schema".to_owned()),
                    execution_ref: None,
                }],
            ),
            Self::Upgrade(target) => (
                ApiMethod::Upgrade,
                vec![exact_target(target, ResourceVerb::UpdateSpec, None)],
            ),
        };
        Ok(AuthorizationRequest {
            method,
            zone,
            targets,
        })
    }

    pub(crate) fn expected_member(&self) -> &'static str {
        resource_operation(self.api_method()).member
    }

    const fn api_method(&self) -> ApiMethod {
        match self {
            Self::Get(_) => ApiMethod::Get,
            Self::List(_) => ApiMethod::List,
            Self::Watch(_) => ApiMethod::Watch,
            Self::Create(_) => ApiMethod::Create,
            Self::UpdateSpec(_) => ApiMethod::UpdateSpec,
            Self::UpdateStatus(_) => ApiMethod::UpdateStatus,
            Self::UpdateMetadata(_) => ApiMethod::UpdateMetadata,
            Self::UpdateFinalizers(_) => ApiMethod::UpdateFinalizers,
            Self::Delete(_) => ApiMethod::Delete,
            Self::CommitBatch(_) => ApiMethod::CommitBatch,
            Self::ResolveRef(_) => ApiMethod::ResolveRef,
            Self::InspectSchema(_) => ApiMethod::InspectSchema,
            Self::Upgrade(_) => ApiMethod::Upgrade,
        }
    }

    fn session_target(&self) -> Option<&ResourceRef> {
        match self {
            Self::Get(target)
            | Self::Create(target)
            | Self::UpdateSpec(target)
            | Self::UpdateStatus(target)
            | Self::UpdateMetadata(target)
            | Self::UpdateFinalizers(target)
            | Self::Delete(target)
            | Self::ResolveRef(target)
            | Self::Upgrade(target) => Some(target),
            Self::List(_) | Self::Watch(_) | Self::CommitBatch(_) | Self::InspectSchema(_) => None,
        }
    }

    fn matches_route_target(&self, route_target: &RouteTarget) -> bool {
        let RouteTarget::Resource(route_target) = route_target else {
            return true;
        };
        match self {
            Self::Get(target)
            | Self::Create(target)
            | Self::UpdateSpec(target)
            | Self::UpdateStatus(target)
            | Self::UpdateMetadata(target)
            | Self::UpdateFinalizers(target)
            | Self::Delete(target)
            | Self::ResolveRef(target)
            | Self::Upgrade(target) => target == route_target,
            Self::List(_) | Self::Watch(_) | Self::CommitBatch(_) | Self::InspectSchema(_) => false,
        }
    }
}

impl core::fmt::Debug for ResourceCall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match self {
            Self::Get(_) => "Get",
            Self::List(_) => "List",
            Self::Watch(_) => "Watch",
            Self::Create(_) => "Create",
            Self::UpdateSpec(_) => "UpdateSpec",
            Self::UpdateStatus(_) => "UpdateStatus",
            Self::UpdateMetadata(_) => "UpdateMetadata",
            Self::UpdateFinalizers(_) => "UpdateFinalizers",
            Self::Delete(_) => "Delete",
            Self::CommitBatch(_) => "CommitBatch",
            Self::ResolveRef(_) => "ResolveRef",
            Self::InspectSchema(_) => "InspectSchema",
            Self::Upgrade(_) => "Upgrade",
        };
        write!(f, "ResourceCall::{kind}(<redacted>)")
    }
}

fn exact_target(
    target: &ResourceRef,
    verb: ResourceVerb,
    subresource: Option<&str>,
) -> AuthorizationTarget {
    AuthorizationTarget {
        resource_type: target.resource_type().clone(),
        resource_name: Some(target.name().clone()),
        verb,
        subresource: subresource.map(str::to_owned),
        execution_ref: None,
    }
}

fn query_targets(query: &ResourceQuery, verb: ResourceVerb) -> Vec<AuthorizationTarget> {
    query
        .resource_types
        .iter()
        .flat_map(|resource_type| {
            if query.resource_names.is_empty() {
                vec![AuthorizationTarget {
                    resource_type: resource_type.clone(),
                    resource_name: None,
                    verb,
                    subresource: None,
                    execution_ref: None,
                }]
            } else {
                query
                    .resource_names
                    .iter()
                    .map(|name| AuthorizationTarget {
                        resource_type: resource_type.clone(),
                        resource_name: Some(name.clone()),
                        verb,
                        subresource: None,
                        execution_ref: None,
                    })
                    .collect()
            }
        })
        .collect()
}

/// Method invocation delivered only after exact route and authorization checks.
pub struct DeliveredInvocation {
    route: RouteKey,
    operation: OperationSpec,
    resource_call: Option<ResourceCall>,
    payload: Vec<u8>,
    cancellation: Cancellation,
}

impl DeliveredInvocation {
    /// Borrow the exact route.
    pub const fn route(&self) -> &RouteKey {
        &self.route
    }

    /// Borrow the operation metadata.
    pub const fn operation(&self) -> &OperationSpec {
        &self.operation
    }

    /// Borrow the exact resource call, when this is a resource-service request.
    pub const fn resource_call(&self) -> Option<&ResourceCall> {
        self.resource_call.as_ref()
    }

    /// Borrow the opaque service payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Borrow cancellation state.
    pub const fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

impl core::fmt::Debug for DeliveredInvocation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeliveredInvocation")
            .field("route", &self.route)
            .field("operation", &self.operation)
            .field("resource_call", &self.resource_call)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

/// Stream open delivered only after exact route and authorization checks.
pub struct DeliveredStream {
    route: RouteKey,
    operation: OperationSpec,
    resource_call: Option<ResourceCall>,
    incoming: IncomingStream,
    cancellation: Cancellation,
}

impl DeliveredStream {
    /// Borrow the exact route.
    pub const fn route(&self) -> &RouteKey {
        &self.route
    }

    /// Borrow the operation metadata.
    pub const fn operation(&self) -> &OperationSpec {
        &self.operation
    }

    /// Borrow the exact resource call, when this is a resource stream.
    pub const fn resource_call(&self) -> Option<&ResourceCall> {
        self.resource_call.as_ref()
    }

    /// Borrow cancellation state.
    pub const fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// Consume the dispatch and retain the destination stream reader.
    pub fn into_incoming(self) -> IncomingStream {
        self.incoming
    }
}

impl core::fmt::Debug for DeliveredStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeliveredStream")
            .field("route", &self.route)
            .field("operation", &self.operation)
            .field("resource_call", &self.resource_call)
            .field("incoming", &self.incoming)
            .finish()
    }
}

struct BusCore {
    zone: ZoneId,
    registry: Mutex<Registry>,
    authorizer: BusAuthorizer,
    operations: Mutex<OperationTable>,
    streams: Arc<StreamBridge>,
    clock: Arc<dyn BusClock>,
    max_payload_bytes: usize,
    observer: Arc<dyn BusObserver>,
    #[cfg(test)]
    invocation_hooks: Mutex<InvocationHooks>,
}

#[cfg(test)]
#[derive(Default)]
struct InvocationHooks {
    after_resolve: Option<Arc<InvocationHook>>,
    before_invoke: Option<Arc<InvocationHook>>,
}

#[cfg(test)]
struct InvocationHook {
    reached: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl BusCore {
    fn cleanup_session(&self, session: SessionId) {
        // Revoke first so no new operation can acquire a lease after the
        // cancellation sweep has started.
        self.lock_registry().remove(session);
        self.lock_operations().cancel_session(session);
        self.streams.cancel_session(session);
    }

    fn observe_error(&self, event: BusEvent, error: &BusError) {
        self.observer
            .record(event, BusFailureReason::from_error(error));
    }

    fn lock_registry(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_operations(&self) -> MutexGuard<'_, OperationTable> {
        self.operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Administration handle for one self-contained Zone bus.
pub struct ZoneBus {
    core: Arc<BusCore>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusEvent {
    Invoke,
    OpenStream,
    Cancel,
    Cleanup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusFailureReason {
    Authorization,
    Route,
    Session,
    Capacity,
    Backpressure,
    RouteRevoked,
    Deadline,
    Cancelled,
    Authentication,
    Generation,
    Transport,
    Protocol,
    Endpoint,
    Abandoned,
}

impl BusFailureReason {
    const fn from_error(error: &BusError) -> Self {
        match error {
            BusError::Authorization(_) => Self::Authorization,
            BusError::InvalidResourceCall | BusError::RouteShape | BusError::Registry(_) => {
                Self::Route
            }
            BusError::SessionMismatch | BusError::SessionClosed => Self::Session,
            BusError::Cancelled => Self::Cancelled,
            BusError::Operation(OperationError::DeadlineExceeded) => Self::Deadline,
            BusError::Operation(OperationError::RouteRevoked) => Self::RouteRevoked,
            BusError::Operation(OperationError::CapacityExceeded)
            | BusError::Operation(OperationError::SessionCapacityExceeded)
            | BusError::Stream(StreamError::StreamCapacityExceeded)
            | BusError::Stream(StreamError::PrincipalCapacityExceeded) => Self::Capacity,
            BusError::Operation(_) | BusError::Stream(_) => Self::Backpressure,
            BusError::Endpoint(EndpointError::Session(failure)) => match failure.class() {
                crate::registry::EndpointFailureClass::Authentication => Self::Authentication,
                crate::registry::EndpointFailureClass::Authorization => Self::Authorization,
                crate::registry::EndpointFailureClass::Generation => Self::Generation,
                crate::registry::EndpointFailureClass::Backpressure => Self::Backpressure,
                crate::registry::EndpointFailureClass::Deadline => Self::Deadline,
                crate::registry::EndpointFailureClass::Transport => Self::Transport,
                crate::registry::EndpointFailureClass::Protocol => Self::Protocol,
                crate::registry::EndpointFailureClass::Internal => Self::Endpoint,
            },
            BusError::Endpoint(_) | BusError::InvalidConfig => Self::Endpoint,
        }
    }
}

pub trait BusObserver: Send + Sync {
    fn record(&self, event: BusEvent, reason: BusFailureReason);
}

#[derive(Debug, Default)]
pub struct NoopBusObserver;

impl BusObserver for NoopBusObserver {
    fn record(&self, _event: BusEvent, _reason: BusFailureReason) {}
}

impl ZoneBus {
    /// Construct a bus with a process-monotonic clock.
    pub fn new(
        zone: ZoneId,
        authorizer: BusAuthorizer,
        config: BusConfig,
    ) -> Result<(Self, ZoneRegistrar), BusError> {
        Self::with_clock(zone, authorizer, config, Arc::new(SystemClock::new()))
    }

    pub fn with_observer(
        zone: ZoneId,
        authorizer: BusAuthorizer,
        config: BusConfig,
        observer: Arc<dyn BusObserver>,
    ) -> Result<(Self, ZoneRegistrar), BusError> {
        Self::with_clock_and_observer(
            zone,
            authorizer,
            config,
            Arc::new(SystemClock::new()),
            observer,
        )
    }

    /// Construct a bus with an injected monotonic clock.
    pub fn with_clock(
        zone: ZoneId,
        authorizer: BusAuthorizer,
        config: BusConfig,
        clock: Arc<dyn BusClock>,
    ) -> Result<(Self, ZoneRegistrar), BusError> {
        Self::with_clock_and_observer(zone, authorizer, config, clock, Arc::new(NoopBusObserver))
    }

    pub fn with_clock_and_observer(
        zone: ZoneId,
        authorizer: BusAuthorizer,
        config: BusConfig,
        clock: Arc<dyn BusClock>,
        observer: Arc<dyn BusObserver>,
    ) -> Result<(Self, ZoneRegistrar), BusError> {
        if config.max_payload_bytes == 0
            || config.max_routes_per_session == 0
            || config.max_total_routes == 0
            || config.max_routes_per_session > config.max_total_routes
        {
            return Err(BusError::InvalidConfig);
        }
        let operations =
            OperationTable::new(config.max_operations, config.max_operations_per_session)?;
        let streams = StreamBridge::new(config.stream_limits)?;
        let core = Arc::new(BusCore {
            registry: Mutex::new(Registry::new(
                zone.clone(),
                config.max_routes_per_session,
                config.max_total_routes,
            )),
            zone,
            authorizer,
            operations: Mutex::new(operations),
            streams,
            clock,
            max_payload_bytes: config.max_payload_bytes,
            observer,
            #[cfg(test)]
            invocation_hooks: Mutex::new(InvocationHooks::default()),
        });
        Ok((
            Self {
                core: Arc::clone(&core),
            },
            ZoneRegistrar {
                core,
                component_admission: ComponentSessionRegistrar {
                    identity: Arc::new(ComponentSessionAdmissionIdentity),
                },
            },
        ))
    }

    /// Atomically install a new native policy and trusted runtime state.
    pub fn replace_policy(
        &self,
        policy: PolicySet,
        state: AuthorizationState,
    ) -> Result<(), BusError> {
        self.core.authorizer.replace_policy(policy, state)?;
        Ok(())
    }

    /// Fail closed for all new work while durable policy is unavailable.
    pub fn mark_policy_unavailable(&self) {
        self.core.authorizer.mark_policy_unavailable();
    }
}

impl core::fmt::Debug for ZoneBus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ZoneBus(<redacted>)")
    }
}

/// Single, non-cloneable authority that consumes authenticated registrations.
pub struct ZoneRegistrar {
    core: Arc<BusCore>,
    component_admission: ComponentSessionRegistrar,
}

struct ComponentSessionAdmissionIdentity;

struct ComponentSessionRegistrar {
    identity: Arc<ComponentSessionAdmissionIdentity>,
}

/// Single-use proof that a ComponentSession candidate was minted by one
/// concrete Zone registrar.
pub struct ComponentSessionAdmission {
    identity: Arc<ComponentSessionAdmissionIdentity>,
}

impl core::fmt::Debug for ComponentSessionAdmission {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ComponentSessionAdmission(<redacted>)")
    }
}

impl SessionRegistrationCapability<ComponentSessionRegistrar> for ComponentSessionAdmission {
    type Error = BusError;

    fn consume(self, registrar: &ComponentSessionRegistrar) -> Result<(), Self::Error> {
        if Arc::ptr_eq(&self.identity, &registrar.identity) {
            Ok(())
        } else {
            Err(BusError::SessionMismatch)
        }
    }
}

impl ZoneRegistrar {
    #[cfg(test)]
    pub(crate) fn register(
        &mut self,
        registration: SessionRegistration,
    ) -> Result<BusIngress, BusError> {
        let context = registration.context().ok_or(BusError::SessionMismatch)?;
        self.core
            .authorizer
            .authorize_connect(context, &self.core.zone)?;
        let session = self.core.lock_registry().register(registration)?;
        Ok(BusIngress {
            core: Arc::clone(&self.core),
            session,
            closed: false,
        })
    }

    /// Replace a session with the exact next reconnect generation.
    #[cfg(test)]
    pub(crate) fn reconnect(
        &mut self,
        mut previous: BusIngress,
        registration: SessionRegistration,
    ) -> Result<BusIngress, BusError> {
        if !Arc::ptr_eq(&self.core, &previous.core) || previous.closed {
            return Err(BusError::SessionMismatch);
        }
        let context = registration.context().ok_or(BusError::SessionMismatch)?;
        self.core
            .authorizer
            .authorize_connect(context, &self.core.zone)?;
        let session = self
            .core
            .lock_registry()
            .reconnect(previous.session, registration)?;
        self.core.lock_operations().cancel_session(previous.session);
        self.core.streams.cancel_session(previous.session);
        previous.closed = true;
        Ok(BusIngress {
            core: Arc::clone(&self.core),
            session,
            closed: false,
        })
    }
}

struct ComponentEndpoint {
    session: AsyncMutex<AuthenticatedComponentSession<()>>,
    clock: Arc<dyn BusClock>,
    locality: d2b_contracts::v3::Locality,
    generation: u64,
    cancellation: SessionCancellationHandle,
    active: Mutex<BTreeMap<OperationId, d2b_session::contract::RequestId>>,
}

#[async_trait::async_trait]
impl crate::registry::BusEndpoint for ComponentEndpoint {
    async fn authorize(
        &self,
        route: &RouteKey,
        verb: d2b_resource_api::authz::SessionVerb,
        target: Option<&ResourceRef>,
        now_tick: u64,
    ) -> Result<(), EndpointError> {
        let request = if self.locality == d2b_contracts::v3::Locality::AdjacentZone {
            SessionAuthorizationRequest::relay(
                route.service().clone(),
                route.member().as_str(),
                route.zone().clone(),
                target.cloned(),
                verb,
                route.zone().clone(),
            )
        } else {
            SessionAuthorizationRequest::new(
                verb,
                route.service().clone(),
                route.member().as_str(),
                route.zone().clone(),
                target.cloned(),
            )
        }
        .map_err(|_| EndpointError::Rejected)?;
        self.session
            .lock()
            .await
            .authorize(request, now_tick)
            .await
            .map(|_| ())
            .map_err(EndpointError::from)
    }

    async fn invoke(&self, request: DeliveredInvocation) -> Result<BusResponse, EndpointError> {
        let ordinary = d2b_resource_api::authz::SessionVerb::Invoke;
        let operation = SessionOperation::method(
            request.route().service().clone(),
            request.route().member().as_str(),
        )
        .map_err(|_| EndpointError::Rejected)?;
        let verb = operation.required_verb(ordinary);
        let now_tick = self.clock.now_tick();
        let request_id = ttrpc_request_id(self.generation, request.payload())
            .map_err(|_| EndpointError::Rejected)?;
        let target = request
            .resource_call()
            .and_then(ResourceCall::session_target)
            .cloned();
        let authorization = if self.locality == d2b_contracts::v3::Locality::AdjacentZone {
            SessionAuthorizationRequest::relay(
                request.route().service().clone(),
                request.route().member().as_str(),
                request.route().zone().clone(),
                target,
                verb,
                request.route().zone().clone(),
            )
        } else {
            SessionAuthorizationRequest::new(
                verb,
                request.route().service().clone(),
                request.route().member().as_str(),
                request.route().zone().clone(),
                target,
            )
        }
        .map_err(|_| EndpointError::Rejected)?;
        let mut session = self.session.lock().await;
        let permit = session
            .authorize(authorization, now_tick)
            .await
            .map_err(EndpointError::from)?;
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(request.operation().id().clone(), request_id.clone());
        if let Err(error) = session
            .start_authorized_ttrpc(
                permit,
                request_id.clone(),
                request.payload().to_vec(),
                now_tick,
            )
            .await
        {
            self.active
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(request.operation().id());
            return Err(EndpointError::from(error));
        }
        let response = loop {
            let response = session.receive_ttrpc().await.map_err(EndpointError::from)?;
            let response_id = ttrpc_request_id(self.generation, &response)
                .map_err(|_| EndpointError::Rejected)?;
            if response_id == request_id {
                break response;
            }
        };
        let _ = session.complete_ttrpc(request_id).await;
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(request.operation().id());
        Ok(BusResponse::new(response))
    }

    async fn open_stream(&self, _request: DeliveredStream) -> Result<(), EndpointError> {
        Err(EndpointError::Unavailable)
    }

    async fn cancel(&self, operation: &OperationId) -> Result<(), EndpointError> {
        let request_id = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(operation);
        if let Some(request_id) = request_id {
            self.cancellation
                .cancel(request_id)
                .await
                .map_err(EndpointError::from)?;
        }
        Ok(())
    }
}

impl ZoneRegistrar {
    /// Mint a single-use acceptor bound to this registrar instance.
    pub fn component_session_acceptor(
        &self,
        policy: EndpointPolicy,
        authority: Box<dyn SessionAuthority>,
    ) -> d2b_session::Result<SessionAcceptor<ComponentSessionAdmission>> {
        SessionAcceptor::new(
            policy,
            self.core.zone.clone(),
            authority,
            ComponentSessionAdmission {
                identity: Arc::clone(&self.component_admission.identity),
            },
        )
    }

    /// Consume an authenticated candidate and install it only after native
    /// connect authorization succeeds.
    pub async fn register_component_session(
        &mut self,
        session: AuthenticatedComponentSession<ComponentSessionAdmission>,
    ) -> Result<BusIngress, BusError> {
        let session = session.consume_registration(&self.component_admission)?;
        let binding = session.route_binding();
        if binding.zone() != &self.core.zone {
            return Err(BusError::SessionMismatch);
        }
        self.core
            .authorizer
            .authorize_connect(binding.context(), &self.core.zone)?;
        let routes = routes_for_admitted_session(&binding)?;
        let cancellation = session.cancellation_handle();
        let endpoint: Arc<dyn crate::registry::BusEndpoint> = Arc::new(ComponentEndpoint {
            session: AsyncMutex::new(session),
            clock: Arc::clone(&self.core.clock),
            locality: binding.locality(),
            generation: binding.reconnect_generation().get(),
            cancellation,
            active: Mutex::new(BTreeMap::new()),
        });
        let registration = SessionRegistration::admitted(binding, routes, endpoint);
        let session = self.core.lock_registry().register(registration)?;
        Ok(BusIngress {
            core: Arc::clone(&self.core),
            session,
            closed: false,
        })
    }

    pub async fn reconnect_component_session(
        &mut self,
        mut previous: BusIngress,
        session: AuthenticatedComponentSession<ComponentSessionAdmission>,
    ) -> Result<BusIngress, BusError> {
        if !Arc::ptr_eq(&self.core, &previous.core) || previous.closed {
            return Err(BusError::SessionMismatch);
        }
        let session = session.consume_registration(&self.component_admission)?;
        let binding = session.route_binding();
        if binding.zone() != &self.core.zone {
            return Err(BusError::SessionMismatch);
        }
        self.core
            .authorizer
            .authorize_connect(binding.context(), &self.core.zone)?;
        let routes = routes_for_admitted_session(&binding)?;
        let cancellation = session.cancellation_handle();
        let endpoint: Arc<dyn crate::registry::BusEndpoint> = Arc::new(ComponentEndpoint {
            session: AsyncMutex::new(session),
            clock: Arc::clone(&self.core.clock),
            locality: binding.locality(),
            generation: binding.reconnect_generation().get(),
            cancellation,
            active: Mutex::new(BTreeMap::new()),
        });
        let registration = SessionRegistration::admitted(binding, routes, endpoint);
        let session = self
            .core
            .lock_registry()
            .reconnect(previous.session, registration)?;
        self.core.lock_operations().cancel_session(previous.session);
        self.core.streams.cancel_session(previous.session);
        previous.closed = true;
        Ok(BusIngress {
            core: Arc::clone(&self.core),
            session,
            closed: false,
        })
    }

    pub async fn disconnect_component_session(
        &mut self,
        registration: BusIngress,
    ) -> Result<(), BusError> {
        self.revoke(registration)
    }
}

fn routes_for_admitted_session(
    binding: &AuthenticatedSessionRouteBinding,
) -> Result<Vec<RouteKey>, BusError> {
    if binding.subject_ref().resource_type().as_str() != "Provider" {
        return Ok(Vec::new());
    }
    let target_ref = binding
        .provider_ref()
        .unwrap_or_else(|| binding.subject_ref())
        .clone();
    let target = if target_ref.resource_type().as_str() == "Provider" {
        RouteTarget::provider(target_ref)?
    } else {
        RouteTarget::resource(target_ref)?
    };
    let generations = crate::registry::RouteGenerations::new(
        binding.provider_generation(),
        binding.controller_generation(),
        binding.reconnect_generation(),
    );
    GENERATED_OPERATION_CATALOG
        .iter()
        .filter(|entry| entry.service == binding.service().as_str())
        .map(|entry| {
            let member = if entry.kind == OperationKind::Stream {
                crate::registry::RouteMember::stream(entry.member)?
            } else {
                crate::registry::RouteMember::method(entry.member)?
            };
            Ok(RouteKey::new(
                binding.zone().clone(),
                binding.service().clone(),
                member,
                target.clone(),
                binding.schema().clone(),
                generations,
            ))
        })
        .collect()
}

impl ZoneRegistrar {
    /// Revoke a session, its routes, operations, and streams.
    pub fn revoke(&mut self, mut ingress: BusIngress) -> Result<(), BusError> {
        if !Arc::ptr_eq(&self.core, &ingress.core) || ingress.closed {
            return Err(BusError::SessionMismatch);
        }
        self.core.cleanup_session(ingress.session);
        ingress.closed = true;
        Ok(())
    }
}

impl core::fmt::Debug for ZoneRegistrar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ZoneRegistrar(<redacted>)")
    }
}

/// Non-cloneable ingress bound to one consumed authenticated session.
pub struct BusIngress {
    core: Arc<BusCore>,
    session: SessionId,
    closed: bool,
}

struct OperationLease {
    core: Arc<BusCore>,
    source: SessionId,
    operation: OperationId,
    armed: bool,
}

impl OperationLease {
    fn new(core: Arc<BusCore>, source: SessionId, operation: OperationId) -> Self {
        Self {
            core,
            source,
            operation,
            armed: true,
        }
    }

    fn finish(&mut self) -> Result<(), BusError> {
        if !self.armed {
            return Ok(());
        }
        self.armed = false;
        self.core.lock_operations().finish(
            &self.operation,
            self.source,
            self.core.clock.now_tick(),
        )?;
        Ok(())
    }

    fn abort(&mut self) -> Option<crate::operations::CancelTarget> {
        if !self.armed {
            return None;
        }
        self.armed = false;
        self.core
            .lock_operations()
            .abort(&self.operation, self.source)
    }
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        let Some(target) = self.abort() else {
            return;
        };
        if target.route.generations().session() != target.generation {
            self.core
                .observe_error(BusEvent::Cancel, &BusError::SessionMismatch);
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let core = Arc::clone(&self.core);
            let operation = self.operation.clone();
            runtime.spawn(async move {
                if let Err(error) = target.endpoint.cancel(&operation).await {
                    core.observe_error(BusEvent::Cancel, &BusError::Endpoint(error));
                }
            });
        } else {
            self.core
                .observer
                .record(BusEvent::Cancel, BusFailureReason::Abandoned);
        }
    }
}

impl BusIngress {
    async fn authorize_route(
        &self,
        route: &RouteKey,
        resource_call: Option<&ResourceCall>,
        verb: SessionVerb,
        stream: bool,
    ) -> Result<(), BusError> {
        let source = self.core.lock_registry().source(self.session)?;
        if let Some(context) = source.context.as_ref() {
            self.core
                .authorizer
                .authorize_dispatch(context, route, resource_call, stream)?;
        }
        if source.session_authorization {
            source
                .endpoint
                .authorize(
                    route,
                    verb,
                    resource_call.and_then(ResourceCall::session_target),
                    self.core.clock.now_tick(),
                )
                .await
                .map_err(BusError::Endpoint)
        } else {
            Ok(())
        }
    }

    /// Invoke a non-resource exact service method.
    pub async fn invoke(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        payload: Vec<u8>,
    ) -> Result<BusResponse, BusError> {
        let result = self.invoke_inner(route, operation, None, payload).await;
        if let Err(error) = &result {
            self.core.observe_error(BusEvent::Invoke, error);
        }
        result
    }

    /// Invoke an exact ResourceService method.
    pub async fn invoke_resource(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        call: ResourceCall,
        payload: Vec<u8>,
    ) -> Result<BusResponse, BusError> {
        let result = self
            .invoke_inner(route, operation, Some(call), payload)
            .await;
        if let Err(error) = &result {
            self.core.observe_error(BusEvent::Invoke, error);
        }
        result
    }

    async fn invoke_inner(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        resource_call: Option<ResourceCall>,
        payload: Vec<u8>,
    ) -> Result<BusResponse, BusError> {
        self.ensure_open()?;
        if !route.member().is_method() || payload.len() > self.core.max_payload_bytes {
            return Err(BusError::RouteShape);
        }
        validate_resource_route(&route, resource_call.as_ref())?;

        let ordinary = SessionVerb::Invoke;
        let session_operation =
            SessionOperation::method(route.service().clone(), route.member().as_str())
                .map_err(|_| BusError::RouteShape)?;
        self.authorize_route(
            &route,
            resource_call.as_ref(),
            session_operation.required_verb(ordinary),
            false,
        )
        .await?;
        let destination = self.core.lock_registry().resolve(&route)?;
        #[cfg(test)]
        self.wait_for_invocation_hook(true).await;
        let endpoint = destination.endpoint();
        let now = self.core.clock.now_tick();
        let cancellation = self.core.lock_operations().begin(
            &operation,
            self.session,
            destination,
            route.clone(),
            now,
        )?;
        let mut lease =
            OperationLease::new(Arc::clone(&self.core), self.session, operation.id().clone());
        let delivered = DeliveredInvocation {
            route,
            operation: operation.clone(),
            resource_call,
            payload,
            cancellation: cancellation.clone(),
        };
        #[cfg(test)]
        self.wait_for_invocation_hook(false).await;
        let remaining = operation.deadline_tick().saturating_sub(now);
        enum InvokeOutcome {
            Cancelled,
            Deadline,
            Response(Result<BusResponse, EndpointError>),
        }
        let outcome = tokio::select! {
            biased;
            () = cancellation.cancelled() => InvokeOutcome::Cancelled,
            () = tokio::time::sleep(Duration::from_millis(remaining)) => InvokeOutcome::Deadline,
            response = endpoint.invoke(delivered) => InvokeOutcome::Response(response),
        };
        let response = match outcome {
            InvokeOutcome::Response(response) => response,
            InvokeOutcome::Cancelled => return Err(BusError::Cancelled),
            InvokeOutcome::Deadline => {
                let _ = lease.abort();
                if let Err(error) = endpoint.cancel(operation.id()).await {
                    self.core
                        .observe_error(BusEvent::Cancel, &BusError::Endpoint(error));
                }
                return Err(BusError::Operation(OperationError::DeadlineExceeded));
            }
        };
        lease.finish()?;
        let response = response.map_err(BusError::Endpoint)?;
        if response.as_bytes().len() > self.core.max_payload_bytes {
            return Err(BusError::RouteShape);
        }
        Ok(response)
    }

    /// Open a non-resource named stream.
    pub async fn open_stream(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        stream: StreamName,
        initial_credit: usize,
    ) -> Result<BusStream, BusError> {
        let result = self
            .open_stream_inner(route, operation, None, stream, initial_credit)
            .await;
        if let Err(error) = &result {
            self.core.observe_error(BusEvent::OpenStream, error);
        }
        result
    }

    /// Open a resource-backed named stream while preserving its selector.
    pub async fn open_resource_stream(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        call: ResourceCall,
        stream: StreamName,
        initial_credit: usize,
    ) -> Result<BusStream, BusError> {
        let result = self
            .open_stream_inner(route, operation, Some(call), stream, initial_credit)
            .await;
        if let Err(error) = &result {
            self.core.observe_error(BusEvent::OpenStream, error);
        }
        result
    }

    async fn open_stream_inner(
        &self,
        route: RouteKey,
        operation: OperationSpec,
        resource_call: Option<ResourceCall>,
        stream: StreamName,
        initial_credit: usize,
    ) -> Result<BusStream, BusError> {
        self.ensure_open()?;
        if !route.member().is_stream() {
            return Err(BusError::RouteShape);
        }
        validate_resource_route(&route, resource_call.as_ref())?;
        self.authorize_route(
            &route,
            resource_call.as_ref(),
            SessionVerb::OpenStream,
            true,
        )
        .await?;
        let destination = self.core.lock_registry().resolve(&route)?;
        let source_principal = self.core.lock_registry().principal(self.session)?;
        let destination_session = destination.destination();
        let destination_principal = destination.destination_principal();
        let endpoint = destination.endpoint();
        let (outgoing, incoming) = self.core.streams.open(
            stream,
            self.session,
            source_principal,
            destination_session,
            destination_principal,
            initial_credit,
        )?;
        let now = self.core.clock.now_tick();
        let cancellation = self.core.lock_operations().begin(
            &operation,
            self.session,
            destination,
            route.clone(),
            now,
        )?;
        let lease =
            OperationLease::new(Arc::clone(&self.core), self.session, operation.id().clone());
        let dispatch = DeliveredStream {
            route,
            operation: operation.clone(),
            resource_call,
            incoming,
            cancellation: cancellation.clone(),
        };
        let remaining = operation.deadline_tick().saturating_sub(now);
        enum StreamOutcome {
            Cancelled,
            Deadline,
            Opened(Result<(), EndpointError>),
        }
        let outcome = tokio::select! {
            biased;
            () = cancellation.cancelled() => StreamOutcome::Cancelled,
            () = tokio::time::sleep(Duration::from_millis(remaining)) => StreamOutcome::Deadline,
            result = endpoint.open_stream(dispatch) => StreamOutcome::Opened(result),
        };
        match outcome {
            StreamOutcome::Opened(result) => result.map_err(BusError::Endpoint)?,
            StreamOutcome::Cancelled => return Err(BusError::Cancelled),
            StreamOutcome::Deadline => {
                let mut lease = lease;
                let _ = lease.abort();
                if let Err(error) = endpoint.cancel(operation.id()).await {
                    self.core
                        .observe_error(BusEvent::Cancel, &BusError::Endpoint(error));
                }
                return Err(BusError::Operation(OperationError::DeadlineExceeded));
            }
        }
        Ok(BusStream {
            lease: Some(lease),
            cancellation,
            outgoing: Some(outgoing),
        })
    }

    /// Cancel one operation owned by this exact ingress.
    pub async fn cancel(&self, operation: &OperationId) -> Result<(), BusError> {
        let result = self.cancel_inner(operation).await;
        if let Err(error) = &result {
            self.core.observe_error(BusEvent::Cancel, error);
        }
        result
    }

    async fn cancel_inner(&self, operation: &OperationId) -> Result<(), BusError> {
        self.ensure_open()?;
        let route = self
            .core
            .lock_operations()
            .route_for_cancel(operation, self.session)?;
        let source = self.core.lock_registry().source(self.session)?;
        if let Some(context) = source.context.as_ref() {
            self.core.authorizer.authorize_cancel(context, &route)?;
        }
        if source.session_authorization {
            source
                .endpoint
                .authorize(
                    &route,
                    SessionVerb::Cancel,
                    None,
                    self.core.clock.now_tick(),
                )
                .await
                .map_err(BusError::Endpoint)?;
        }
        let target = self
            .core
            .lock_operations()
            .cancel(operation, self.session)?;
        debug_assert!(target.cancellation.is_cancelled());
        if target.route.generations().session() != target.generation {
            return Err(BusError::SessionMismatch);
        }
        target
            .endpoint
            .cancel(operation)
            .await
            .map_err(BusError::Endpoint)?;
        Ok(())
    }

    fn ensure_open(&self) -> Result<(), BusError> {
        if self.closed {
            Err(BusError::SessionClosed)
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    async fn wait_for_invocation_hook(&self, after_resolve: bool) {
        let hook = {
            let hooks = self
                .core
                .invocation_hooks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if after_resolve {
                hooks.after_resolve.clone()
            } else {
                hooks.before_invoke.clone()
            }
        };
        if let Some(hook) = hook {
            hook.reached.notify_one();
            hook.release.notified().await;
        }
    }
}

impl core::fmt::Debug for BusIngress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BusIngress(<redacted>)")
    }
}

impl Drop for BusIngress {
    fn drop(&mut self) {
        if !self.closed {
            self.core
                .observer
                .record(BusEvent::Cleanup, BusFailureReason::Abandoned);
            self.core.cleanup_session(self.session);
            self.closed = true;
        }
    }
}

/// Source-side named stream that retains its operation lease.
pub struct BusStream {
    lease: Option<OperationLease>,
    cancellation: Cancellation,
    outgoing: Option<OutgoingStream>,
}

impl BusStream {
    /// Borrow the stream name.
    pub fn name(&self) -> &StreamName {
        self.outgoing
            .as_ref()
            .expect("open stream owns an outgoing handle")
            .name()
    }

    /// Send one frame after checking cancellation.
    pub async fn send(&self, payload: Vec<u8>) -> Result<(), BusError> {
        let result = if self.cancellation.is_cancelled() {
            Err(BusError::Cancelled)
        } else {
            self.outgoing.as_ref().map_or_else(
                || Err(BusError::SessionClosed),
                |outgoing| outgoing.send(payload).map_err(BusError::Stream),
            )
        };
        if let Err(error) = &result
            && let Some(lease) = self.lease.as_ref()
        {
            lease.core.observe_error(BusEvent::OpenStream, error);
        }
        result
    }

    /// Close the stream and complete its operation lease.
    pub async fn close(mut self) -> Result<(), BusError> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), BusError> {
        if let Some(mut outgoing) = self.outgoing.take() {
            outgoing.close();
            if self.cancellation.is_cancelled() {
                return Err(BusError::Cancelled);
            }
            if let Some(mut lease) = self.lease.take() {
                lease.finish()?;
            }
        }
        Ok(())
    }
}

impl core::fmt::Debug for BusStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BusStream(<redacted>)")
    }
}

impl Drop for BusStream {
    fn drop(&mut self) {
        let core = self.lease.as_ref().map(|lease| Arc::clone(&lease.core));
        if let Err(error) = self.finish()
            && let Some(core) = core
        {
            core.observe_error(BusEvent::Cleanup, &error);
        }
    }
}

fn validate_resource_route(
    route: &RouteKey,
    resource_call: Option<&ResourceCall>,
) -> Result<(), BusError> {
    match resource_call {
        Some(call)
            if route.service().as_str() == "d2b.resource.v3"
                && route.member().as_str() == call.expected_member()
                && call.matches_route_target(route.target()) =>
        {
            Ok(())
        }
        Some(_) => Err(BusError::InvalidResourceCall),
        None if route.service().as_str() != "d2b.resource.v3" => Ok(()),
        None => Err(BusError::InvalidResourceCall),
    }
}

/// Closed bus failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusError {
    InvalidConfig,
    InvalidResourceCall,
    RouteShape,
    SessionMismatch,
    SessionClosed,
    Cancelled,
    Authorization(AuthorizationError),
    Registry(RegistryError),
    Operation(OperationError),
    Stream(StreamError),
    Endpoint(EndpointError),
}

impl core::fmt::Display for BusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig => f.write_str("bus configuration is invalid"),
            Self::InvalidResourceCall => {
                f.write_str("resource call does not match its exact route")
            }
            Self::RouteShape => f.write_str("route member or payload shape is invalid"),
            Self::SessionMismatch => {
                f.write_str("session belongs to another registration authority")
            }
            Self::SessionClosed => f.write_str("session is closed"),
            Self::Cancelled => f.write_str("operation was cancelled"),
            Self::Authorization(error) => write!(f, "authorization failed: {error}"),
            Self::Registry(error) => write!(f, "route registry failed: {error}"),
            Self::Operation(error) => write!(f, "operation failed: {error}"),
            Self::Stream(error) => write!(f, "stream failed: {error}"),
            Self::Endpoint(error) => write!(f, "endpoint failed: {error}"),
        }
    }
}

impl std::error::Error for BusError {}

impl From<AuthorizationError> for BusError {
    fn from(value: AuthorizationError) -> Self {
        Self::Authorization(value)
    }
}

impl From<RegistryError> for BusError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}

impl From<OperationError> for BusError {
    fn from(value: OperationError) -> Self {
        Self::Operation(value)
    }
}

impl From<StreamError> for BusError {
    fn from(value: StreamError) -> Self {
        Self::Stream(value)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use d2b_contracts::v3::{
        AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
        EvidenceClass, Locality, ReconnectGeneration, ResourceGeneration, ResourceUid,
        SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose, TranscriptHash,
        TransportBinding, ZoneRevision,
    };
    use d2b_resource_api::authz::{
        ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
        NativeAuthorizer, PolicyRule, RelayGrantAuthority, SessionVerb,
    };
    use d2b_resource_store::PolicySnapshot;
    use tokio::sync::Notify;

    use super::*;
    use crate::registry::{BusEndpoint, RouteGenerations, RouteMember, RouteTarget};

    const CALLER_UID: &str = "11111111-1111-4111-8111-111111111111";
    const ENDPOINT_UID: &str = "22222222-2222-4222-8222-222222222222";

    type RecordedCall = (RouteKey, Option<ResourceCall>, Vec<u8>);

    struct RecordingEndpoint {
        calls: Mutex<Vec<RecordedCall>>,
        incoming: Mutex<Vec<IncomingStream>>,
        cancel_count: AtomicUsize,
        blocking: bool,
        response: Vec<u8>,
        started: Notify,
        release: Notify,
    }

    impl RecordingEndpoint {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                incoming: Mutex::new(Vec::new()),
                cancel_count: AtomicUsize::new(0),
                blocking: false,
                response: b"response".to_vec(),
                started: Notify::new(),
                release: Notify::new(),
            })
        }

        fn blocking() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                incoming: Mutex::new(Vec::new()),
                cancel_count: AtomicUsize::new(0),
                blocking: true,
                response: b"response".to_vec(),
                started: Notify::new(),
                release: Notify::new(),
            })
        }

        fn oversized() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                incoming: Mutex::new(Vec::new()),
                cancel_count: AtomicUsize::new(0),
                blocking: false,
                response: vec![0; DEFAULT_MAX_PAYLOAD_BYTES + 1],
                started: Notify::new(),
                release: Notify::new(),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl BusEndpoint for RecordingEndpoint {
        async fn invoke(&self, request: DeliveredInvocation) -> Result<BusResponse, EndpointError> {
            self.calls.lock().unwrap().push((
                request.route().clone(),
                request.resource_call().cloned(),
                request.payload().to_vec(),
            ));
            if self.blocking {
                self.started.notify_one();
                self.release.notified().await;
            }
            Ok(BusResponse::new(self.response.clone()))
        }

        async fn open_stream(&self, request: DeliveredStream) -> Result<(), EndpointError> {
            self.calls.lock().unwrap().push((
                request.route().clone(),
                request.resource_call().cloned(),
                Vec::new(),
            ));
            self.incoming.lock().unwrap().push(request.into_incoming());
            Ok(())
        }

        async fn cancel(&self, _operation: &OperationId) -> Result<(), EndpointError> {
            self.cancel_count.fetch_add(1, Ordering::AcqRel);
            self.release.notify_one();
            Ok(())
        }
    }

    struct Harness {
        bus: ZoneBus,
        registrar: ZoneRegistrar,
        caller: BusIngress,
        endpoint_ingress: BusIngress,
        endpoint: Arc<RecordingEndpoint>,
        route: RouteKey,
        subjects: Vec<BoundSubject>,
        clock: Arc<ManualClock>,
    }

    struct HarnessSpec<'a> {
        service: &'a str,
        member: RouteMember,
        caller_ref: &'a str,
        locality: Locality,
        evidence: EvidenceClass,
        session_verbs: Vec<SessionVerb>,
        resource_verbs: Vec<ResourceVerb>,
        endpoint: Arc<RecordingEndpoint>,
    }

    fn harness(spec: HarnessSpec<'_>) -> Harness {
        harness_with_config(spec, BusConfig::default())
    }

    fn harness_with_config(spec: HarnessSpec<'_>, config: BusConfig) -> Harness {
        harness_with_config_and_observer(spec, config, Arc::new(NoopBusObserver))
    }

    fn harness_with_config_and_observer(
        spec: HarnessSpec<'_>,
        config: BusConfig,
        observer: Arc<dyn BusObserver>,
    ) -> Harness {
        let zone = ZoneId::parse("dev").unwrap();
        let schema = fingerprint('1');
        let generations = RouteGenerations::new(
            Some(ResourceGeneration::new(2).unwrap()),
            Some(ControllerGeneration::new(3).unwrap()),
            ReconnectGeneration::new(1).unwrap(),
        );
        let route = RouteKey::new(
            zone.clone(),
            ServiceName::parse(spec.service).unwrap(),
            spec.member,
            RouteTarget::provider(ResourceRef::parse("Provider/system-core").unwrap()).unwrap(),
            schema.clone(),
            generations,
        );
        let caller = context(
            spec.caller_ref,
            CALLER_UID,
            spec.service,
            schema.clone(),
            generations,
            spec.locality,
            spec.evidence,
        );
        let endpoint_context = context(
            "Provider/system-core",
            ENDPOINT_UID,
            spec.service,
            schema,
            generations,
            Locality::Local,
            EvidenceClass::EnrolledKk,
        );
        let subjects = vec![bound_subject(&caller), bound_subject(&endpoint_context)];
        let policy = policy(1, &subjects, &spec.session_verbs, &spec.resource_verbs);
        let native = NativeAuthorizer::new(ApiCatalog::standard(), Some(policy)).unwrap();
        let authorizer = BusAuthorizer::new(native, state(1)).unwrap();
        let clock = Arc::new(ManualClock::new(1));
        let (bus, mut registrar) =
            ZoneBus::with_clock_and_observer(zone, authorizer, config, clock.clone(), observer)
                .unwrap();
        let endpoint_ingress = registrar
            .register(SessionRegistration::new(
                endpoint_context,
                vec![route.clone()],
                spec.endpoint.clone(),
            ))
            .unwrap();
        let caller = registrar
            .register(SessionRegistration::new(
                caller,
                Vec::new(),
                spec.endpoint.clone(),
            ))
            .unwrap();
        Harness {
            bus,
            registrar,
            caller,
            endpoint_ingress,
            endpoint: spec.endpoint,
            route,
            subjects,
            clock,
        }
    }

    #[derive(Default)]
    struct CaptureObserver(Mutex<Vec<(BusEvent, BusFailureReason)>>);

    impl BusObserver for CaptureObserver {
        fn record(&self, event: BusEvent, reason: BusFailureReason) {
            self.0.lock().unwrap().push((event, reason));
        }
    }

    fn context(
        subject_ref: &str,
        uid: &str,
        service: &str,
        schema: SchemaFingerprint,
        generations: RouteGenerations,
        locality: Locality,
        evidence: EvidenceClass,
    ) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            ResourceRef::parse(subject_ref).unwrap(),
            ResourceUid::parse(uid).unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            evidence,
            SessionPurpose::parse("zone-bus").unwrap(),
            ServiceName::parse(service).unwrap(),
            SessionBinding::new(
                schema,
                TransportBinding::new(locality, digest('2')),
                generations.session(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
        .with_provider_ref(ResourceRef::parse("Provider/system-core").unwrap())
        .with_provider_generation(generations.provider().unwrap())
        .with_controller_generation(generations.controller().unwrap())
    }

    fn fingerprint(value: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", value.to_string().repeat(64))).unwrap()
    }

    fn digest(value: char) -> BindingDigest {
        BindingDigest::parse(format!("sha256:{}", value.to_string().repeat(64))).unwrap()
    }

    fn bound_subject(context: &AuthenticatedSubjectContext) -> BoundSubject {
        BoundSubject {
            subject_ref: context.subject_ref().clone(),
            subject_uid: context.subject_uid().clone(),
        }
    }

    fn policy(
        revision: u64,
        subjects: &[BoundSubject],
        session_verbs: &[SessionVerb],
        resource_verbs: &[ResourceVerb],
    ) -> PolicySet {
        let catalog = ApiCatalog::standard();
        let resource_types = (!resource_verbs.is_empty())
            .then(|| ResourceTypeName::parse("Host").unwrap())
            .into_iter();
        let rule = PolicyRule::new(
            &catalog,
            resource_types,
            resource_verbs.iter().copied(),
            session_verbs.iter().copied(),
            [],
            [],
            [ZoneId::parse("dev").unwrap()],
            [],
        )
        .unwrap();
        let role =
            CompiledRole::new(ResourceRef::parse("Role/bus-test").unwrap(), vec![rule]).unwrap();
        let relay_authority = if session_verbs.contains(&SessionVerb::Relay) {
            RelayGrantAuthority::CoreGenerated
        } else {
            RelayGrantAuthority::None
        };
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            subjects.iter().cloned(),
            BindingScope::default(),
            relay_authority,
        )
        .unwrap();
        PolicySet::new(&catalog, revision, vec![role], vec![binding]).unwrap()
    }

    fn state(revision: u64) -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: revision,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                controller_generation: Some(ControllerGeneration::new(3).unwrap()),
            },
            zone_policy_revision: ZoneRevision::new(revision),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick: revision,
        }
    }

    fn operation(id: &str) -> OperationSpec {
        OperationSpec::new(OperationId::parse(id).unwrap(), 100).unwrap()
    }

    fn resource_harness(
        member: RouteMember,
        session_verbs: Vec<SessionVerb>,
        resource_verbs: Vec<ResourceVerb>,
        caller_ref: &str,
        locality: Locality,
        evidence: EvidenceClass,
    ) -> Harness {
        harness(HarnessSpec {
            service: "d2b.resource.v3",
            member,
            caller_ref,
            locality,
            evidence,
            session_verbs,
            resource_verbs,
            endpoint: RecordingEndpoint::new(),
        })
    }

    #[test]
    fn route_members_reject_wildcards_and_topic_shapes() {
        for invalid in [
            "",
            "*",
            "ResourceService/*",
            "ResourceService/Get?",
            "/ResourceService/Get",
            "ResourceService/Get/",
        ] {
            assert_eq!(
                RouteMember::method(invalid),
                Err(RegistryError::InvalidMember)
            );
            assert_eq!(
                RouteMember::stream(invalid),
                Err(RegistryError::InvalidMember)
            );
        }
    }

    #[test]
    fn consumed_session_identity_cannot_be_registered_twice() {
        let mut harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let duplicate = context(
            "User/alice",
            CALLER_UID,
            "d2b.resource.v3",
            harness.route.schema().clone(),
            harness.route.generations(),
            Locality::Local,
            EvidenceClass::UnixPeer,
        );

        assert!(matches!(
            harness.registrar.register(SessionRegistration::new(
                duplicate,
                Vec::new(),
                harness.endpoint.clone(),
            )),
            Err(BusError::Registry(RegistryError::DuplicateSessionIdentity))
        ));
    }

    #[test]
    fn exact_route_cannot_be_claimed_by_a_second_identity() {
        let mut harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let second = context(
            "User/bob",
            "33333333-3333-4333-8333-333333333333",
            "d2b.resource.v3",
            harness.route.schema().clone(),
            harness.route.generations(),
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        harness
            .bus
            .replace_policy(
                policy(
                    2,
                    &[
                        harness.subjects[0].clone(),
                        harness.subjects[1].clone(),
                        bound_subject(&second),
                    ],
                    &[SessionVerb::Connect, SessionVerb::Invoke],
                    &[ResourceVerb::Get],
                ),
                state(2),
            )
            .unwrap();

        assert!(matches!(
            harness.registrar.register(SessionRegistration::new(
                second,
                vec![harness.route.clone()],
                harness.endpoint.clone(),
            )),
            Err(BusError::Registry(RegistryError::DuplicateRoute))
        ));
    }

    #[tokio::test]
    async fn exact_route_is_required_and_no_direct_resource_fallback_exists() {
        let mut harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let response = harness
            .caller
            .invoke_resource(
                harness.route.clone(),
                operation("exact"),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                b"exact-payload".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(response.as_bytes(), b"response");
        assert_eq!(harness.endpoint.call_count(), 1);

        let resource_route = RouteKey::new(
            harness.route.zone().clone(),
            harness.route.service().clone(),
            harness.route.member().clone(),
            RouteTarget::resource(ResourceRef::parse("Host/system").unwrap()).unwrap(),
            harness.route.schema().clone(),
            harness.route.generations(),
        );
        let resource_endpoint = context(
            "User/resource-endpoint",
            "44444444-4444-4444-8444-444444444444",
            "d2b.resource.v3",
            harness.route.schema().clone(),
            harness.route.generations(),
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        harness
            .bus
            .replace_policy(
                policy(
                    2,
                    &[
                        harness.subjects[0].clone(),
                        harness.subjects[1].clone(),
                        bound_subject(&resource_endpoint),
                    ],
                    &[SessionVerb::Connect, SessionVerb::Invoke],
                    &[ResourceVerb::Get],
                ),
                state(2),
            )
            .unwrap();
        let resource_ingress = harness
            .registrar
            .register(SessionRegistration::new(
                resource_endpoint,
                vec![resource_route.clone()],
                harness.endpoint.clone(),
            ))
            .unwrap();
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    resource_route,
                    operation("wrong-resource"),
                    ResourceCall::Get(ResourceRef::parse("Host/other").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::InvalidResourceCall)
        );
        assert_eq!(harness.endpoint.call_count(), 1);
        harness.registrar.revoke(resource_ingress).unwrap();

        let unregistered = RouteKey::new(
            harness.route.zone().clone(),
            harness.route.service().clone(),
            harness.route.member().clone(),
            RouteTarget::resource(ResourceRef::parse("Host/system").unwrap()).unwrap(),
            harness.route.schema().clone(),
            harness.route.generations(),
        );
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    unregistered,
                    operation("no-fallback"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Registry(RegistryError::RouteNotFound))
        );
        assert_eq!(harness.endpoint.call_count(), 1);
    }

    #[tokio::test]
    async fn zone_mismatch_is_rejected_before_delivery() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let wrong_zone = RouteKey::new(
            ZoneId::parse("personal").unwrap(),
            harness.route.service().clone(),
            harness.route.member().clone(),
            harness.route.target().clone(),
            harness.route.schema().clone(),
            harness.route.generations(),
        );
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    wrong_zone,
                    operation("wrong-zone"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Authorization(AuthorizationError::ZoneMismatch))
        );
        assert_eq!(harness.endpoint.call_count(), 0);
    }

    #[tokio::test]
    async fn diagnostics_require_the_exact_service_method_and_grant_no_invoke() {
        let exact = harness(HarnessSpec {
            service: "d2b.audit.v3",
            member: RouteMember::method("AuditService/Export").unwrap(),
            caller_ref: "User/alice",
            locality: Locality::Local,
            evidence: EvidenceClass::UnixPeer,
            session_verbs: vec![SessionVerb::Connect, SessionVerb::AuditExport],
            resource_verbs: Vec::new(),
            endpoint: RecordingEndpoint::new(),
        });
        assert!(
            exact
                .caller
                .invoke(exact.route.clone(), operation("audit-export"), Vec::new(),)
                .await
                .is_ok()
        );

        let near_miss = harness(HarnessSpec {
            service: "d2b.audit.v3",
            member: RouteMember::method("AuditService/Inspect").unwrap(),
            caller_ref: "User/alice",
            locality: Locality::Local,
            evidence: EvidenceClass::UnixPeer,
            session_verbs: vec![SessionVerb::Connect, SessionVerb::AuditExport],
            resource_verbs: Vec::new(),
            endpoint: RecordingEndpoint::new(),
        });
        assert_eq!(
            near_miss
                .caller
                .invoke(
                    near_miss.route.clone(),
                    operation("audit-near-miss"),
                    Vec::new(),
                )
                .await,
            Err(BusError::RouteShape)
        );
        assert_eq!(near_miss.endpoint.call_count(), 0);

        let support = harness(HarnessSpec {
            service: "d2b.support.v3",
            member: RouteMember::method("SupportService/GenerateBundle").unwrap(),
            caller_ref: "User/alice",
            locality: Locality::Local,
            evidence: EvidenceClass::UnixPeer,
            session_verbs: vec![SessionVerb::Connect, SessionVerb::SupportBundle],
            resource_verbs: Vec::new(),
            endpoint: RecordingEndpoint::new(),
        });
        assert!(
            support
                .caller
                .invoke(
                    support.route.clone(),
                    operation("support-bundle"),
                    Vec::new(),
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn relay_and_target_verb_are_independently_required() {
        let no_relay = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "ZoneLink/parent",
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
        );
        assert_eq!(
            no_relay
                .caller
                .invoke_resource(
                    no_relay.route.clone(),
                    operation("relay-missing"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Authorization(
                AuthorizationError::RelayGrantMissing
            ))
        );

        let no_target = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![
                SessionVerb::Connect,
                SessionVerb::Invoke,
                SessionVerb::Relay,
            ],
            Vec::new(),
            "ZoneLink/parent",
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
        );
        assert_eq!(
            no_target
                .caller
                .invoke_resource(
                    no_target.route.clone(),
                    operation("target-missing"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Authorization(AuthorizationError::Native(
                d2b_resource_api::authz::AuthorizationDenial::RelayTargetGrantMissing
            )))
        );
    }

    #[test]
    fn adjacent_relay_identity_never_inherits_a_local_subject_grant() {
        let zone = ZoneId::parse("dev").unwrap();
        let schema = fingerprint('1');
        let generations = RouteGenerations::new(
            Some(ResourceGeneration::new(2).unwrap()),
            Some(ControllerGeneration::new(3).unwrap()),
            ReconnectGeneration::new(1).unwrap(),
        );
        let local = context(
            "User/alice",
            CALLER_UID,
            "d2b.resource.v3",
            schema.clone(),
            generations,
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let relay = context(
            "ZoneLink/parent",
            ENDPOINT_UID,
            "d2b.resource.v3",
            schema,
            generations,
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
        );
        let native = NativeAuthorizer::new(
            ApiCatalog::standard(),
            Some(policy(
                1,
                &[bound_subject(&local)],
                &[SessionVerb::Connect, SessionVerb::Relay],
                &[],
            )),
        )
        .unwrap();
        let (_bus, mut registrar) = ZoneBus::new(
            zone,
            BusAuthorizer::new(native, state(1)).unwrap(),
            BusConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            registrar.register(SessionRegistration::new(
                relay,
                Vec::new(),
                RecordingEndpoint::new(),
            )),
            Err(BusError::Authorization(
                AuthorizationError::SessionVerbMissing(SessionVerb::Connect)
            ))
        ));
    }

    #[test]
    fn provider_routes_reject_peer_self_assertion() {
        let zone = ZoneId::parse("dev").unwrap();
        let schema = fingerprint('1');
        let generations = RouteGenerations::new(
            Some(ResourceGeneration::new(2).unwrap()),
            Some(ControllerGeneration::new(3).unwrap()),
            ReconnectGeneration::new(1).unwrap(),
        );
        let forged = context(
            "Provider/attacker",
            CALLER_UID,
            "d2b.echo.v3",
            schema.clone(),
            generations,
            Locality::Local,
            EvidenceClass::EnrolledKk,
        );
        let subjects = vec![bound_subject(&forged)];
        let native = NativeAuthorizer::new(
            ApiCatalog::standard(),
            Some(policy(1, &subjects, &[SessionVerb::Connect], &[])),
        )
        .unwrap();
        let (bus, mut registrar) = ZoneBus::new(
            zone.clone(),
            BusAuthorizer::new(native, state(1)).unwrap(),
            BusConfig::default(),
        )
        .unwrap();
        let route = RouteKey::new(
            zone,
            ServiceName::parse("d2b.echo.v3").unwrap(),
            RouteMember::method("EchoService/Call").unwrap(),
            RouteTarget::provider(ResourceRef::parse("Provider/system-core").unwrap()).unwrap(),
            schema,
            generations,
        );
        let result = registrar.register(SessionRegistration::new(
            forged,
            vec![route],
            RecordingEndpoint::new(),
        ));
        assert!(matches!(
            result,
            Err(BusError::Registry(RegistryError::ProviderAssertion))
        ));
        drop(bus);
    }

    #[tokio::test]
    async fn list_and_watch_selectors_survive_an_adjacent_hop_exactly() {
        let mut list = resource_harness(
            RouteMember::method("ResourceService/List").unwrap(),
            vec![
                SessionVerb::Connect,
                SessionVerb::Invoke,
                SessionVerb::Relay,
            ],
            vec![ResourceVerb::List],
            "ZoneLink/parent",
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
        );
        let nameless = ResourceQuery::new(
            vec![ResourceTypeName::parse("Host").unwrap()],
            Vec::new(),
            vec![
                ResourceFilter::new("metadata.managedBy", vec!["configuration".to_owned()])
                    .unwrap(),
            ],
        )
        .unwrap();
        list.caller
            .invoke_resource(
                list.route.clone(),
                operation("list"),
                ResourceCall::List(nameless.clone()),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            list.endpoint.calls.lock().unwrap()[0].1,
            Some(ResourceCall::List(nameless))
        );

        let watch_route = RouteKey::new(
            list.route.zone().clone(),
            list.route.service().clone(),
            RouteMember::method("ResourceService/Watch").unwrap(),
            list.route.target().clone(),
            list.route.schema().clone(),
            list.route.generations(),
        );
        let endpoint_context = context(
            "Provider/system-core",
            ENDPOINT_UID,
            "d2b.resource.v3",
            list.route.schema().clone(),
            list.route.generations(),
            Locality::Local,
            EvidenceClass::EnrolledKk,
        );
        list.registrar.revoke(list.endpoint_ingress).unwrap();
        list.endpoint_ingress = list
            .registrar
            .register(SessionRegistration::new(
                endpoint_context,
                vec![watch_route.clone()],
                list.endpoint.clone(),
            ))
            .unwrap();
        list.bus
            .replace_policy(
                policy(
                    2,
                    &list.subjects,
                    &[
                        SessionVerb::Connect,
                        SessionVerb::Invoke,
                        SessionVerb::Relay,
                    ],
                    &[ResourceVerb::Watch],
                ),
                state(2),
            )
            .unwrap();
        let named = ResourceQuery::new(
            vec![ResourceTypeName::parse("Host").unwrap()],
            vec![ResourceName::parse("system").unwrap()],
            vec![ResourceFilter::new("status.phase", vec!["Ready".to_owned()]).unwrap()],
        )
        .unwrap();
        list.caller
            .invoke_resource(
                watch_route,
                operation("watch"),
                ResourceCall::Watch(named.clone()),
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            list.endpoint.calls.lock().unwrap()[1].1,
            Some(ResourceCall::Watch(named))
        );
    }

    #[tokio::test]
    async fn policy_replacement_revokes_a_previously_authorized_route() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        harness
            .caller
            .invoke_resource(
                harness.route.clone(),
                operation("before-revoke"),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                Vec::new(),
            )
            .await
            .unwrap();
        harness
            .bus
            .replace_policy(
                policy(2, &harness.subjects, &[SessionVerb::Connect], &[]),
                state(2),
            )
            .unwrap();
        assert!(matches!(
            harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation("after-revoke"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Authorization(
                AuthorizationError::SessionVerbMissing(SessionVerb::Invoke)
            ))
        ));
        assert_eq!(harness.endpoint.call_count(), 1);
    }

    #[tokio::test]
    async fn reconnect_replaces_routes_and_refuses_the_old_generation() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let Harness {
            bus,
            mut registrar,
            caller,
            endpoint_ingress,
            endpoint,
            route,
            subjects: _,
            clock: _,
        } = harness;
        let generations = RouteGenerations::new(
            route.generations().provider(),
            route.generations().controller(),
            ReconnectGeneration::new(2).unwrap(),
        );
        let new_route = RouteKey::new(
            route.zone().clone(),
            route.service().clone(),
            route.member().clone(),
            route.target().clone(),
            route.schema().clone(),
            generations,
        );
        let new_endpoint = context(
            "Provider/system-core",
            ENDPOINT_UID,
            "d2b.resource.v3",
            route.schema().clone(),
            generations,
            Locality::Local,
            EvidenceClass::EnrolledKk,
        );
        let endpoint_ingress = registrar
            .reconnect(
                endpoint_ingress,
                SessionRegistration::new(new_endpoint, vec![new_route.clone()], endpoint.clone()),
            )
            .unwrap();
        let new_caller = context(
            "User/alice",
            CALLER_UID,
            "d2b.resource.v3",
            route.schema().clone(),
            generations,
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let caller = registrar
            .reconnect(
                caller,
                SessionRegistration::new(new_caller, Vec::new(), endpoint.clone()),
            )
            .unwrap();
        assert_eq!(
            caller
                .invoke_resource(
                    route,
                    operation("old-generation"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Authorization(
                AuthorizationError::SessionBindingMismatch
            ))
        );
        assert!(
            caller
                .invoke_resource(
                    new_route,
                    operation("new-generation"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await
                .is_ok()
        );
        drop(endpoint_ingress);
        drop(bus);
    }

    #[tokio::test]
    async fn revoke_between_resolution_and_begin_rejects_the_route_lease() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let hook = Arc::new(InvocationHook {
            reached: Notify::new(),
            release: Notify::new(),
        });
        harness
            .caller
            .core
            .invocation_hooks
            .lock()
            .unwrap()
            .after_resolve = Some(Arc::clone(&hook));
        let mut registrar = harness.registrar;
        let invoke = harness.caller.invoke_resource(
            harness.route,
            operation("revoke-before-begin"),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        );
        let revoke = async {
            hook.reached.notified().await;
            registrar.revoke(harness.endpoint_ingress).unwrap();
            hook.release.notify_one();
        };
        let (result, ()) = tokio::join!(invoke, revoke);
        assert_eq!(
            result,
            Err(BusError::Operation(OperationError::RouteRevoked))
        );
        assert_eq!(harness.endpoint.call_count(), 0);
    }

    #[tokio::test]
    async fn revoke_after_begin_cancels_before_endpoint_invocation() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let hook = Arc::new(InvocationHook {
            reached: Notify::new(),
            release: Notify::new(),
        });
        harness
            .caller
            .core
            .invocation_hooks
            .lock()
            .unwrap()
            .before_invoke = Some(Arc::clone(&hook));
        let mut registrar = harness.registrar;
        let invoke = harness.caller.invoke_resource(
            harness.route,
            operation("revoke-before-invoke"),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        );
        let revoke = async {
            hook.reached.notified().await;
            registrar.revoke(harness.endpoint_ingress).unwrap();
            hook.release.notify_one();
        };
        let (result, ()) = tokio::join!(invoke, revoke);
        assert_eq!(result, Err(BusError::Cancelled));
        assert_eq!(harness.endpoint.call_count(), 0);
    }

    #[tokio::test]
    async fn cancellation_uses_the_pinned_reverse_route() {
        let endpoint = RecordingEndpoint::blocking();
        let harness = harness(HarnessSpec {
            service: "d2b.resource.v3",
            member: RouteMember::method("ResourceService/Get").unwrap(),
            caller_ref: "User/alice",
            locality: Locality::Local,
            evidence: EvidenceClass::UnixPeer,
            session_verbs: vec![
                SessionVerb::Connect,
                SessionVerb::Invoke,
                SessionVerb::Cancel,
            ],
            resource_verbs: vec![ResourceVerb::Get],
            endpoint: endpoint.clone(),
        });
        let id = OperationId::parse("cancel-operation").unwrap();
        let invoke = harness.caller.invoke_resource(
            harness.route.clone(),
            OperationSpec::new(id.clone(), 100).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        );
        let cancel = async {
            endpoint.started.notified().await;
            harness.caller.cancel(&id).await
        };
        let (invoke_result, cancel_result) = tokio::join!(invoke, cancel);
        assert_eq!(invoke_result, Err(BusError::Cancelled));
        assert_eq!(cancel_result, Ok(()));
        assert_eq!(endpoint.cancel_count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn concurrent_invocations_saturate_the_operation_bound() {
        let endpoint = RecordingEndpoint::blocking();
        let harness = harness_with_config(
            HarnessSpec {
                service: "d2b.resource.v3",
                member: RouteMember::method("ResourceService/Get").unwrap(),
                caller_ref: "User/alice",
                locality: Locality::Local,
                evidence: EvidenceClass::UnixPeer,
                session_verbs: vec![SessionVerb::Connect, SessionVerb::Invoke],
                resource_verbs: vec![ResourceVerb::Get],
                endpoint: endpoint.clone(),
            },
            BusConfig {
                max_operations: 1,
                max_operations_per_session: 1,
                ..BusConfig::default()
            },
        );
        let first = harness.caller.invoke_resource(
            harness.route.clone(),
            operation("first-in-flight"),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        );
        let second = async {
            endpoint.started.notified().await;
            let result = harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation("second-in-flight"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await;
            endpoint.release.notify_one();
            result
        };
        let (first_result, second_result) = tokio::join!(first, second);
        assert!(first_result.is_ok());
        assert_eq!(
            second_result,
            Err(BusError::Operation(OperationError::CapacityExceeded))
        );
        assert_eq!(endpoint.call_count(), 1);
    }

    #[tokio::test]
    async fn deadline_expires_before_endpoint_delivery() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        harness.clock.advance_to(100);
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation("expired-before-delivery"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::Operation(OperationError::DeadlineExceeded))
        );
        assert_eq!(harness.endpoint.call_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn active_deadline_cancels_a_hung_endpoint_and_reclaims_the_slot() {
        let endpoint = RecordingEndpoint::blocking();
        let harness = harness_with_config(
            HarnessSpec {
                service: "d2b.resource.v3",
                member: RouteMember::method("ResourceService/Get").unwrap(),
                caller_ref: "User/alice",
                locality: Locality::Local,
                evidence: EvidenceClass::UnixPeer,
                session_verbs: vec![SessionVerb::Connect, SessionVerb::Invoke],
                resource_verbs: vec![ResourceVerb::Get],
                endpoint: endpoint.clone(),
            },
            BusConfig {
                max_operations: 1,
                max_operations_per_session: 1,
                ..BusConfig::default()
            },
        );
        let result = harness
            .caller
            .invoke_resource(
                harness.route.clone(),
                OperationSpec::new(OperationId::parse("hung").unwrap(), 2).unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                Vec::new(),
            )
            .await;
        assert_eq!(
            result,
            Err(BusError::Operation(OperationError::DeadlineExceeded))
        );

        let second = harness.caller.invoke_resource(
            harness.route.clone(),
            OperationSpec::new(OperationId::parse("hung").unwrap(), 100).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        );
        let release = async {
            while endpoint.call_count() < 2 {
                tokio::task::yield_now().await;
            }
            endpoint.release.notify_one();
        };
        let (second, ()) = tokio::join!(second, release);
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn dropping_invoke_future_reclaims_operation_id_and_capacity() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let hook = Arc::new(InvocationHook {
            reached: Notify::new(),
            release: Notify::new(),
        });
        harness
            .caller
            .core
            .invocation_hooks
            .lock()
            .unwrap()
            .before_invoke = Some(Arc::clone(&hook));
        let operation = operation("dropped-future");
        let mut invoke = Box::pin(harness.caller.invoke_resource(
            harness.route.clone(),
            operation.clone(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            Vec::new(),
        ));
        tokio::select! {
            result = &mut invoke => panic!("invoke unexpectedly completed: {result:?}"),
            () = hook.reached.notified() => {}
        }
        drop(invoke);
        harness
            .caller
            .core
            .invocation_hooks
            .lock()
            .unwrap()
            .before_invoke = None;
        assert!(
            harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation,
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn oversized_endpoint_response_is_rejected_after_lease_cleanup() {
        let endpoint = RecordingEndpoint::oversized();
        let harness = harness(HarnessSpec {
            service: "d2b.resource.v3",
            member: RouteMember::method("ResourceService/Get").unwrap(),
            caller_ref: "User/alice",
            locality: Locality::Local,
            evidence: EvidenceClass::UnixPeer,
            session_verbs: vec![SessionVerb::Connect, SessionVerb::Invoke],
            resource_verbs: vec![ResourceVerb::Get],
            endpoint,
        });
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation("oversized-response"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::RouteShape)
        );
        assert_eq!(
            harness
                .caller
                .invoke_resource(
                    harness.route.clone(),
                    operation("oversized-response"),
                    ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                    Vec::new(),
                )
                .await,
            Err(BusError::RouteShape)
        );
        assert_eq!(harness.endpoint.call_count(), 2);
    }

    #[tokio::test]
    async fn bus_observer_receives_only_closed_failure_labels() {
        let observer = Arc::new(CaptureObserver::default());
        let harness = harness_with_config_and_observer(
            HarnessSpec {
                service: "d2b.resource.v3",
                member: RouteMember::method("ResourceService/Get").unwrap(),
                caller_ref: "User/alice",
                locality: Locality::Local,
                evidence: EvidenceClass::UnixPeer,
                session_verbs: vec![SessionVerb::Connect, SessionVerb::Invoke],
                resource_verbs: vec![ResourceVerb::Get],
                endpoint: RecordingEndpoint::new(),
            },
            BusConfig::default(),
            observer.clone(),
        );
        assert_eq!(
            harness
                .caller
                .invoke(
                    harness.route.clone(),
                    operation("observed-invalid-call"),
                    Vec::new(),
                )
                .await,
            Err(BusError::InvalidResourceCall)
        );
        assert_eq!(
            observer.0.lock().unwrap().as_slice(),
            &[(BusEvent::Invoke, BusFailureReason::Route)]
        );
    }

    #[test]
    fn endpoint_session_failures_preserve_actionable_details_and_closed_labels() {
        use crate::registry::EndpointFailureClass;
        use d2b_contracts::v3::component_session::{Remediation, SessionErrorCode};

        let cases = [
            (
                SessionErrorCode::AuthenticationFailed,
                EndpointFailureClass::Authentication,
                BusFailureReason::Authentication,
                Remediation::ReEnrollPeer,
            ),
            (
                SessionErrorCode::PolicyDenied,
                EndpointFailureClass::Authorization,
                BusFailureReason::Authorization,
                Remediation::RepairConfiguration,
            ),
            (
                SessionErrorCode::GenerationMismatch,
                EndpointFailureClass::Generation,
                BusFailureReason::Generation,
                Remediation::ReplaceGeneration,
            ),
            (
                SessionErrorCode::QueueBackpressure,
                EndpointFailureClass::Backpressure,
                BusFailureReason::Backpressure,
                Remediation::ReduceLoad,
            ),
            (
                SessionErrorCode::DeadlineExpired,
                EndpointFailureClass::Deadline,
                BusFailureReason::Deadline,
                Remediation::RetryBounded,
            ),
            (
                SessionErrorCode::SessionDisconnected,
                EndpointFailureClass::Transport,
                BusFailureReason::Transport,
                Remediation::RestartAgent,
            ),
            (
                SessionErrorCode::RecordMalformed,
                EndpointFailureClass::Protocol,
                BusFailureReason::Protocol,
                Remediation::RepairConfiguration,
            ),
            (
                SessionErrorCode::InternalInvariant,
                EndpointFailureClass::Internal,
                BusFailureReason::Endpoint,
                Remediation::RestartAgent,
            ),
        ];
        for (code, class, expected_label, remediation) in cases {
            let endpoint_error = EndpointError::from(d2b_session::SessionError::new(code));
            let EndpointError::Session(failure) = endpoint_error else {
                panic!("session errors preserve their endpoint failure");
            };
            assert_eq!(failure.class(), class);
            assert_eq!(failure.code(), code);
            assert_eq!(failure.remediation(), remediation);
            assert_eq!(
                BusFailureReason::from_error(&BusError::Endpoint(EndpointError::Session(failure))),
                expected_label
            );
            let display = EndpointError::Session(failure).to_string();
            assert!(display.contains(code.as_str()));
            assert!(display.contains(remediation.as_str()));
        }
        assert_eq!(
            BusFailureReason::from_error(&BusError::Operation(OperationError::RouteRevoked)),
            BusFailureReason::RouteRevoked
        );
    }

    #[tokio::test]
    async fn routed_named_stream_enforces_credit_and_preserves_watch_query() {
        let harness = resource_harness(
            RouteMember::stream("ResourceService/Watch").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::OpenStream],
            vec![ResourceVerb::Watch],
            "User/alice",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let query = ResourceQuery::new(
            vec![ResourceTypeName::parse("Host").unwrap()],
            Vec::new(),
            vec![ResourceFilter::new("status.phase", vec!["Ready".to_owned()]).unwrap()],
        )
        .unwrap();
        let stream = harness
            .caller
            .open_resource_stream(
                harness.route.clone(),
                operation("watch-stream"),
                ResourceCall::Watch(query.clone()),
                StreamName::parse("watch:hosts").unwrap(),
                4,
            )
            .await
            .unwrap();
        assert_eq!(
            harness.endpoint.calls.lock().unwrap()[0].1,
            Some(ResourceCall::Watch(query))
        );
        stream.send(vec![1, 2, 3, 4]).await.unwrap();
        assert_eq!(
            stream.send(vec![5]).await,
            Err(BusError::Stream(StreamError::CreditExceeded))
        );
        let incoming = harness.endpoint.incoming.lock().unwrap().pop().unwrap();
        let frame = incoming.receive_next().await.unwrap();
        assert_eq!(frame.stream(), stream.name());
        assert_eq!(frame.payload(), &[1, 2, 3, 4]);
        incoming.grant(stream.name(), 1).await.unwrap();
        stream.send(vec![5]).await.unwrap();
        stream.close().await.unwrap();
    }

    #[test]
    fn debug_surfaces_redact_routes_payloads_and_identity() {
        let harness = resource_harness(
            RouteMember::method("ResourceService/Get").unwrap(),
            vec![SessionVerb::Connect, SessionVerb::Invoke],
            vec![ResourceVerb::Get],
            "User/sentinel-subject",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let source = harness
            .caller
            .core
            .lock_registry()
            .source(harness.caller.session)
            .unwrap();
        let subject = source.context.as_ref().unwrap().subject_ref();
        assert_eq!(subject.resource_type().as_str(), "User");
        assert_eq!(subject.name().as_str(), "sentinel-subject");
        assert_eq!(harness.route.service().as_str(), "d2b.resource.v3");
        assert_eq!(
            harness.route.target().resource_ref().name().as_str(),
            "system-core"
        );
        let rendered = format!(
            "{:?} {:?} {:?} {:?}",
            harness.bus, harness.registrar, harness.caller, harness.route
        );
        assert!(!rendered.contains("sentinel-subject"));
        assert!(!rendered.contains("system-core"));
        assert!(!rendered.contains("d2b.resource.v3"));
    }

    #[test]
    fn manual_clock_only_moves_forward() {
        let clock = ManualClock::new(7);
        clock.advance_to(3);
        assert_eq!(clock.now_tick(), 7);
        clock.advance_to(9);
        assert_eq!(clock.now_tick(), 9);
    }
}
