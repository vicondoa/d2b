use std::{fmt, sync::Arc};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, ControllerGeneration, EvidenceClass, Locality,
    ReconnectGeneration, ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint,
    ServiceName, SessionPurpose, TranscriptHash, TransportBinding as IdentityTransportBinding,
    ZoneId,
    component_session::{
        AuthorizationLease, BootstrapIdentityBinding, ChannelClass, EndpointPolicy, HandshakeOffer,
        HealthState, MetricLabels, MetricReason, MetricResult, NoiseProfile, OperationClass,
        RequestId, SessionErrorCode, TransportClass,
    },
};
use d2b_resource_api::authz::SessionVerb;

use crate::{
    ComponentSessionDriver, MetricEvent, MetricsSink, NoopMetrics, OwnedTransport, Result,
    SessionDriverHandle, SessionEngine, SessionError, SessionOperation,
    handshake::EstablishedAuthentication, metrics::reason_for_error,
};

/// Redacted transport evidence presented to the trusted session authority.
///
/// This is evidence input, not an authenticated identity. The authority must
/// validate it against its private registry before returning a subject.
pub struct TransportEvidence {
    class: EvidenceClass,
    binding_digest: BindingDigest,
}

impl TransportEvidence {
    /// Construct evidence from a transport adapter's verified observation.
    pub fn new(class: EvidenceClass, binding_digest: BindingDigest) -> Self {
        Self {
            class,
            binding_digest,
        }
    }

    /// Return the evidence class.
    pub const fn class(&self) -> EvidenceClass {
        self.class
    }

    /// Borrow the redacted evidence binding.
    pub fn binding_digest(&self) -> &BindingDigest {
        &self.binding_digest
    }
}

impl fmt::Debug for TransportEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransportEvidence(<redacted>)")
    }
}

/// Immutable handshake values supplied to the trusted authority.
pub struct SessionAuthenticationBinding {
    evidence_class: EvidenceClass,
    purpose: SessionPurpose,
    service: ServiceName,
    schema_fingerprint: SchemaFingerprint,
    transport_class: TransportClass,
    transport_binding: IdentityTransportBinding,
    bootstrap_identity: Option<BootstrapIdentityBinding>,
    reconnect_generation: ReconnectGeneration,
    transcript_hash: TranscriptHash,
    remote_static_key: Option<[u8; 32]>,
}

impl SessionAuthenticationBinding {
    /// Return the required authenticated evidence class.
    pub const fn evidence_class(&self) -> EvidenceClass {
        self.evidence_class
    }

    /// Borrow the endpoint purpose.
    pub fn purpose(&self) -> &SessionPurpose {
        &self.purpose
    }

    /// Borrow the exact service name.
    pub fn service(&self) -> &ServiceName {
        &self.service
    }

    /// Borrow the exact schema fingerprint.
    pub fn schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.schema_fingerprint
    }

    /// Return the exact transport class authenticated by the Noise prologue.
    pub const fn transport_class(&self) -> TransportClass {
        self.transport_class
    }

    /// Borrow the transport channel binding.
    pub fn transport_binding(&self) -> &IdentityTransportBinding {
        &self.transport_binding
    }

    /// Borrow the one-time identity consumed by an IKpsk2 handshake.
    pub fn bootstrap_identity(&self) -> Option<&BootstrapIdentityBinding> {
        self.bootstrap_identity.as_ref()
    }

    /// Return the reconnect generation.
    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    /// Borrow the Noise transcript hash.
    pub fn transcript_hash(&self) -> &TranscriptHash {
        &self.transcript_hash
    }

    /// Borrow the authenticated remote static key for enrolled or bootstrap profiles.
    pub fn remote_static_key(&self) -> Option<&[u8; 32]> {
        self.remote_static_key.as_ref()
    }
}

impl fmt::Debug for SessionAuthenticationBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionAuthenticationBinding(<redacted>)")
    }
}

/// Exact authorization attributes presented by the session to its authority.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionAuthorizationRequest {
    verb: SessionVerb,
    operation: SessionOperation,
    target_zone: ZoneId,
    target: Option<ResourceRef>,
    forwarded_target_verb: Option<SessionVerb>,
    next_hop_zone: Option<ZoneId>,
}

impl SessionAuthorizationRequest {
    /// Build an exact method or stream authorization request.
    pub fn new(
        verb: SessionVerb,
        service: ServiceName,
        operation: impl Into<String>,
        target_zone: ZoneId,
        target: Option<ResourceRef>,
    ) -> Result<Self> {
        Self::new_inner(verb, service, operation, target_zone, target, None, None)
    }

    /// Build a one-hop relay request with immutable target authorization.
    pub fn relay(
        service: ServiceName,
        operation: impl Into<String>,
        target_zone: ZoneId,
        target: Option<ResourceRef>,
        forwarded_target_verb: SessionVerb,
        next_hop_zone: ZoneId,
    ) -> Result<Self> {
        Self::new_inner(
            SessionVerb::Relay,
            service,
            operation,
            target_zone,
            target,
            Some(forwarded_target_verb),
            Some(next_hop_zone),
        )
    }

