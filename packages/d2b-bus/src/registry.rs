//! Exact route registry owned by one Zone.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ControllerGeneration, EvidenceClass, Locality,
    ReconnectGeneration, ResourceGeneration, ResourceRef, SchemaFingerprint, ServiceName, ZoneId,
};

use crate::{
    operations::SessionId,
    router::{DeliveredInvocation, DeliveredStream},
};

/// A method or named-stream member in an exact route.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteMember {
    Method(String),
    Stream(String),
}

impl RouteMember {
    /// Construct an exact method member.
    pub fn method(value: impl Into<String>) -> Result<Self, RegistryError> {
        validate_member(value.into()).map(Self::Method)
    }

    /// Construct an exact named-stream member.
    pub fn stream(value: impl Into<String>) -> Result<Self, RegistryError> {
        validate_member(value.into()).map(Self::Stream)
    }

    /// Borrow the exact generated member name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Method(value) | Self::Stream(value) => value,
        }
    }

    /// Whether this member names a method.
    pub const fn is_method(&self) -> bool {
        matches!(self, Self::Method(_))
    }

    /// Whether this member names a stream.
    pub const fn is_stream(&self) -> bool {
        matches!(self, Self::Stream(_))
    }
}

fn validate_member(value: String) -> Result<String, RegistryError> {
    if value.is_empty()
        || value.len() > 128
        || value.contains('*')
        || value.contains('?')
        || value.starts_with('/')
        || value.ends_with('/')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
    {
        return Err(RegistryError::InvalidMember);
    }
    Ok(value)
}

impl core::fmt::Debug for RouteMember {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Method(_) => "RouteMember::Method(<redacted>)",
            Self::Stream(_) => "RouteMember::Stream(<redacted>)",
        })
    }
}

/// Exact resource or Provider addressed by a route.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteTarget {
    Resource(ResourceRef),
    Provider(ResourceRef),
}

impl RouteTarget {
    /// Construct an exact non-Provider resource target.
    pub fn resource(value: ResourceRef) -> Result<Self, RegistryError> {
        if value.resource_type().as_str() == "Provider" {
            return Err(RegistryError::TargetKindMismatch);
        }
        Ok(Self::Resource(value))
    }

    /// Construct an exact Provider target.
    pub fn provider(value: ResourceRef) -> Result<Self, RegistryError> {
        if value.resource_type().as_str() != "Provider" {
            return Err(RegistryError::TargetKindMismatch);
        }
        Ok(Self::Provider(value))
    }

    /// Borrow the exact addressed reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        match self {
            Self::Resource(value) | Self::Provider(value) => value,
        }
    }
}

impl core::fmt::Debug for RouteTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Resource(_) => "RouteTarget::Resource(<redacted>)",
            Self::Provider(_) => "RouteTarget::Provider(<redacted>)",
        })
    }
}

/// Exact Provider, controller, and reconnect generations in a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteGenerations {
    provider: Option<ResourceGeneration>,
    controller: Option<ControllerGeneration>,
    session: ReconnectGeneration,
}

impl RouteGenerations {
    /// Construct a generation tuple.
    pub const fn new(
        provider: Option<ResourceGeneration>,
        controller: Option<ControllerGeneration>,
        session: ReconnectGeneration,
    ) -> Self {
        Self {
            provider,
            controller,
            session,
        }
    }

    /// Return the Provider generation, when the service is Provider-bound.
    pub const fn provider(self) -> Option<ResourceGeneration> {
        self.provider
    }

    /// Return the controller generation, when the route is controller-bound.
    pub const fn controller(self) -> Option<ControllerGeneration> {
        self.controller
    }

    /// Return the authenticated reconnect generation.
    pub const fn session(self) -> ReconnectGeneration {
        self.session
    }
}

/// Complete exact route key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteKey {
    zone: ZoneId,
    service: ServiceName,
    member: RouteMember,
    target: RouteTarget,
    schema: SchemaFingerprint,
    generations: RouteGenerations,
}

