//! ComponentSession admission for notification streams.
//!
//! Two authorities used to live beside the graph: a hand-matched service
//! package string that decided whether a route was a notification session,
//! and the host presentation channel a delivered notification was presented
//! through. Both are declared here instead.
//!
//! [`NOTIFICATION_SERVICE`] is the Provider's one service-method source. The
//! session admission resolves the route's service and purpose through it, so
//! a purpose is only ever admitted because a declared method serves it.
//!
//! [`notification_endpoint_bindings`] is the Provider's one endpoint-grant
//! source, and [`admit_notification_endpoint`] is the only gate that admits
//! one. A missing or withdrawn relationship is refused at
//! [`NotificationEndpointRefusal::relationship_absent`], which is what stops a
//! delivery instead of letting it reach the desktop presentation another way.
//!
//! Nothing here reads notification content. A summary, a body, an action
//! label, or a caller-supplied opaque key can never become a slot, a purpose,
//! a consumer, or a source endpoint, because no content value is an input to
//! any derivation in this module.

use d2b_contracts_resource::v3::identity::EvidenceClass;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingKey, BindingSlot, DesiredRevision, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, execution_policy::BoundedToken,
    identity::ReconnectGeneration,
};
use d2b_provider_toolkit::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

/// The declared methods of the notification service.
///
/// The service carries three declared purposes, and the method names are what
/// make them distinct: a source session is admitted for `deliver-source`, a
/// desktop observer for `notify-observer` or `invoke-action`, and the guest
/// source configuration is read through `reconcile-sources`.
pub const NOTIFICATION_METHODS: &[ServiceMethod] = &[
    ServiceMethod::zone_plane("deliver-source"),
    ServiceMethod::zone_plane("notify-observer"),
    ServiceMethod::zone_plane("invoke-action"),
    ServiceMethod::zone_plane("close-notification"),
    ServiceMethod::zone_plane("reconcile-sources"),
];

/// The notification Provider's declared service.
///
/// The two declared streams are the Guest-source to host-sink carriage and
/// the host-sink to observer carriage. Neither carries the Provider's own
/// state: a notification is transient and lives only for the session.
pub const NOTIFICATION_SERVICE: ServiceDecl = ServiceDecl {
    id: crate::SERVICE_PACKAGE,
    methods: NOTIFICATION_METHODS,
    attach_kinds: &[],
    streams: &[crate::SINK_STREAM, crate::OBSERVER_STREAM],
    endpoint_policy: None,
};

/// The display family's declared service package, through which a local
/// desktop `User` or an authenticated display `Guest` reaches the
/// notification Provider.
///
/// The identifier is the display Provider's own declaration (U26); the
/// notification Provider consumes it as a declared dependency and never mints
/// or mutates it.
pub const DISPLAY_DESKTOP_SERVICE: &str = "d2b.display.v3";

/// The display family's declared Provider reference, the authority behind
/// [`DISPLAY_DESKTOP_SERVICE`]. It is the crate's one display-family Provider
/// reference, consumed here as a declared dependency.
pub const DISPLAY_DESKTOP_PROVIDER_REF: &str = "Provider/display-wayland";

/// Stream admission purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPurpose {
    /// Guest source to host sink stream.
    NotificationSource,
    /// Local desktop observer stream.
    DesktopObserver,
}

impl AdmissionPurpose {
    /// The declared service method this purpose is admitted for.
    ///
    /// The mapping is one-way and total: a route is admitted for a purpose
    /// because the Provider's declared service names the method that serves
    /// it, never because the route's package string looked right.
    pub const fn declared_method(self) -> &'static str {
        match self {
            Self::NotificationSource => "deliver-source",
            Self::DesktopObserver => "notify-observer",
        }
    }
}

/// Whether one authenticated route is this Provider's service and the route
/// carries a Provider generation.
///
/// This is the Provider-ref half of session admission. The purpose half is
/// [`route_serves_declared_method`], which additionally requires the declared
/// service to answer the method that purpose names.
fn route_serves_declared_method(
    route: &AuthenticatedSessionRouteBinding,
    purpose: AdmissionPurpose,
) -> bool {
    route.service().as_str() == NOTIFICATION_SERVICE.id
        && route
            .provider_ref()
            .is_some_and(|provider| provider.to_canonical_string() == crate::PROVIDER_REF)
        && route.provider_generation().is_some()
        && NOTIFICATION_SERVICE.declares_method(purpose.declared_method())
}