    fn new_inner(
        verb: SessionVerb,
        service: ServiceName,
        operation: impl Into<String>,
        target_zone: ZoneId,
        target: Option<ResourceRef>,
        forwarded_target_verb: Option<SessionVerb>,
        next_hop_zone: Option<ZoneId>,
    ) -> Result<Self> {
        let operation = operation.into();
        let stream = matches!(
            if verb == SessionVerb::Relay {
                forwarded_target_verb.unwrap_or(verb)
            } else {
                verb
            },
            SessionVerb::OpenStream | SessionVerb::Observe
        );
        let operation = if stream {
            SessionOperation::stream(service, operation)?
        } else {
            SessionOperation::method(service, operation)?
        };
        let relay_fields_valid = matches!(verb, SessionVerb::Relay)
            == (forwarded_target_verb.is_some() && next_hop_zone.is_some());
        let relay_target_valid = forwarded_target_verb.is_none_or(|target_verb| {
            matches!(
                target_verb,
                SessionVerb::Invoke
                    | SessionVerb::OpenStream
                    | SessionVerb::Cancel
                    | SessionVerb::Observe
            )
        });
        let diagnostic_binding_valid = match verb {
            SessionVerb::AuditExport => {
                operation.diagnostic_verb() == Some(SessionVerb::AuditExport)
            }
            SessionVerb::SupportBundle => {
                operation.diagnostic_verb() == Some(SessionVerb::SupportBundle)
            }
            _ => operation.diagnostic_verb().is_none(),
        };
        if !relay_fields_valid || !relay_target_valid || !diagnostic_binding_valid {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        Ok(Self {
            verb,
            operation,
            target_zone,
            target,
            forwarded_target_verb,
            next_hop_zone,
        })
    }

    /// Return the closed session verb.
    pub const fn verb(&self) -> SessionVerb {
        self.verb
    }

    /// Borrow the exact service.
    pub fn service(&self) -> &ServiceName {
        self.operation.service()
    }

    /// Borrow the exact method or named-stream operation.
    pub fn operation(&self) -> &str {
        self.operation.member().as_str()
    }

    /// Borrow the typed exact service operation.
    pub const fn operation_contract(&self) -> &SessionOperation {
        &self.operation
    }

    /// Borrow the immutable target Zone.
    pub fn target_zone(&self) -> &ZoneId {
        &self.target_zone
    }

    /// Borrow the optional exact resource target.
    pub fn target(&self) -> Option<&ResourceRef> {
        self.target.as_ref()
    }

    /// Return the immutable forwarded target verb for a relay.
    pub const fn forwarded_target_verb(&self) -> Option<SessionVerb> {
        self.forwarded_target_verb
    }

    /// Borrow the route-selected next hop for a relay.
    pub fn next_hop_zone(&self) -> Option<&ZoneId> {
        self.next_hop_zone.as_ref()
    }
}

impl fmt::Debug for SessionAuthorizationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionAuthorizationRequest")
            .field("verb", &self.verb)
            .field("service", &"<redacted>")
            .field("operation", &"<redacted>")
            .field("target", &"<redacted>")
            .finish()
    }
}

mod authority_seal {
    pub trait Sealed {}
}

/// Trusted evidence mapper and native authorization hook.
///
/// Implementations are confined to this crate. The acceptor consumes this
/// object and stores it beside the authenticated subject. Neither value can be
/// recovered or shared independently.
#[async_trait]
trait SessionAuthority: authority_seal::Sealed + Send {
    /// Authenticate evidence, map one subject, and authorize session connect.
    async fn authenticate_connect(
        &mut self,
        evidence: TransportEvidence,
        binding: &SessionAuthenticationBinding,
        expected_zone: &ZoneId,
        now_tick: u64,
    ) -> Result<(AuthenticatedSubjectContext, AuthorizationLease)>;

    /// Revalidate one exact method or stream under current native policy.
    async fn authorize(
        &mut self,
        subject: &AuthenticatedSubjectContext,
        request: &SessionAuthorizationRequest,
        previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> Result<AuthorizationLease>;
}

type AuthenticateSession = dyn FnOnce(
        TransportEvidence,
        &SessionAuthenticationBinding,
        &ZoneId,
        u64,
    ) -> Result<(AuthenticatedSubjectContext, AuthorizationLease)>
    + Send;
type AuthorizeSession = dyn FnMut(
        &AuthenticatedSubjectContext,
        &SessionAuthorizationRequest,
        AuthorizationLease,
        u64,
    ) -> Result<AuthorizationLease>
    + Send;

struct VerifiedAdapterAuthority {
    authenticate: Option<Box<AuthenticateSession>>,
    authorize: Box<AuthorizeSession>,
    authenticated_subject: Option<AuthenticatedSubjectContext>,
}

impl authority_seal::Sealed for VerifiedAdapterAuthority {}

#[async_trait]
impl SessionAuthority for VerifiedAdapterAuthority {
    async fn authenticate_connect(
        &mut self,
        evidence: TransportEvidence,
        binding: &SessionAuthenticationBinding,
        expected_zone: &ZoneId,
        now_tick: u64,
    ) -> Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
        let authenticate = self
            .authenticate
            .take()
            .ok_or_else(|| SessionError::new(SessionErrorCode::PolicyDenied))?;
        let (subject, lease) = authenticate(evidence, binding, expected_zone, now_tick)?;
        self.authenticated_subject = Some(subject.clone());
        Ok((subject, lease))
    }

