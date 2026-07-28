//! Exact route registry owned by one Zone.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ControllerGeneration, EvidenceClass, Locality,
    ReconnectGeneration, ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint,
    ServiceName, ZoneId,
    component_session::{Remediation, SessionErrorCode},
};
use d2b_resource_api::authz::SessionVerb;
use d2b_session::{AuthenticatedSessionRouteBinding, OperationMember};

use crate::{
    operations::SessionId,
    router::{DeliveredInvocation, DeliveredStream},
};

/// A method or named-stream member in an exact route.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteMember {
    Method(OperationMember),
    Stream(OperationMember),
}

impl RouteMember {
    /// Construct an exact method member.
    pub fn method(value: impl Into<String>) -> Result<Self, RegistryError> {
        OperationMember::method(value)
            .map(Self::Method)
            .map_err(|_| RegistryError::InvalidMember)
    }

    /// Construct an exact named-stream member.
    pub fn stream(value: impl Into<String>) -> Result<Self, RegistryError> {
        OperationMember::stream(value)
            .map(Self::Stream)
            .map_err(|_| RegistryError::InvalidMember)
    }

    /// Borrow the exact generated member name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Method(value) | Self::Stream(value) => value.as_str(),
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
    Session(EndpointSessionFailure),
}

/// Identity-free session failure classes preserved across bus dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointFailureClass {
    Authentication,
    Authorization,
    Generation,
    Backpressure,
    Deadline,
    Cancellation,
    Transport,
    Protocol,
    Internal,
}

/// Identity-free, actionable session failure preserved at the bus seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointSessionFailure {
    class: EndpointFailureClass,
    code: SessionErrorCode,
    remediation: Remediation,
}

impl EndpointSessionFailure {
    pub const fn class(self) -> EndpointFailureClass {
        self.class
    }

    pub const fn code(self) -> SessionErrorCode {
        self.code
    }

    pub const fn remediation(self) -> Remediation {
        self.remediation
    }
}

impl From<d2b_session::SessionError> for EndpointError {
    fn from(error: d2b_session::SessionError) -> Self {
        use d2b_session::SessionErrorClass as Source;
        let class = match error.class() {
            Source::Authentication => EndpointFailureClass::Authentication,
            Source::Authorization => EndpointFailureClass::Authorization,
            Source::Generation => EndpointFailureClass::Generation,
            Source::Backpressure => EndpointFailureClass::Backpressure,
            Source::Deadline => EndpointFailureClass::Deadline,
            Source::Cancellation => EndpointFailureClass::Cancellation,
            Source::Transport => EndpointFailureClass::Transport,
            Source::Protocol => EndpointFailureClass::Protocol,
            Source::Internal => EndpointFailureClass::Internal,
        };
        Self::Session(EndpointSessionFailure {
            class,
            code: error.code(),
            remediation: error.remediation(),
        })
    }
}

impl core::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "endpoint is unavailable",
            Self::Rejected => "endpoint rejected the request",
            Self::Internal => "endpoint failed",
            Self::Session(failure) => {
                return write!(
                    f,
                    "endpoint session failed class={} code={} remediation={}",
                    failure.class().as_str(),
                    failure.code().as_str(),
                    failure.remediation().as_str()
                );
            }
        })
    }
}

impl EndpointFailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Generation => "generation",
            Self::Backpressure => "backpressure",
            Self::Deadline => "deadline",
            Self::Cancellation => "cancellation",
            Self::Transport => "transport",
            Self::Protocol => "protocol",
            Self::Internal => "internal",
        }
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
    /// Invalidate correlated sends synchronously before route revocation.
    fn invalidate_session(&self) {}

    /// Revalidate the source session's exact operation before dispatch.
    async fn authorize(
        &self,
        _route: &RouteKey,
        _verb: SessionVerb,
        _target: Option<&ResourceRef>,
        _now_tick: u64,
    ) -> Result<(), EndpointError> {
        Err(EndpointError::Unavailable)
    }

    /// Deliver one already-authorized method invocation.
    async fn invoke(&self, request: DeliveredInvocation) -> Result<BusResponse, EndpointError>;

    /// Open one already-authorized named stream.
    async fn open_stream(&self, request: DeliveredStream) -> Result<(), EndpointError>;

    /// Notify the exact destination of an authorized cancellation.
    async fn cancel(
        &self,
        _operation: &crate::operations::OperationId,
    ) -> Result<(), EndpointError> {
        Err(EndpointError::Unavailable)
    }
}