impl RouteKey {
    /// Construct a route from validated exact components.
    pub const fn new(
        zone: ZoneId,
        service: ServiceName,
        member: RouteMember,
        target: RouteTarget,
        schema: SchemaFingerprint,
        generations: RouteGenerations,
    ) -> Self {
        Self {
            zone,
            service,
            member,
            target,
            schema,
            generations,
        }
    }

    /// Borrow the destination Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the generated service package.
    pub const fn service(&self) -> &ServiceName {
        &self.service
    }

    /// Borrow the exact method or stream member.
    pub const fn member(&self) -> &RouteMember {
        &self.member
    }

    /// Borrow the exact addressed target.
    pub const fn target(&self) -> &RouteTarget {
        &self.target
    }

    /// Borrow the schema fingerprint.
    pub const fn schema(&self) -> &SchemaFingerprint {
        &self.schema
    }

    /// Return the exact generation tuple.
    pub const fn generations(&self) -> RouteGenerations {
        self.generations
    }
}

impl core::fmt::Debug for RouteKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RouteKey")
            .field("member", &self.member)
            .field("target", &self.target)
            .field("generations", &self.generations)
            .field("identity", &"<redacted>")
            .finish()
    }
}

/// Closed endpoint dispatch failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    Unavailable,
    Rejected,
    Internal,
}

impl core::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "endpoint is unavailable",
            Self::Rejected => "endpoint rejected the request",
            Self::Internal => "endpoint failed",
        })
    }
}

impl std::error::Error for EndpointError {}

/// Opaque endpoint response bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct BusResponse(Vec<u8>);

impl BusResponse {
    /// Construct a response from bounded service bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Borrow the response bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Debug for BusResponse {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("BusResponse")
            .field(&format_args!("<{} bytes>", self.0.len()))
            .finish()
    }
}

/// An exact generated service endpoint.
#[async_trait]
pub trait BusEndpoint: Send + Sync + 'static {
    /// Deliver one already-authorized method invocation.
    async fn invoke(&self, request: DeliveredInvocation) -> Result<BusResponse, EndpointError>;

    /// Open one already-authorized named stream.
    async fn open_stream(&self, request: DeliveredStream) -> Result<(), EndpointError>;

    /// Notify the exact destination of an authorized cancellation.
    async fn cancel(&self, _operation: &crate::operations::OperationId) {}
}

/// Registration input consumed by the Zone's single registration authority.
pub struct SessionRegistration {
    context: AuthenticatedSubjectContext,
    routes: Vec<RouteKey>,
    endpoint: Arc<dyn BusEndpoint>,
}

impl SessionRegistration {
    /// Bind authenticated session claims, exact routes, and one endpoint.
    pub fn new(
        context: AuthenticatedSubjectContext,
        routes: Vec<RouteKey>,
        endpoint: Arc<dyn BusEndpoint>,
    ) -> Self {
        Self {
            context,
            routes,
            endpoint,
        }
    }

    pub(crate) const fn context(&self) -> &AuthenticatedSubjectContext {
        &self.context
    }
}

impl core::fmt::Debug for SessionRegistration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionRegistration")
            .field("route_count", &self.routes.len())
            .field("context", &"<redacted>")
            .finish()
    }
}

struct RegisteredSession {
    context: AuthenticatedSubjectContext,
    routes: BTreeSet<RouteKey>,
    endpoint: Arc<dyn BusEndpoint>,
}

pub(crate) struct ResolvedEndpoint {
    pub(crate) session: SessionId,
    pub(crate) endpoint: Arc<dyn BusEndpoint>,
}

pub(crate) struct Registry {
    zone: ZoneId,
    next_session: u64,
    sessions: BTreeMap<SessionId, RegisteredSession>,
    routes: BTreeMap<RouteKey, SessionId>,
}