    async fn authorize(
        &mut self,
        subject: &AuthenticatedSubjectContext,
        request: &SessionAuthorizationRequest,
        previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> Result<AuthorizationLease> {
        if self.authenticated_subject.as_ref() != Some(subject) {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        (self.authorize)(subject, request, previous_lease, now_tick)
    }
}

impl fmt::Debug for VerifiedAdapterAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedAdapterAuthority(<redacted>)")
    }
}

/// Single-use builder for an authenticated ComponentSession.
pub struct SessionAcceptor<C> {
    policy: EndpointPolicy,
    expected_zone: ZoneId,
    authority: Box<dyn SessionAuthority>,
    metrics: Arc<dyn MetricsSink>,
    registration_capability: C,
}

impl<C> SessionAcceptor<C> {
    /// Consume adapter-verified identity binding, registrar-owned policy
    /// callbacks, and the instance-bound registration capability.
    pub fn from_verified_adapter<A, Z>(
        policy: EndpointPolicy,
        expected_zone: ZoneId,
        authenticate: A,
        authorize: Z,
        registration_capability: C,
    ) -> Result<Self>
    where
        A: FnOnce(
                TransportEvidence,
                &SessionAuthenticationBinding,
                &ZoneId,
                u64,
            ) -> Result<(AuthenticatedSubjectContext, AuthorizationLease)>
            + Send
            + 'static,
        Z: FnMut(
                &AuthenticatedSubjectContext,
                &SessionAuthorizationRequest,
                AuthorizationLease,
                u64,
            ) -> Result<AuthorizationLease>
            + Send
            + 'static,
    {
        HandshakeOffer::from(policy.clone())
            .validate()
            .map_err(SessionError::from)?;
        Ok(Self {
            policy,
            expected_zone,
            authority: Box::new(VerifiedAdapterAuthority {
                authenticate: Some(Box::new(authenticate)),
                authorize: Box::new(authorize),
                authenticated_subject: None,
            }),
            metrics: Arc::new(NoopMetrics),
            registration_capability,
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<dyn MetricsSink>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Consume a completed session engine and mint one authenticated candidate.
    pub async fn admit<T>(
        mut self,
        mut engine: SessionEngine<T>,
        evidence: TransportEvidence,
        now_tick: u64,
    ) -> Result<AuthenticatedComponentSession<C>>
    where
        T: OwnedTransport + 'static,
    {
        engine.set_metrics(Arc::clone(&self.metrics));
        macro_rules! admit_try {
            ($expression:expr) => {
                match $expression {
                    Ok(value) => value,
                    Err(error) => {
                        engine.record_failure(
                            MetricEvent::ConnectAttempt,
                            ChannelClass::SessionControl,
                            OperationClass::Connect,
                            error,
                        );
                        return Err(error);
                    }
                }
            };
        }
        let authentication = admit_try!(engine.take_authentication(&self.policy));
        let binding = admit_try!(authentication_binding(&self.policy, authentication));
        admit_try!(validate_transport_evidence(
            &self.policy,
            &binding,
            &evidence
        ));
        admit_try!(validate_bootstrap_zone(&binding, &self.expected_zone));
        let (subject, lease) = self
            .authority
            .authenticate_connect(evidence, &binding, &self.expected_zone, now_tick)
            .await
            .inspect_err(|error| {
                engine.record_failure(
                    MetricEvent::ConnectAttempt,
                    ChannelClass::SessionControl,
                    OperationClass::Connect,
                    *error,
                );
            })?;
        admit_try!(validate_subject(&subject, &self.expected_zone, &binding));
        if !lease.is_valid_at(now_tick) {
            let error = SessionError::new(SessionErrorCode::PolicyDenied);
            engine.record_failure(
                MetricEvent::ConnectAttempt,
                ChannelClass::SessionControl,
                OperationClass::Connect,
                error,
            );
            return Err(error);
        }
        engine.record_metric(
            MetricEvent::ConnectAttempt,
            ChannelClass::SessionControl,
            OperationClass::Connect,
            MetricResult::Accepted,
            MetricReason::None,
        );
        let cleanup_observer = SessionCleanupObserver::new(&self.policy, Arc::clone(&self.metrics));
        Ok(AuthenticatedComponentSession {
            registration_capability: self.registration_capability,
            expected_zone: self.expected_zone,
            subject,
            lease,
            authority: self.authority,
            driver: engine.into_driver(),
            cleanup_observer,
        })
    }
}

impl<C> fmt::Debug for SessionAcceptor<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionAcceptor(<redacted>)")
    }
}

/// Authenticated session candidate that has not passed bus registration.
///
/// This value is not a routing capability. A registrar must consume it and
/// run native authorization before installing any routes.
pub struct AuthenticatedComponentSession<C> {
    registration_capability: C,
    expected_zone: ZoneId,
    subject: AuthenticatedSubjectContext,
    lease: AuthorizationLease,
    authority: Box<dyn SessionAuthority>,
    driver: SessionDriverHandle,
    cleanup_observer: SessionCleanupObserver,
}

/// Cloneable correlated ttrpc data plane separated from mutable authorization.
#[derive(Clone)]
pub struct AuthenticatedTtrpcHandle {
    driver: SessionDriverHandle,
    cleanup_observer: SessionCleanupObserver,
}

fn validate_ttrpc_permit(permit: &AuthorizedSessionOperation, now_tick: u64) -> Result<()> {
    if !permit.lease.is_valid_at(now_tick)
        || !matches!(
            permit.request.verb,
            SessionVerb::Invoke | SessionVerb::AuditExport | SessionVerb::SupportBundle
        )
    {
        return Err(SessionError::new(SessionErrorCode::PolicyDenied));
    }
    Ok(())
}

impl AuthenticatedTtrpcHandle {
    /// Mint an attempt guard that can synchronously fence an admitted write.
    pub fn attempt_guard(&self) -> crate::Cancellation {
        crate::Cancellation::new()
    }