/// Registration input consumed by the Zone's single registration authority.
pub(crate) struct SessionRegistration {
    identity: SessionIdentity,
    context: Option<AuthenticatedSubjectContext>,
    session_authorization: bool,
    routes: Vec<RouteKey>,
    endpoint: Arc<dyn BusEndpoint>,
}

impl SessionRegistration {
    #[cfg(test)]
    pub(crate) fn new(
        context: AuthenticatedSubjectContext,
        routes: Vec<RouteKey>,
        endpoint: Arc<dyn BusEndpoint>,
    ) -> Self {
        Self {
            identity: SessionIdentity::from_context(&context),
            context: Some(context),
            session_authorization: false,
            routes,
            endpoint,
        }
    }

    pub(crate) fn admitted(
        binding: AuthenticatedSessionRouteBinding,
        routes: Vec<RouteKey>,
        endpoint: Arc<dyn BusEndpoint>,
    ) -> Self {
        Self {
            identity: SessionIdentity::from_authenticated(&binding),
            context: Some(binding.context().clone()),
            session_authorization: true,
            routes,
            endpoint,
        }
    }

    #[cfg(test)]
    pub(crate) const fn context(&self) -> Option<&AuthenticatedSubjectContext> {
        self.context.as_ref()
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
    identity: SessionIdentity,
    context: Option<AuthenticatedSubjectContext>,
    session_authorization: bool,
    routes: BTreeSet<RouteKey>,
    endpoint: Arc<dyn BusEndpoint>,
    route_lease: Arc<RouteLeaseState>,
}

#[derive(Clone, PartialEq, Eq)]
struct SessionIdentity {
    zone: ZoneId,
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    evidence_class: EvidenceClass,
    locality: Locality,
    service: ServiceName,
    schema: SchemaFingerprint,
    reconnect_generation: ReconnectGeneration,
    provider_ref: Option<ResourceRef>,
    provider_generation: Option<ResourceGeneration>,
    controller_generation: Option<ControllerGeneration>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PrincipalId(ResourceUid);

impl PrincipalId {
    #[cfg(test)]
    pub(crate) fn test(uid: ResourceUid) -> Self {
        Self(uid)
    }
}

impl SessionIdentity {
    #[cfg(test)]
    fn from_context(context: &AuthenticatedSubjectContext) -> Self {
        Self {
            zone: ZoneId::parse(context.zone_ref().name().as_str())
                .expect("authenticated Zone reference has a valid name"),
            subject_ref: context.subject_ref().clone(),
            subject_uid: context.subject_uid().clone(),
            evidence_class: context.evidence_class(),
            locality: context.transport_binding().locality(),
            service: context.service().clone(),
            schema: context.schema_fingerprint().clone(),
            reconnect_generation: context.reconnect_generation(),
            provider_ref: context.provider_ref().cloned(),
            provider_generation: context.provider_generation(),
            controller_generation: context.controller_generation(),
        }
    }