impl Registry {
    pub(crate) fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            next_session: 1,
            sessions: BTreeMap::new(),
            routes: BTreeMap::new(),
        }
    }

    pub(crate) fn register(
        &mut self,
        registration: SessionRegistration,
    ) -> Result<SessionId, RegistryError> {
        self.validate_registration(&registration, None)?;
        let session = SessionId(self.next_session);
        self.next_session = self
            .next_session
            .checked_add(1)
            .ok_or(RegistryError::SessionIdExhausted)?;
        self.install(session, registration);
        Ok(session)
    }

    pub(crate) fn reconnect(
        &mut self,
        previous: SessionId,
        registration: SessionRegistration,
    ) -> Result<SessionId, RegistryError> {
        let prior = self
            .sessions
            .get(&previous)
            .ok_or(RegistryError::SessionNotFound)?;
        if prior.context.subject_ref() != registration.context.subject_ref()
            || prior.context.subject_uid() != registration.context.subject_uid()
            || prior.context.zone_ref() != registration.context.zone_ref()
            || registration.context.reconnect_generation().get()
                != prior
                    .context
                    .reconnect_generation()
                    .get()
                    .checked_add(1)
                    .ok_or(RegistryError::ReconnectGeneration)?
            || !route_sets_reconnect(&prior.routes, &registration.routes)
        {
            return Err(RegistryError::ReconnectGeneration);
        }
        self.validate_registration(&registration, Some(previous))?;

        let session = SessionId(self.next_session);
        self.next_session = self
            .next_session
            .checked_add(1)
            .ok_or(RegistryError::SessionIdExhausted)?;
        self.remove(previous);
        self.install(session, registration);
        Ok(session)
    }

    fn validate_registration(
        &self,
        registration: &SessionRegistration,
        replacing: Option<SessionId>,
    ) -> Result<(), RegistryError> {
        ensure_context_zone(&registration.context, &self.zone)?;
        ensure_transport_evidence(&registration.context)?;
        if self.sessions.iter().any(|(session, registered)| {
            Some(*session) != replacing
                && registered.context.subject_ref() == registration.context.subject_ref()
                && registered.context.subject_uid() == registration.context.subject_uid()
        }) {
            return Err(RegistryError::DuplicateSessionIdentity);
        }

        let distinct = registration.routes.iter().cloned().collect::<BTreeSet<_>>();
        if distinct.len() != registration.routes.len() {
            return Err(RegistryError::DuplicateRoute);
        }
        for route in &registration.routes {
            validate_route_binding(&self.zone, &registration.context, route)?;
            if self
                .routes
                .get(route)
                .is_some_and(|session| Some(*session) != replacing)
            {
                return Err(RegistryError::DuplicateRoute);
            }
        }
        Ok(())
    }

    fn install(&mut self, session: SessionId, registration: SessionRegistration) {
        let routes = registration.routes.into_iter().collect::<BTreeSet<_>>();
        for route in &routes {
            self.routes.insert(route.clone(), session);
        }
        self.sessions.insert(
            session,
            RegisteredSession {
                context: registration.context,
                routes,
                endpoint: registration.endpoint,
            },
        );
    }

    pub(crate) fn remove(&mut self, session: SessionId) -> bool {
        let Some(registered) = self.sessions.remove(&session) else {
            return false;
        };
        for route in registered.routes {
            self.routes.remove(&route);
        }
        true
    }

    pub(crate) fn with_context<T>(
        &self,
        session: SessionId,
        apply: impl FnOnce(&AuthenticatedSubjectContext) -> T,
    ) -> Result<T, RegistryError> {
        self.sessions
            .get(&session)
            .map(|registered| apply(&registered.context))
            .ok_or(RegistryError::SessionNotFound)
    }

    pub(crate) fn resolve(&self, route: &RouteKey) -> Result<ResolvedEndpoint, RegistryError> {
        if route.zone != self.zone {
            return Err(RegistryError::ZoneMismatch);
        }
        let session = *self.routes.get(route).ok_or(RegistryError::RouteNotFound)?;
        let endpoint = Arc::clone(
            &self
                .sessions
                .get(&session)
                .ok_or(RegistryError::InternalInvariant)?
                .endpoint,
        );
        Ok(ResolvedEndpoint { session, endpoint })
    }
}