    /// Start one request under a permit minted by the authenticated session.
    pub async fn start(
        &self,
        permit: AuthorizedSessionOperation,
        request_id: RequestId,
        frame: Vec<u8>,
        cancellation: crate::Cancellation,
        now_tick: u64,
    ) -> Result<()> {
        validate_ttrpc_permit(&permit, now_tick)?;
        self.driver
            .start_ttrpc_guarded(request_id, frame, cancellation)
            .await
    }

    /// Receive the next authenticated ttrpc frame.
    pub async fn receive(&self) -> Result<Vec<u8>> {
        self.driver.receive_ttrpc().await
    }

    /// Remove one terminal correlated request.
    pub async fn complete(&self, request_id: RequestId) -> Result<bool> {
        let result = ComponentSessionDriver::complete_ttrpc(&self.driver, request_id).await;
        if let Err(error) = result {
            self.cleanup_observer.record(OperationClass::Invoke, error);
        }
        result
    }
}

impl fmt::Debug for AuthenticatedTtrpcHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedTtrpcHandle(<redacted>)")
    }
}

#[derive(Clone)]
struct SessionCleanupObserver {
    metrics: Arc<dyn MetricsSink>,
    labels: MetricLabels,
}

impl SessionCleanupObserver {
    fn new(policy: &EndpointPolicy, metrics: Arc<dyn MetricsSink>) -> Self {
        Self {
            metrics,
            labels: MetricLabels {
                transport: policy.transport_binding.transport,
                purpose: policy.purpose,
                service: policy.service,
                channel_class: ChannelClass::TtrpcControl,
                noise: policy.noise_profile,
                locality: policy.transport_binding.locality,
                operation_class: OperationClass::Invoke,
                attachment_class: None,
                health_state: HealthState::Degraded,
                result: MetricResult::Rejected,
                reason: MetricReason::InternalInvariant,
            },
        }
    }

    fn record(&self, operation_class: OperationClass, error: SessionError) {
        let mut labels = self.labels;
        labels.operation_class = operation_class;
        labels.reason = reason_for_error(error.code());
        self.metrics.record(MetricEvent::CleanupFailure, labels, 1);
    }
}

#[async_trait]
trait SessionCancellationDriver: Send + Sync {
    fn generation(&self) -> u64;
    async fn cancel(&self, generation: u64, request_id: RequestId) -> Result<()>;
    async fn complete_ttrpc(&self, request_id: RequestId) -> Result<bool>;
}

#[async_trait]
impl<T> SessionCancellationDriver for T
where
    T: ComponentSessionDriver + Send + Sync + ?Sized,
{
    fn generation(&self) -> u64 {
        ComponentSessionDriver::generation(self)
    }

    async fn cancel(&self, generation: u64, request_id: RequestId) -> Result<()> {
        ComponentSessionDriver::cancel(self, generation, request_id).await
    }

    async fn complete_ttrpc(&self, request_id: RequestId) -> Result<bool> {
        ComponentSessionDriver::complete_ttrpc(self, request_id).await
    }
}

/// Restricted concurrent cancellation surface for one authenticated session.
#[derive(Clone)]
pub struct SessionCancellationHandle {
    driver: Arc<dyn SessionCancellationDriver>,
    cleanup_observer: SessionCleanupObserver,
    writer_fence: crate::Cancellation,
}

impl SessionCancellationHandle {
    /// Revoke every queued or future batch for this session generation and
    /// wait for batches already admitted by the writer.
    pub fn revoke_generation_writes(&self) -> impl Future<Output = ()> + Send + 'static {
        let fence = self.writer_fence.cancel_and_wait();
        async move {
            fence.await;
        }
    }

    /// Signal cancellation for one exact request in the current generation.
    pub async fn cancel(&self, request_id: RequestId) -> Result<()> {
        let delivery = self
            .driver
            .cancel(self.driver.generation(), request_id.clone())
            .await;
        let completion = self.driver.complete_ttrpc(request_id).await;
        if let Err(error) = completion {
            self.cleanup_observer.record(OperationClass::Cancel, error);
        }
        delivery?;
        match completion {
            Err(error) if error.code() != SessionErrorCode::SessionDisconnected => Err(error),
            _ => Ok(()),
        }
    }
}