/// Whether one authenticated route is the display family's declared desktop
/// dependency, through which a local desktop identity reaches this Provider.
///
/// The desktop identity itself is still taken from the committed `User` row
/// the caller selects; this only establishes that the route was authenticated
/// by the display family's declared service rather than by a name.
fn route_serves_display_dependency(route: &AuthenticatedSessionRouteBinding) -> bool {
    route.service().as_str() == DISPLAY_DESKTOP_SERVICE
        && route.provider_ref().is_some_and(|provider| {
            provider.to_canonical_string() == DISPLAY_DESKTOP_PROVIDER_REF
        })
}

/// Transport class used by a notification session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportClass {
    /// Enrolled Noise KK transport.
    EnrolledNoiseKk,
    /// Local Unix seqpacket with SO_PEERCRED admission.
    UnixSeqpacket,
    /// Any other transport.
    Other,
}

/// Authenticated notification session projected from ComponentSession.
///
/// There is no public constructor from booleans, strings, or observer labels.
/// The two admission constructors consume the route metadata produced by the
/// canonical session authority.
#[derive(Debug, PartialEq, Eq)]
pub struct SessionEvidence {
    subject_ref: ResourceRef,
    zone: ZoneId,
    generation: u64,
    purpose: AdmissionPurpose,
    transport: TransportClass,
}

impl SessionEvidence {
    /// Admit a notification session directly from the canonical ComponentSession.
    pub fn from_component_session<C>(
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<Self, AdmissionError> {
        let evidence = Self::from_authenticated_route(session.route_binding())?;
        evidence.admit()?;
        Ok(evidence)
    }

    /// Admit notification evidence from a route authenticated and registered
    /// by the daemon's Zone bus.
    pub fn from_authenticated_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        let evidence = Self::from_route_binding(route)?;
        evidence.admit()?;
        Ok(evidence)
    }