    fn from_authenticated(binding: &AuthenticatedSessionRouteBinding) -> Self {
        Self {
            zone: binding.zone().clone(),
            subject_ref: binding.subject_ref().clone(),
            subject_uid: binding.subject_uid().clone(),
            evidence_class: binding.evidence_class(),
            locality: binding.locality(),
            service: binding.service().clone(),
            schema: binding.schema().clone(),
            reconnect_generation: binding.reconnect_generation(),
            provider_ref: binding.provider_ref().cloned(),
            provider_generation: binding.provider_generation(),
            controller_generation: binding.controller_generation(),
        }
    }
}

struct RouteLeaseState {
    revoked: Mutex<bool>,
}

pub(crate) struct RevocableRouteLease {
    destination: SessionId,
    destination_principal: PrincipalId,
    generation: ReconnectGeneration,
    endpoint: Arc<dyn BusEndpoint>,
    state: Arc<RouteLeaseState>,
}

impl RevocableRouteLease {
    #[cfg(test)]
    pub(crate) fn test(
        destination: SessionId,
        generation: ReconnectGeneration,
        endpoint: Arc<dyn BusEndpoint>,
    ) -> Self {
        Self {
            destination,
            destination_principal: PrincipalId(
                ResourceUid::parse("00000000-0000-4000-8000-000000000001").unwrap(),
            ),
            generation,
            endpoint,
            state: Arc::new(RouteLeaseState {
                revoked: Mutex::new(false),
            }),
        }
    }

    pub(crate) const fn destination(&self) -> SessionId {
        self.destination
    }

    pub(crate) fn destination_principal(&self) -> PrincipalId {
        self.destination_principal.clone()
    }

    pub(crate) const fn generation(&self) -> ReconnectGeneration {
        self.generation
    }

    pub(crate) fn endpoint(&self) -> Arc<dyn BusEndpoint> {
        Arc::clone(&self.endpoint)
    }

    pub(crate) fn with_active<T>(&self, action: impl FnOnce() -> T) -> Result<T, RegistryError> {
        let revoked = self
            .state
            .revoked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *revoked {
            return Err(RegistryError::RouteRevoked);
        }
        Ok(action())
    }
}

pub(crate) struct ResolvedSource {
    pub(crate) context: Option<AuthenticatedSubjectContext>,
    pub(crate) endpoint: Arc<dyn BusEndpoint>,
    pub(crate) session_authorization: bool,
}

pub(crate) struct Registry {
    zone: ZoneId,
    max_routes_per_session: usize,
    max_total_routes: usize,
    next_session: u64,
    sessions: BTreeMap<SessionId, RegisteredSession>,
    routes: BTreeMap<RouteKey, SessionId>,
}

impl Registry {
    pub(crate) fn new(
        zone: ZoneId,
        max_routes_per_session: usize,
        max_total_routes: usize,
    ) -> Self {
        Self {
            zone,
            max_routes_per_session,
            max_total_routes,
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
        if prior.identity.subject_ref != registration.identity.subject_ref
            || prior.identity.subject_uid != registration.identity.subject_uid
            || prior.identity.zone != registration.identity.zone
            || registration.identity.reconnect_generation.get()
                != prior
                    .identity
                    .reconnect_generation
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
        prior.endpoint.invalidate_session();
        self.remove(previous);
        self.install(session, registration);
        Ok(session)
    }

    fn validate_registration(
        &self,
        registration: &SessionRegistration,
        replacing: Option<SessionId>,
    ) -> Result<(), RegistryError> {
        let replacing_routes = replacing
            .and_then(|session| self.sessions.get(&session))
            .map_or(0, |registered| registered.routes.len());
        if registration.routes.len() > self.max_routes_per_session
            || self
                .routes
                .len()
                .saturating_sub(replacing_routes)
                .saturating_add(registration.routes.len())
                > self.max_total_routes
        {
            return Err(RegistryError::RouteCapacity);
        }
        ensure_context_zone(&registration.identity, &self.zone)?;
        ensure_transport_evidence(&registration.identity)?;
        if self.sessions.iter().any(|(session, registered)| {
            Some(*session) != replacing
                && registered.identity.subject_ref == registration.identity.subject_ref
                && registered.identity.subject_uid == registration.identity.subject_uid
        }) {
            return Err(RegistryError::DuplicateSessionIdentity);
        }

        let distinct = registration.routes.iter().cloned().collect::<BTreeSet<_>>();
        if distinct.len() != registration.routes.len() {
            return Err(RegistryError::DuplicateRoute);
        }
        for route in &registration.routes {
            validate_route_binding(&self.zone, &registration.identity, route)?;
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
                identity: registration.identity,
                context: registration.context,
                session_authorization: registration.session_authorization,
                routes,
                endpoint: registration.endpoint,
                route_lease: Arc::new(RouteLeaseState {
                    revoked: Mutex::new(false),
                }),
            },
        );
    }