impl fmt::Debug for SessionCancellationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionCancellationHandle(<redacted>)")
    }
}

/// Redacted routing metadata derived only from an authenticated candidate.
///
/// This value carries no driver, authority, lease, transport binding, or
/// transcript and cannot be converted back into an admitted session.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedSessionRouteBinding {
    context: AuthenticatedSubjectContext,
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

impl AuthenticatedSessionRouteBinding {
    /// Borrow the authenticated context for registrar-owned authorization.
    pub fn context(&self) -> &AuthenticatedSubjectContext {
        &self.context
    }

    pub fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub fn subject_ref(&self) -> &ResourceRef {
        &self.subject_ref
    }

    pub fn subject_uid(&self) -> &ResourceUid {
        &self.subject_uid
    }

    pub const fn evidence_class(&self) -> EvidenceClass {
        self.evidence_class
    }

    pub const fn locality(&self) -> Locality {
        self.locality
    }

    pub fn service(&self) -> &ServiceName {
        &self.service
    }

    pub fn schema(&self) -> &SchemaFingerprint {
        &self.schema
    }

    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    pub fn provider_ref(&self) -> Option<&ResourceRef> {
        self.provider_ref.as_ref()
    }

    pub const fn provider_generation(&self) -> Option<ResourceGeneration> {
        self.provider_generation
    }

    pub const fn controller_generation(&self) -> Option<ControllerGeneration> {
        self.controller_generation
    }
}

impl fmt::Debug for AuthenticatedSessionRouteBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedSessionRouteBinding(<redacted>)")
    }
}

/// A registration capability consumes itself against one concrete registrar.
///
/// The capability implementation owns validation. The session never exposes
/// the value to a caller or to a caller-supplied closure.
pub trait SessionRegistrationCapability<R> {
    type Error;

    fn consume(self, registrar: &R) -> std::result::Result<(), Self::Error>;
}

impl<C> AuthenticatedComponentSession<C> {
    fn ttrpc_handle(&self) -> AuthenticatedTtrpcHandle {
        AuthenticatedTtrpcHandle {
            driver: self.driver.clone(),
            cleanup_observer: self.cleanup_observer.clone(),
        }
    }

    /// Consume the instance-bound capability and split out the correlated data
    /// plane for the registrar-owned response dispatcher.
    pub fn consume_registration<R>(
        self,
        registrar: &R,
    ) -> std::result::Result<(AuthenticatedComponentSession<()>, AuthenticatedTtrpcHandle), C::Error>
    where
        C: SessionRegistrationCapability<R>,
    {
        let Self {
            registration_capability,
            expected_zone,
            subject,
            lease,
            authority,
            driver,
            cleanup_observer,
        } = self;
        registration_capability.consume(registrar)?;
        let ttrpc = AuthenticatedTtrpcHandle {
            driver: driver.clone(),
            cleanup_observer: cleanup_observer.clone(),
        };
        Ok((
            AuthenticatedComponentSession {
                registration_capability: (),
                expected_zone,
                subject,
                lease,
                authority,
                driver,
                cleanup_observer,
            },
            ttrpc,
        ))
    }

    /// Clone a cancellation-only handle that carries no claims or send access.
    pub fn cancellation_handle(&self) -> SessionCancellationHandle {
        SessionCancellationHandle {
            driver: Arc::new(self.driver.clone()),
            cleanup_observer: self.cleanup_observer.clone(),
            writer_fence: self.driver.writer_fence(),
        }
    }

    /// Return the active authorization revision.
    pub const fn authorization_revision(&self) -> u64 {
        self.lease.policy_revision()
    }

    /// Snapshot non-authority routing metadata without exposing session claims.
    pub fn route_binding(&self) -> AuthenticatedSessionRouteBinding {
        AuthenticatedSessionRouteBinding {
            context: self.subject.clone(),
            zone: self.expected_zone.clone(),
            subject_ref: self.subject.subject_ref().clone(),
            subject_uid: self.subject.subject_uid().clone(),
            evidence_class: self.subject.evidence_class(),
            locality: self.subject.transport_binding().locality(),
            service: self.subject.service().clone(),
            schema: self.subject.schema_fingerprint().clone(),
            reconnect_generation: self.subject.reconnect_generation(),
            provider_ref: self.subject.provider_ref().cloned(),
            provider_generation: self.subject.provider_generation(),
            controller_generation: self.subject.controller_generation(),
        }
    }