    /// Admit a daemon-local Guest source route after ComponentSession
    /// authentication and Zone registration.
    pub fn from_daemon_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        if !route_serves_declared_method(
            &route,
            AdmissionPurpose::NotificationSource,
        ) || route.evidence_class() != EvidenceClass::UnixPeer
            || route.locality() != d2b_contracts_resource::v3::identity::Locality::Local
            || route.subject_ref().resource_type().as_str() != "Guest"
            || route.reconnect_generation().get() == 0
        {
            return Err(AdmissionError::SessionUnauthenticated);
        }
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::NotificationSource,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Project an authenticated Provider transport into one Guest selected
    /// from committed daemon state.
    pub fn from_daemon_route_for_guest(
        route: AuthenticatedSessionRouteBinding,
        guest_ref: ResourceRef,
    ) -> Result<Self, AdmissionError> {
        if !route_serves_declared_method(
            &route,
            AdmissionPurpose::NotificationSource,
        ) || route.evidence_class() != EvidenceClass::UnixPeer
            || route.locality() != d2b_contracts_resource::v3::identity::Locality::Local
            || route.subject_ref().resource_type().as_str() != "Provider"
            || guest_ref.resource_type().as_str() != "Guest"
            || route.reconnect_generation().get() == 0
        {
            return Err(AdmissionError::SessionUnauthenticated);
        }
        Ok(Self {
            subject_ref: guest_ref,
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::NotificationSource,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Admit a local desktop observer from the daemon's authenticated display
    /// route.  The observer identity is still taken from verified route
    /// metadata; only the service package differs because the display route
    /// is the daemon's local desktop authority.
    pub fn from_display_observer_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        if !route_serves_display_dependency(&route)
            || route.evidence_class() != EvidenceClass::UnixPeer
            || route.locality() != d2b_contracts_resource::v3::identity::Locality::Local
            || route.subject_ref().resource_type().as_str() != "User"
            || route.provider_generation().is_none()
            || route.reconnect_generation().get() == 0
        {
            return Err(AdmissionError::SessionUnauthenticated);
        }
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::DesktopObserver,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Admit the host observer identity from a display dependency route and
    /// its committed User resource.  The route authenticates the display
    /// session and Zone; the resource-plane User reference is accepted only
    /// after the daemon has committed the matching dependency.
    pub fn from_display_dependency_route(
        route: AuthenticatedSessionRouteBinding,
        user_ref: ResourceRef,
    ) -> Result<Self, AdmissionError> {
        if !route_serves_display_dependency(&route)
            || route.evidence_class() != EvidenceClass::UnixPeer
            || route.locality() != d2b_contracts_resource::v3::identity::Locality::Local
            || route.subject_ref().resource_type().as_str() != "Guest"
            || user_ref.resource_type().as_str() != "User"
            || route.provider_generation().is_none()
            || route.reconnect_generation().get() == 0
        {
            return Err(AdmissionError::SessionUnauthenticated);
        }
        Ok(Self {
            subject_ref: user_ref,
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::DesktopObserver,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Admit a Guest source from an authenticated enrolled session.
    #[allow(dead_code)]
    pub(crate) fn from_source_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        validate_route(&route, EvidenceClass::EnrolledKk, false)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::NotificationSource,
            transport: TransportClass::EnrolledNoiseKk,
        })
    }

    /// Admit a local desktop observer from authenticated Unix peer evidence.
    #[allow(dead_code)]
    pub(crate) fn from_observer_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        validate_route(&route, EvidenceClass::UnixPeer, true)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::DesktopObserver,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Check all fixed service, transport, and authentication requirements.
    pub fn admit(&self) -> Result<(), AdmissionError> {
        if self.generation == 0 {
            return Err(AdmissionError::SessionNotEstablished);
        }
        match (self.purpose, self.transport) {
            (AdmissionPurpose::NotificationSource, TransportClass::EnrolledNoiseKk)
            | (AdmissionPurpose::NotificationSource, TransportClass::UnixSeqpacket)
            | (AdmissionPurpose::DesktopObserver, TransportClass::UnixSeqpacket) => Ok(()),
            _ => Err(AdmissionError::TransportMismatch),
        }
    }

    fn from_route_binding(route: AuthenticatedSessionRouteBinding) -> Result<Self, AdmissionError> {
        let subject_type = route.subject_ref().resource_type().as_str();
        let (purpose, transport, expected_evidence, local_only) = match subject_type {
            "Guest" => (
                AdmissionPurpose::NotificationSource,
                TransportClass::EnrolledNoiseKk,
                EvidenceClass::EnrolledKk,
                false,
            ),
            "User" => (
                AdmissionPurpose::DesktopObserver,
                TransportClass::UnixSeqpacket,
                EvidenceClass::UnixPeer,
                true,
            ),
            _ => return Err(AdmissionError::SessionUnauthenticated),
        };
        validate_route(&route, expected_evidence, local_only)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose,
            transport,
        })
    }

    /// Admit the session specifically for host-observer delivery and action
    /// invocation.
    pub fn admit_observer(&self) -> Result<(), AdmissionError> {
        self.admit()?;
        if self.purpose == AdmissionPurpose::DesktopObserver {
            Ok(())
        } else {
            Err(AdmissionError::TransportMismatch)
        }
    }

    /// Admit the session specifically for a Guest notification source.
    pub fn admit_source(&self) -> Result<(), AdmissionError> {
        self.admit()?;
        if self.purpose == AdmissionPurpose::NotificationSource {
            Ok(())
        } else {
            Err(AdmissionError::TransportMismatch)
        }
    }

    /// Return the subject/Zone/generation binding used for nonce state.
    pub fn session_key(&self) -> String {
        format!(
            "{}@{}#{}",
            self.subject_ref.to_canonical_string(),
            self.zone.as_str(),
            self.generation
        )
    }

    /// Borrow the authenticated subject reference for exact source binding.
    pub const fn subject_ref(&self) -> &ResourceRef {
        &self.subject_ref
    }

    /// Whether this evidence is a Guest source session.
    pub const fn is_source(&self) -> bool {
        matches!(self.purpose, AdmissionPurpose::NotificationSource)
    }

    /// Borrow the authenticated Zone.
    pub fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the authenticated reconnect generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
pub(crate) fn test_observer(subject: &str) -> SessionEvidence {
    let subject_ref = format!("User/{subject}");
    SessionEvidence {
        subject_ref: ResourceRef::parse(&subject_ref).unwrap(),
        zone: ZoneId::parse("work").unwrap(),
        generation: 1,
        purpose: AdmissionPurpose::DesktopObserver,
        transport: TransportClass::UnixSeqpacket,
    }
}

#[cfg(test)]
pub(crate) fn test_source(subject: &str) -> SessionEvidence {
    test_source_at(subject, 1)
}

#[cfg(test)]
pub(crate) fn test_source_at(subject: &str, generation: u64) -> SessionEvidence {
    test_source_at_zone(subject, generation, "work")
}

#[cfg(test)]
pub(crate) fn test_source_at_zone(subject: &str, generation: u64, zone: &str) -> SessionEvidence {
    let subject_ref = format!("Guest/{subject}");
    SessionEvidence {
        subject_ref: ResourceRef::parse(&subject_ref).unwrap(),
        zone: ZoneId::parse(zone).unwrap(),
        generation,
        purpose: AdmissionPurpose::NotificationSource,
        transport: TransportClass::EnrolledNoiseKk,
    }
}

fn validate_route(
    route: &AuthenticatedSessionRouteBinding,
    expected_evidence: EvidenceClass,
    local_only: bool,
) -> Result<(), AdmissionError> {
    if route.service().as_str() != NOTIFICATION_SERVICE.id {
        return Err(AdmissionError::ServiceMismatch);
    }
    if !route.provider_ref().is_some_and(|provider| {
        provider.to_canonical_string() == crate::PROVIDER_REF
    }) || route.provider_generation().is_none()
    {
        return Err(AdmissionError::SessionUnauthenticated);
    }
    if route.evidence_class() != expected_evidence
        || (local_only && route.locality() != d2b_contracts_resource::v3::identity::Locality::Local)
    {
        return Err(AdmissionError::TransportMismatch);
    }
    let subject_type = route.subject_ref().resource_type().as_str();
    if (local_only && subject_type != "User") || (!local_only && subject_type != "Guest") {
        return Err(AdmissionError::SessionUnauthenticated);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_specific_admission_rejects_cross_role_reuse() {
        let observer = test_observer("alice");
        let source = test_source("guest");
        assert!(observer.admit_observer().is_ok());
        assert!(observer.admit_source().is_err());
        assert!(source.admit_source().is_ok());
        assert!(source.admit_observer().is_err());
    }

    #[test]
    fn session_key_binds_subject_zone_and_reconnect_generation() {
        let first = test_observer("alice");
        let second = test_observer("bob");
        assert_ne!(first.session_key(), second.session_key());
        assert_eq!(first.zone().as_str(), "work");
        assert_eq!(first.generation(), 1);
    }

    #[test]
    fn zero_reconnect_generation_is_not_admitted() {
        assert_eq!(
            test_source_at("guest", 0).admit(),
            Err(AdmissionError::SessionNotEstablished)
        );
    }
}

/// Stable session admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    /// Session handshake is incomplete.
    SessionNotEstablished,
    /// Session has no authenticated caller.
    SessionUnauthenticated,
    /// Service package is not the v3 package.
    ServiceMismatch,
    /// Transport is not permitted for the selected purpose.
    TransportMismatch,
}

impl core::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionNotEstablished => "session-not-established",
            Self::SessionUnauthenticated => "session-unauthenticated",
            Self::ServiceMismatch => "session-service-mismatch",
            Self::TransportMismatch => "session-untrusted-transport",
        })
    }
}