fn ensure_context_zone(
    context: &AuthenticatedSubjectContext,
    zone: &ZoneId,
) -> Result<(), RegistryError> {
    if context.zone_ref().resource_type().as_str() != "Zone"
        || context.zone_ref().name().as_str() != zone.as_str()
    {
        return Err(RegistryError::ZoneMismatch);
    }
    Ok(())
}

fn ensure_transport_evidence(context: &AuthenticatedSubjectContext) -> Result<(), RegistryError> {
    match context.transport_binding().locality() {
        Locality::Local => Ok(()),
        Locality::AdjacentZone
            if context.evidence_class() == EvidenceClass::EnrolledKk
                && matches!(
                    context.subject_ref().resource_type().as_str(),
                    "Zone" | "ZoneLink"
                ) =>
        {
            Ok(())
        }
        Locality::AdjacentZone | Locality::Remote => Err(RegistryError::UnauthenticatedTransport),
    }
}

fn validate_route_binding(
    zone: &ZoneId,
    context: &AuthenticatedSubjectContext,
    route: &RouteKey,
) -> Result<(), RegistryError> {
    if route.zone() != zone {
        return Err(RegistryError::ZoneMismatch);
    }
    if context.service() != route.service()
        || context.schema_fingerprint() != route.schema()
        || context.reconnect_generation() != route.generations().session()
        || context.provider_generation() != route.generations().provider()
        || context.controller_generation() != route.generations().controller()
    {
        return Err(RegistryError::SessionBindingMismatch);
    }

    if context.transport_binding().locality() == Locality::Local {
        if context.subject_ref().resource_type().as_str() == "Provider"
            && context.provider_ref() != Some(context.subject_ref())
        {
            return Err(RegistryError::ProviderAssertion);
        }
        if let RouteTarget::Provider(provider) = route.target()
            && context.provider_ref() != Some(provider)
        {
            return Err(RegistryError::ProviderAssertion);
        }
    }
    Ok(())
}

fn route_sets_reconnect(old: &BTreeSet<RouteKey>, new: &[RouteKey]) -> bool {
    let new = new.iter().collect::<Vec<_>>();
    old.len() == new.len()
        && old.iter().all(|old_route| {
            new.iter()
                .any(|new_route| same_route_except_session(old_route, new_route))
        })
}

fn same_route_except_session(left: &RouteKey, right: &RouteKey) -> bool {
    left.zone == right.zone
        && left.service == right.service
        && left.member == right.member
        && left.target == right.target
        && left.schema == right.schema
        && left.generations.provider == right.generations.provider
        && left.generations.controller == right.generations.controller
}

/// Closed route-registration failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    InvalidMember,
    TargetKindMismatch,
    ZoneMismatch,
    UnauthenticatedTransport,
    SessionBindingMismatch,
    ProviderAssertion,
    DuplicateSessionIdentity,
    DuplicateRoute,
    RouteNotFound,
    SessionNotFound,
    ReconnectGeneration,
    SessionIdExhausted,
    InternalInvariant,
}

impl core::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidMember => "route member must be an exact generated name",
            Self::TargetKindMismatch => "route target kind does not match its resource reference",
            Self::ZoneMismatch => "route or authenticated subject belongs to another Zone",
            Self::UnauthenticatedTransport => "transport evidence cannot register a bus session",
            Self::SessionBindingMismatch => {
                "route does not match the authenticated session binding"
            }
            Self::ProviderAssertion => "Provider route is not bound to authenticated evidence",
            Self::DuplicateSessionIdentity => {
                "authenticated session identity is already registered"
            }
            Self::DuplicateRoute => "exact route is already registered",
            Self::RouteNotFound => "exact route is not registered",
            Self::SessionNotFound => "bus session is not registered",
            Self::ReconnectGeneration => "reconnect is not the exact next session generation",
            Self::SessionIdExhausted => "bus session identity space is exhausted",
            Self::InternalInvariant => "bus registry invariant failed",
        })
    }
}

impl std::error::Error for RegistryError {}