    pub(crate) fn remove(&mut self, session: SessionId) -> bool {
        let Some(registered) = self.sessions.remove(&session) else {
            return false;
        };
        *registered
            .route_lease
            .revoked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        for route in registered.routes {
            self.routes.remove(&route);
        }
        true
    }

    pub(crate) fn invalidate(&self, session: SessionId) -> bool {
        let Some(registered) = self.sessions.get(&session) else {
            return false;
        };
        registered.endpoint.invalidate_session();
        true
    }

    pub(crate) fn source(&self, session: SessionId) -> Result<ResolvedSource, RegistryError> {
        let registered = self
            .sessions
            .get(&session)
            .ok_or(RegistryError::SessionNotFound)?;
        Ok(ResolvedSource {
            context: registered.context.clone(),
            endpoint: Arc::clone(&registered.endpoint),
            session_authorization: registered.session_authorization,
        })
    }

    pub(crate) fn principal(&self, session: SessionId) -> Result<PrincipalId, RegistryError> {
        self.sessions
            .get(&session)
            .map(|registered| PrincipalId(registered.identity.subject_uid.clone()))
            .ok_or(RegistryError::SessionNotFound)
    }

    pub(crate) fn resolve(&self, route: &RouteKey) -> Result<RevocableRouteLease, RegistryError> {
        if route.zone != self.zone {
            return Err(RegistryError::ZoneMismatch);
        }
        let session = *self.routes.get(route).ok_or(RegistryError::RouteNotFound)?;
        let registered = self
            .sessions
            .get(&session)
            .ok_or(RegistryError::InternalInvariant)?;
        Ok(RevocableRouteLease {
            destination: session,
            destination_principal: PrincipalId(registered.identity.subject_uid.clone()),
            generation: route.generations().session(),
            endpoint: Arc::clone(&registered.endpoint),
            state: Arc::clone(&registered.route_lease),
        })
    }
}

fn ensure_context_zone(identity: &SessionIdentity, zone: &ZoneId) -> Result<(), RegistryError> {
    if &identity.zone != zone {
        return Err(RegistryError::ZoneMismatch);
    }
    Ok(())
}

fn ensure_transport_evidence(identity: &SessionIdentity) -> Result<(), RegistryError> {
    match identity.locality {
        Locality::Local => Ok(()),
        Locality::AdjacentZone
            if identity.evidence_class == EvidenceClass::EnrolledKk
                && matches!(
                    identity.subject_ref.resource_type().as_str(),
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
    identity: &SessionIdentity,
    route: &RouteKey,
) -> Result<(), RegistryError> {
    if route.zone() != zone {
        return Err(RegistryError::ZoneMismatch);
    }
    if &identity.service != route.service()
        || &identity.schema != route.schema()
        || identity.reconnect_generation != route.generations().session()
        || identity.provider_generation != route.generations().provider()
        || identity.controller_generation != route.generations().controller()
    {
        return Err(RegistryError::SessionBindingMismatch);
    }

    if identity.locality == Locality::Local {
        if identity.subject_ref.resource_type().as_str() == "Provider"
            && identity.provider_ref.as_ref() != Some(&identity.subject_ref)
        {
            return Err(RegistryError::ProviderAssertion);
        }
        if let RouteTarget::Provider(provider) = route.target()
            && identity.provider_ref.as_ref() != Some(provider)
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
    RouteCapacity,
    RouteNotFound,
    RouteRevoked,
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
            Self::RouteCapacity => "route registration capacity is exhausted",
            Self::RouteNotFound => "exact route is not registered",
            Self::RouteRevoked => "exact route registration was revoked",
            Self::SessionNotFound => "bus session is not registered",
            Self::ReconnectGeneration => "reconnect is not the exact next session generation",
            Self::SessionIdExhausted => "bus session identity space is exhausted",
            Self::InternalInvariant => "bus registry invariant failed",
        })
    }
}

impl std::error::Error for RegistryError {}