impl std::error::Error for AdmissionError {}

// ---------------------------------------------------------------------------
// Declared endpoint grants (U28)
// ---------------------------------------------------------------------------

/// One declared notification delivery channel and the endpoint relationship it
/// requires.
///
/// The two roles are the Provider's declared streams. Each carries its own
/// stable consumer slot and its own bounded purpose, so a Guest-source
/// relationship can never be reused as the desktop presentation channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NotificationEndpointRole {
    /// The Guest-source stream a notification request arrives on.
    GuestSource,
    /// The desktop presentation channel a sanitized notification is shown on.
    DesktopSink,
}

impl NotificationEndpointRole {
    /// Every delivery role, in the Provider's preserved stream order.
    pub const ALL: [Self; 2] = [Self::GuestSource, Self::DesktopSink];

    /// The declared stream this channel carries.
    pub const fn stream(self) -> &'static str {
        match self {
            Self::GuestSource => crate::SINK_STREAM,
            Self::DesktopSink => crate::OBSERVER_STREAM,
        }
    }

    /// The stable consumer slot this channel is admitted under.
    pub const fn slot(self) -> &'static str {
        match self {
            Self::GuestSource => "notification-guest-source",
            Self::DesktopSink => "notification-desktop-sink",
        }
    }

    /// The bounded usage purpose this channel is admitted for.
    pub const fn purpose(self) -> &'static str {
        match self {
            Self::GuestSource => "notification-guest-source",
            Self::DesktopSink => "notification-desktop-presentation",
        }
    }

    /// The attachment form the consumer uses to reach this channel.
    pub const fn attachment(self) -> EndpointAttachmentKind {
        EndpointAttachmentKind::Connect
    }
}