    /// Authorize one exact operation and mint a non-cloneable permit.
    pub async fn authorize(
        &mut self,
        request: SessionAuthorizationRequest,
        now_tick: u64,
    ) -> Result<AuthorizedSessionOperation> {
        validate_zone(&self.subject, &self.expected_zone)?;
        let zone_scope_valid = if request.verb == SessionVerb::Relay {
            let next_hop = request.next_hop_zone.as_ref();
            let forwarded = self.subject.transport_binding().locality() == Locality::AdjacentZone
                && request.target_zone == self.expected_zone
                && next_hop == Some(&self.expected_zone);
            let outbound = request.target_zone != self.expected_zone
                && next_hop.is_some_and(|next_hop| next_hop != &self.expected_zone);
            forwarded || outbound
        } else {
            request.target_zone == self.expected_zone
        };
        if !zone_scope_valid || !self.lease.is_valid_at(now_tick) {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }

        let lease = self
            .authority
            .authorize(&self.subject, &request, self.lease, now_tick)
            .await?;
        validate_zone(&self.subject, &self.expected_zone)?;
        if !lease.is_valid_at(now_tick) {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        self.lease = lease;
        Ok(AuthorizedSessionOperation { request, lease })
    }

    /// Receive one authenticated ttrpc frame for authorization and dispatch.
    pub async fn receive_ttrpc(&mut self) -> Result<Vec<u8>> {
        self.driver.receive_ttrpc().await
    }

    /// Send one ttrpc frame under a consumed exact-operation permit.
    pub async fn send_authorized_ttrpc(
        &mut self,
        permit: AuthorizedSessionOperation,
        frame: Vec<u8>,
        now_tick: u64,
    ) -> Result<()> {
        if !permit.lease.is_valid_at(now_tick)
            || !matches!(
                permit.request.verb,
                SessionVerb::Invoke | SessionVerb::AuditExport | SessionVerb::SupportBundle
            )
        {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        self.driver.send_ttrpc(frame).await
    }

    /// Start one correlated ttrpc request under a consumed operation permit.
    pub async fn start_authorized_ttrpc(
        &mut self,
        permit: AuthorizedSessionOperation,
        request_id: RequestId,
        frame: Vec<u8>,
        now_tick: u64,
    ) -> Result<()> {
        let handle = self.ttrpc_handle();
        let cancellation = handle.attempt_guard();
        handle
            .start(permit, request_id, frame, cancellation, now_tick)
            .await
    }

    /// Remove one terminal correlated request.
    pub async fn complete_ttrpc(&mut self, request_id: RequestId) -> Result<bool> {
        let result = ComponentSessionDriver::complete_ttrpc(&self.driver, request_id).await;
        if let Err(error) = result {
            self.cleanup_observer.record(OperationClass::Invoke, error);
        }
        result
    }
}

impl<C> fmt::Debug for AuthenticatedComponentSession<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedComponentSession")
            .field("subject", &"<redacted>")
            .field("authorization", &"<redacted>")
            .field("driver", &"<redacted>")
            .finish()
    }
}

/// Non-cloneable proof that one exact operation passed session policy.
pub struct AuthorizedSessionOperation {
    request: SessionAuthorizationRequest,
    lease: AuthorizationLease,
}

impl AuthorizedSessionOperation {
    /// Borrow the exact authorized request.
    pub fn request(&self) -> &SessionAuthorizationRequest {
        &self.request
    }

    /// Return the policy revision that minted this permit.
    pub const fn policy_revision(&self) -> u64 {
        self.lease.policy_revision()
    }

    /// Return the monotonic expiry captured when this work was admitted.
    pub const fn expires_at_tick(&self) -> u64 {
        self.lease.expires_at_tick()
    }
}

impl fmt::Debug for AuthorizedSessionOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedSessionOperation")
            .field("request", &"<redacted>")
            .field("lease", &"<redacted>")
            .finish()
    }
}

fn authentication_binding(
    policy: &EndpointPolicy,
    authentication: EstablishedAuthentication,
) -> Result<SessionAuthenticationBinding> {
    if authentication.generation != policy.reconnect_generation {
        return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
    }
    let schema_fingerprint =
        SchemaFingerprint::parse(format!("sha256:{}", hex(&policy.schema_fingerprint)))
            .map_err(|_| SessionError::new(SessionErrorCode::SchemaMismatch))?;
    let binding_digest = binding_digest(policy.transport_binding.channel_binding)?;
    let locality = match policy.transport_binding.locality {
        crate::contract::Locality::ProcessLocal
        | crate::contract::Locality::HostLocal
        | crate::contract::Locality::GuestLocal => Locality::Local,
        crate::contract::Locality::Remote => Locality::Remote,
    };
    Ok(SessionAuthenticationBinding {
        evidence_class: evidence_class(policy.noise_profile),
        purpose: SessionPurpose::parse(policy.purpose.as_str())
            .map_err(|_| SessionError::new(SessionErrorCode::PurposeMismatch))?,
        service: ServiceName::parse(policy.service.as_str())
            .map_err(|_| SessionError::new(SessionErrorCode::ServiceMismatch))?,
        schema_fingerprint,
        transport_class: policy.transport_binding.transport,
        transport_binding: IdentityTransportBinding::new(locality, binding_digest),
        bootstrap_identity: authentication.bootstrap_identity,
        reconnect_generation: ReconnectGeneration::new(authentication.generation)
            .map_err(|_| SessionError::new(SessionErrorCode::GenerationMismatch))?,
        transcript_hash: TranscriptHash::from_bytes(authentication.transcript_hash),
        remote_static_key: authentication.remote_static_key,
    })
}