/// The exact `Endpoint` rows the notification Provider declares for its two
/// delivery channels.
///
/// The references are row identities, not locators: the desktop session bus
/// connection and the Guest stream socket stay private to their own
/// realizations, and the composing side reads these references from the
/// committed graph rather than from a name or an environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationHostEndpoints {
    guest_source: ResourceRef,
    desktop_sink: ResourceRef,
}

impl NotificationHostEndpoints {
    /// Bind one declared `Endpoint` row to each delivery channel.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationEndpointError::SourceNotEndpoint`] when either
    /// reference is not an `Endpoint` row.
    pub fn new(
        guest_source: ResourceRef,
        desktop_sink: ResourceRef,
    ) -> Result<Self, NotificationEndpointError> {
        for source in [&guest_source, &desktop_sink] {
            if source.resource_type().as_str() != "Endpoint" {
                return Err(NotificationEndpointError::SourceNotEndpoint);
            }
        }
        Ok(Self {
            guest_source,
            desktop_sink,
        })
    }

    /// Borrow the declared `Endpoint` row of one channel.
    pub const fn source_ref(&self, role: NotificationEndpointRole) -> &ResourceRef {
        match role {
            NotificationEndpointRole::GuestSource => &self.guest_source,
            NotificationEndpointRole::DesktopSink => &self.desktop_sink,
        }
    }
}

/// One derived notification endpoint relationship.
///
/// The relationship is derived from the Provider's declared stream vocabulary
/// and the consumer row's identity, so a consumer can only ever reach the
/// exact endpoint this value names, in the declared attachment form, for the
/// declared purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEndpointBinding {
    role: NotificationEndpointRole,
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    request: EndpointBindingRequest,
}

impl NotificationEndpointBinding {
    /// Return the delivery channel this relationship belongs to.
    pub const fn role(&self) -> NotificationEndpointRole {
        self.role
    }

    /// Borrow the exact source `Endpoint`.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer row.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the typed binding request.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }
}

/// Every endpoint relationship one notification consumer row requires, in the
/// Provider's preserved stream order.
///
/// # Errors
///
/// Returns [`NotificationEndpointError::RequestRejected`] when a declared slot
/// or purpose cannot be expressed as a bounded token, or when the consumer row
/// is not a kind an `EndpointBinding` admits.
pub fn notification_endpoint_bindings(
    endpoints: &NotificationHostEndpoints,
    consumer: &ResourceRef,
) -> Result<Vec<NotificationEndpointBinding>, NotificationEndpointError> {
    let mut bindings = Vec::with_capacity(NotificationEndpointRole::ALL.len());
    for role in NotificationEndpointRole::ALL {
        let source_ref = endpoints.source_ref(role).clone();
        let slot = BindingSlot::parse(role.slot())
            .map_err(|_| NotificationEndpointError::RequestRejected)?;
        let purpose = BoundedToken::parse(role.purpose())
            .map_err(|_| NotificationEndpointError::RequestRejected)?;
        let request = EndpointBindingRequest::new(
            source_ref.clone(),
            consumer.clone(),
            slot,
            role.attachment(),
            purpose,
        )
        .map_err(|_| NotificationEndpointError::RequestRejected)?;
        bindings.push(NotificationEndpointBinding {
            role,
            source_ref,
            consumer_ref: consumer.clone(),
            request,
        });
    }
    Ok(bindings)
}

/// The lifecycle one admitted notification endpoint relationship is in.
///
/// A relationship starts [`Self::Admitted`] and reaches every other phase only
/// through a transition, so a revocation cannot be expressed as a
/// construction and delivery cannot begin in a revoked state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEndpointPhase {
    /// The relationship is admitted and may carry new deliveries.
    Admitted,
    /// Outstanding use is being driven to its safe state.
    Draining,
    /// The relationship is revoked; nothing new may be delivered.
    Revoked,
}

/// The committed evidence one admitted notification endpoint relationship is
/// fenced against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEndpointFence {
    zone: ZoneId,
    store: StoreIncarnation,
    desired_revision: DesiredRevision,
    sequence: ZoneDesiredSequence,
    source_generation: ResourceGeneration,
    consumer_generation: ResourceGeneration,
    minimum_reconnect: ReconnectGeneration,
    phase: NotificationEndpointPhase,
}

impl NotificationEndpointFence {
    /// Construct the fence of a freshly admitted relationship.
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is one committed authority input"
    )]
    pub const fn new(
        zone: ZoneId,
        store: StoreIncarnation,
        desired_revision: DesiredRevision,
        sequence: ZoneDesiredSequence,
        source_generation: ResourceGeneration,
        consumer_generation: ResourceGeneration,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            zone,
            store,
            desired_revision,
            sequence,
            source_generation,
            consumer_generation,
            minimum_reconnect,
            phase: NotificationEndpointPhase::Admitted,
        }
    }

    /// Borrow the Zone this relationship belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store incarnation the relationship was admitted in.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// Return the committed desired revision the relationship is fenced at.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }

    /// Return the Zone desired sequence the relationship is fenced at.
    pub const fn sequence(&self) -> ZoneDesiredSequence {
        self.sequence
    }

    /// Return the source generation the relationship is fenced at.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// Return the consumer generation the relationship is fenced at.
    pub const fn consumer_generation(&self) -> ResourceGeneration {
        self.consumer_generation
    }

    /// Return the lowest reconnect generation this relationship still admits.
    pub const fn minimum_reconnect(&self) -> ReconnectGeneration {
        self.minimum_reconnect
    }

    /// Return the lifecycle phase the relationship is in.
    pub const fn phase(&self) -> NotificationEndpointPhase {
        self.phase
    }

    /// Fence the relationship at a newer committed desired revision.
    #[must_use]
    pub fn advance_desired_revision(mut self, desired_revision: DesiredRevision) -> Self {
        self.desired_revision = desired_revision;
        self
    }

    /// Fence the relationship at a newer Zone desired sequence.
    #[must_use]
    pub fn advance_sequence(mut self, sequence: ZoneDesiredSequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// Raise the lowest reconnect generation this relationship admits.
    #[must_use]
    pub fn raise_minimum_reconnect(
        mut self,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        self.minimum_reconnect = minimum_reconnect;
        self
    }

    /// Move the relationship to the draining phase.
    #[must_use]
    pub fn drain(mut self) -> Self {
        self.phase = NotificationEndpointPhase::Draining;
        self
    }

    /// Move the relationship to the revoked phase.
    #[must_use]
    pub fn revoke(mut self) -> Self {
        self.phase = NotificationEndpointPhase::Revoked;
        self
    }
}