fn validate_transport_evidence(
    policy: &EndpointPolicy,
    binding: &SessionAuthenticationBinding,
    evidence: &TransportEvidence,
) -> Result<()> {
    if evidence.class != binding.evidence_class {
        return Err(SessionError::new(
            SessionErrorCode::IdentityEvidenceMismatch,
        ));
    }
    let remote_static_expected = binding.evidence_class != EvidenceClass::UnixPeer;
    if remote_static_expected != binding.remote_static_key.is_some() {
        return Err(SessionError::new(
            SessionErrorCode::IdentityEvidenceMismatch,
        ));
    }
    if &evidence.binding_digest != binding.transport_binding.binding_digest() {
        return Err(SessionError::new(SessionErrorCode::ChannelBindingMismatch));
    }
    let transport_valid = match policy.noise_profile {
        NoiseProfile::Nn25519ChaChaPolySha256 => matches!(
            policy.transport_binding.transport,
            TransportClass::UnixStream
                | TransportClass::UnixSeqpacket
                | TransportClass::InheritedSocketpair
        ),
        NoiseProfile::Kk25519ChaChaPolySha256 => true,
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => true,
    };
    if !transport_valid {
        return Err(SessionError::new(SessionErrorCode::TransportMismatch));
    }
    Ok(())
}

fn validate_subject(
    subject: &AuthenticatedSubjectContext,
    expected_zone: &ZoneId,
    binding: &SessionAuthenticationBinding,
) -> Result<()> {
    validate_zone(subject, expected_zone)?;
    if subject.evidence_class() != binding.evidence_class
        || subject.session_purpose() != &binding.purpose
        || subject.service() != &binding.service
        || subject.schema_fingerprint() != &binding.schema_fingerprint
        || subject.transport_binding() != &binding.transport_binding
        || subject.reconnect_generation() != binding.reconnect_generation
        || subject.transcript_hash() != &binding.transcript_hash
    {
        return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
    }
    if let Some(expected) = &binding.bootstrap_identity
        && (subject.subject_ref() != &expected.subject_ref
            || subject.subject_uid() != &expected.subject_uid
            || subject.zone_ref().name().as_str() != expected.zone.as_str()
            || subject.session_purpose() != &expected.purpose)
    {
        return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
    }

    Ok(())
}

fn validate_bootstrap_zone(
    binding: &SessionAuthenticationBinding,
    expected_zone: &ZoneId,
) -> Result<()> {
    let bootstrap_expected = binding.evidence_class == EvidenceClass::BootstrapIkpsk2;
    if bootstrap_expected != binding.bootstrap_identity.is_some()
        || binding
            .bootstrap_identity
            .as_ref()
            .is_some_and(|identity| &identity.zone != expected_zone)
    {
        return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
    }
    Ok(())
}

const fn evidence_class(profile: NoiseProfile) -> EvidenceClass {
    match profile {
        NoiseProfile::Nn25519ChaChaPolySha256 => EvidenceClass::UnixPeer,
        NoiseProfile::Kk25519ChaChaPolySha256 => EvidenceClass::EnrolledKk,
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => EvidenceClass::BootstrapIkpsk2,
    }
}

fn validate_zone(subject: &AuthenticatedSubjectContext, expected_zone: &ZoneId) -> Result<()> {
    if subject.zone_ref().resource_type().as_str() != "Zone"
        || subject.zone_ref().name().as_str() != expected_zone.as_str()
    {
        return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
    }
    Ok(())
}