/// Live, non-secret observations one delivery attempt presents.
///
/// The evidence is measured against the fence and never merges with it: a
/// caller cannot move a fence by presenting evidence, and a fence cannot
/// adopt a caller's claims without the graph having committed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEndpointEvidence {
    /// The Zone the attempt claims to be operating in.
    pub zone: ZoneId,
    /// The store incarnation the attempt observed.
    pub store: StoreIncarnation,
    /// The source generation the attempt observed.
    pub source_generation: ResourceGeneration,
    /// The consumer generation the attempt observed.
    pub consumer_generation: ResourceGeneration,
    /// The desired revision the attempt observed.
    pub desired_revision: DesiredRevision,
    /// The Zone desired sequence the attempt observed.
    pub sequence: ZoneDesiredSequence,
    /// The reconnect generation this attempt is.
    pub reconnect: ReconnectGeneration,
}

/// One admitted notification endpoint relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedNotificationEndpoint {
    binding: NotificationEndpointBinding,
    key: BindingKey,
    fence: NotificationEndpointFence,
}

impl AdmittedNotificationEndpoint {
    /// Return the admitted delivery channel.
    pub const fn role(&self) -> NotificationEndpointRole {
        self.binding.role
    }

    /// Borrow the admitted relationship.
    pub const fn binding(&self) -> &NotificationEndpointBinding {
        &self.binding
    }

    /// Borrow the KTD3 identity of the admitted relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the fence this admission was proved against.
    pub const fn fence(&self) -> &NotificationEndpointFence {
        &self.fence
    }

    /// Whether this admission still admits new delivery.
    pub fn admits_delivery(&self) -> bool {
        self.fence.phase() == NotificationEndpointPhase::Admitted
    }
}

/// One closed, field-free refusal from the notification endpoint gate.
///
/// The stage and reason are the graph's own vocabulary and the code is a
/// stable label: a refusal never echoes a Zone, a notification, an action
/// label, or any caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationEndpointRefusal {
    stage: AdmissionStage,
    reason: RefusalReason,
    code: &'static str,
}

impl NotificationEndpointRefusal {
    const fn at(stage: AdmissionStage, reason: RefusalReason, code: &'static str) -> Self {
        Self { stage, reason, code }
    }

    /// The refusal for a delivery channel that carries no admitted
    /// relationship at all.
    ///
    /// This is the fail-closed answer for a missing endpoint: the delivery is
    /// refused before the desktop presentation port is touched, so a withdrawn
    /// relationship can never redirect the notification onto another channel.
    pub const fn relationship_absent() -> Self {
        Self::at(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
            "notification-endpoint-relationship-absent",
        )
    }

    /// The refusal for a relationship the graph has revoked.
    pub const fn relationship_revoked() -> Self {
        Self::at(
            AdmissionStage::Revoke,
            RefusalReason::StaleAuthority,
            "notification-endpoint-relationship-revoked",
        )
    }

    /// The refusal for a relationship the graph is draining.
    pub const fn relationship_draining() -> Self {
        Self::at(
            AdmissionStage::Drain,
            RefusalReason::UnprovenEffect,
            "notification-endpoint-relationship-draining",
        )
    }

    /// The refusal for a relationship presented under the wrong channel.
    pub const fn channel_mismatch() -> Self {
        Self::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "notification-endpoint-channel-mismatch",
        )
    }

    /// Return the enforcing stage.
    pub const fn stage(self) -> AdmissionStage {
        self.stage
    }

    /// Return the typed refusal reason.
    pub const fn reason(self) -> RefusalReason {
        self.reason
    }

    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl core::fmt::Display for NotificationEndpointRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for NotificationEndpointRefusal {}

/// The complete root-admitted inputs of one notification endpoint delivery.
///
/// Grouping them keeps the gate a single argument, so a delivery path cannot
/// present a subset of the evidence and be admitted on it.
pub struct NotificationEndpointGate<'a> {
    /// The declared `Endpoint` rows of the Provider's two channels.
    pub endpoints: &'a NotificationHostEndpoints,
    /// The exact consumer row the relationship belongs to.
    pub consumer: &'a ResourceRef,
    /// The relationship the attempt is presenting.
    pub binding: &'a NotificationEndpointBinding,
    /// The committed fence the evidence is measured against.
    pub fence: &'a NotificationEndpointFence,
    /// The live observations the attempt presents.
    pub evidence: &'a NotificationEndpointEvidence,
    /// The committed source row identity.
    pub source_uid: &'a ResourceUid,
    /// The committed consumer row identity.
    pub consumer_uid: &'a ResourceUid,
}

/// Notification endpoint derivation and admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEndpointError {
    /// A declared delivery channel did not name an `Endpoint` row.
    SourceNotEndpoint,
    /// The declared relationship could not be expressed as a typed request.
    RequestRejected,
    /// The observed delivery attempt was refused by the graph.
    Refused(NotificationEndpointRefusal),
}

impl core::fmt::Display for NotificationEndpointError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SourceNotEndpoint => {
                formatter.write_str("notification-endpoint-source-invalid")
            }
            Self::RequestRejected => {
                formatter.write_str("notification-endpoint-request-rejected")
            }
            Self::Refused(refusal) => formatter.write_str(refusal.code()),
        }
    }
}

impl std::error::Error for NotificationEndpointError {}

impl From<NotificationEndpointRefusal> for NotificationEndpointError {
    fn from(value: NotificationEndpointRefusal) -> Self {
        Self::Refused(value)
    }
}

/// The one ordered gate every notification endpoint delivery path runs.
///
/// The relationship is re-derived from the Provider's declared stream
/// vocabulary before anything is compared: an attempt that names another
/// stream's slot, purpose, attachment, or source endpoint is refused as a
/// conflicting declaration rather than narrowed into the one it supplied. The
/// gate stops at the first refusal, so the reported stage is always the
/// earliest reason the attempt is not admitted, and evidence is only ever read.
pub fn admit_notification_endpoint(
    gate: &NotificationEndpointGate<'_>,
) -> Result<AdmittedNotificationEndpoint, NotificationEndpointRefusal> {
    let endpoints = gate.endpoints;
    let fence = gate.fence;
    let evidence = gate.evidence;
    let expected = notification_endpoint_bindings(endpoints, gate.consumer)
        .ok()
        .and_then(|derived| {
            derived
                .into_iter()
                .find(|declared| declared.role() == gate.binding.role())
        });
    let Some(expected) = expected else {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "notification-endpoint-undeclared-stream",
        ));
    };
    if expected.request != *gate.binding.request()
        || expected.source_ref != *gate.binding.source_ref()
        || expected.consumer_ref != *gate.binding.consumer_ref()
    {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "notification-endpoint-request-mismatch",
        ));
    }
    if evidence.zone != *fence.zone() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
            "notification-endpoint-foreign-zone",
        ));
    }
    if evidence.store != *fence.store() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StoreIncarnationMismatch,
            "notification-endpoint-store-incarnation-mismatch",
        ));
    }
    match fence.phase() {
        NotificationEndpointPhase::Revoked => {
            return Err(NotificationEndpointRefusal::relationship_revoked());
        }
        NotificationEndpointPhase::Draining => {
            return Err(NotificationEndpointRefusal::relationship_draining());
        }
        NotificationEndpointPhase::Admitted => {}
    }
    if evidence.source_generation != fence.source_generation() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "notification-endpoint-stale-source-generation",
        ));
    }
    if evidence.consumer_generation != fence.consumer_generation() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "notification-endpoint-stale-consumer-generation",
        ));
    }
    if evidence.desired_revision != fence.desired_revision() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "notification-endpoint-stale-desired-revision",
        ));
    }
    if evidence.sequence != fence.sequence() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "notification-endpoint-stale-desired-sequence",
        ));
    }
    if evidence.reconnect.get() < fence.minimum_reconnect().get() {
        return Err(NotificationEndpointRefusal::at(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
            "notification-endpoint-stale-reconnect-generation",
        ));
    }
    let key = gate
        .binding
        .request()
        .key(
            fence.zone().clone(),
            gate.source_uid.clone(),
            gate.consumer_uid.clone(),
        )
        .map_err(|_| {
            NotificationEndpointRefusal::at(
                AdmissionStage::Admit,
                RefusalReason::IdentityNotAuthorized,
                "notification-endpoint-identity-rejected",
            )
        })?;
    Ok(AdmittedNotificationEndpoint {
        binding: gate.binding.clone(),
        key,
        fence: fence.clone(),
    })
}