fn binding_digest(bytes: [u8; 32]) -> Result<BindingDigest> {
    BindingDigest::parse(format!("sha256:{}", hex(&bytes)))
        .map_err(|_| SessionError::new(SessionErrorCode::ChannelBindingMismatch))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use d2b_contracts::v3::component_session::{EndpointPurpose, Locality, ServicePackage};

    use super::*;

    #[derive(Default)]
    struct CapturingMetrics(Mutex<Vec<(MetricEvent, MetricLabels, u64)>>);

    impl MetricsSink for CapturingMetrics {
        fn record(&self, event: MetricEvent, labels: MetricLabels, value: u64) {
            self.0.lock().unwrap().push((event, labels, value));
        }
    }

    struct MockCancellationDriver {
        cancel_error: Option<SessionError>,
        complete_error: Option<SessionError>,
        complete_calls: AtomicUsize,
    }

    #[async_trait]
    impl SessionCancellationDriver for MockCancellationDriver {
        fn generation(&self) -> u64 {
            7
        }

        async fn cancel(&self, _generation: u64, _request_id: RequestId) -> Result<()> {
            self.cancel_error.map_or(Ok(()), Err)
        }

        async fn complete_ttrpc(&self, _request_id: RequestId) -> Result<bool> {
            self.complete_calls.fetch_add(1, Ordering::AcqRel);
            self.complete_error.map_or(Ok(true), Err)
        }
    }

    fn cleanup_observer(metrics: Arc<dyn MetricsSink>) -> SessionCleanupObserver {
        SessionCleanupObserver {
            metrics,
            labels: MetricLabels {
                transport: TransportClass::UnixSeqpacket,
                purpose: EndpointPurpose::LocalLifecycle,
                service: ServicePackage::ResourceV3,
                channel_class: ChannelClass::TtrpcControl,
                noise: NoiseProfile::Nn25519ChaChaPolySha256,
                locality: Locality::HostLocal,
                operation_class: OperationClass::Cancel,
                attachment_class: None,
                health_state: HealthState::Degraded,
                result: MetricResult::Rejected,
                reason: MetricReason::InternalInvariant,
            },
        }
    }

    fn request_id() -> RequestId {
        RequestId::new(vec![9; 16]).unwrap()
    }

    #[tokio::test]
    async fn cancellation_succeeds_when_terminal_cleanup_is_disconnected() {
        let metrics = Arc::new(CapturingMetrics::default());
        let driver = Arc::new(MockCancellationDriver {
            cancel_error: None,
            complete_error: Some(SessionError::new(SessionErrorCode::SessionDisconnected)),
            complete_calls: AtomicUsize::new(0),
        });
        let handle = SessionCancellationHandle {
            driver: driver.clone(),
            cleanup_observer: cleanup_observer(metrics.clone()),
            writer_fence: crate::Cancellation::new(),
        };

        handle.cancel(request_id()).await.unwrap();
        assert_eq!(driver.complete_calls.load(Ordering::Acquire), 1);
        let events = metrics.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, MetricEvent::CleanupFailure);
        assert_eq!(events[0].1.operation_class, OperationClass::Cancel);
        assert_eq!(events[0].1.reason, MetricReason::Transport);
    }

    #[tokio::test]
    async fn cancellation_propagates_delivery_failure_after_local_cleanup() {
        let metrics = Arc::new(CapturingMetrics::default());
        let driver = Arc::new(MockCancellationDriver {
            cancel_error: Some(SessionError::new(SessionErrorCode::SessionDisconnected)),
            complete_error: None,
            complete_calls: AtomicUsize::new(0),
        });
        let handle = SessionCancellationHandle {
            driver: driver.clone(),
            cleanup_observer: cleanup_observer(metrics.clone()),
            writer_fence: crate::Cancellation::new(),
        };

        let error = handle.cancel(request_id()).await.unwrap_err();
        assert_eq!(error.code(), SessionErrorCode::SessionDisconnected);
        assert_eq!(driver.complete_calls.load(Ordering::Acquire), 1);
        assert!(metrics.0.lock().unwrap().is_empty());
    }

    struct SaturatedCancellationDriver {
        registry: Mutex<crate::RequestRegistry>,
    }

    #[async_trait]
    impl SessionCancellationDriver for SaturatedCancellationDriver {
        fn generation(&self) -> u64 {
            7
        }

        async fn cancel(&self, _generation: u64, _request_id: RequestId) -> Result<()> {
            Err(SessionError::new(SessionErrorCode::QueueBackpressure))
        }

        async fn complete_ttrpc(&self, request_id: RequestId) -> Result<bool> {
            Ok(self.registry.lock().unwrap().complete(&request_id))
        }
    }

    #[tokio::test]
    async fn cancellation_backpressure_releases_capacity_for_reuse() {
        let active = request_id();
        let mut registry = crate::RequestRegistry::with_limit(7, 1).unwrap();
        registry.register(active.clone()).unwrap();
        let driver = Arc::new(SaturatedCancellationDriver {
            registry: Mutex::new(registry),
        });
        let handle = SessionCancellationHandle {
            driver: driver.clone(),
            cleanup_observer: cleanup_observer(Arc::new(CapturingMetrics::default())),
            writer_fence: crate::Cancellation::new(),
        };

        let error = handle.cancel(active).await.unwrap_err();
        assert_eq!(error.code(), SessionErrorCode::QueueBackpressure);
        driver
            .registry
            .lock()
            .unwrap()
            .register(RequestId::new(vec![8; 16]).unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn live_cleanup_failure_is_recorded_and_propagated() {
        let metrics = Arc::new(CapturingMetrics::default());
        let driver = Arc::new(MockCancellationDriver {
            cancel_error: None,
            complete_error: Some(SessionError::new(SessionErrorCode::InternalInvariant)),
            complete_calls: AtomicUsize::new(0),
        });
        let handle = SessionCancellationHandle {
            driver,
            cleanup_observer: cleanup_observer(metrics.clone()),
            writer_fence: crate::Cancellation::new(),
        };

        let error = handle.cancel(request_id()).await.unwrap_err();
        assert_eq!(error.code(), SessionErrorCode::InternalInvariant);
        let events = metrics.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, MetricEvent::CleanupFailure);
        assert_eq!(events[0].1.reason, MetricReason::InternalInvariant);
    }
}
