use std::fmt;

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
    ResourceRef, SchemaFingerprint, ServiceName, SessionPurpose, TranscriptHash,
    TransportBinding as IdentityTransportBinding, ZoneId,
    component_session::{
        AuthorizationLease, BootstrapIdentityBinding, EndpointPolicy, HandshakeOffer, NoiseProfile,
        SessionErrorCode, TransportClass,
    },
};
use d2b_resource_api::authz::SessionVerb;

use crate::{
    ComponentSessionDriver, OwnedTransport, Result, SessionDriverHandle, SessionEngine,
    SessionError, handshake::EstablishedAuthentication,
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
    service: ServiceName,
    operation: String,
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
        let operation_valid = !operation.is_empty()
            && operation.len() <= 128
            && operation
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
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
                service.as_str() == "d2b.audit.v3" && operation == "AuditService.Export"
            }
            SessionVerb::SupportBundle => {
                service.as_str() == "d2b.support.v3" && operation == "SupportService.GenerateBundle"
            }
            _ => true,
        };
        if !operation_valid
            || !relay_fields_valid
            || !relay_target_valid
            || !diagnostic_binding_valid
        {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        Ok(Self {
            verb,
            service,
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
        &self.service
    }

    /// Borrow the exact method or named-stream operation.
    pub fn operation(&self) -> &str {
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

/// Trusted evidence mapper and native authorization hook.
///
/// The acceptor consumes this object and stores it beside the authenticated
/// subject. Neither value can be recovered or shared independently.
#[async_trait]
pub trait SessionAuthority: Send {
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

/// Single-use builder for an authenticated ComponentSession.
pub struct SessionAcceptor {
    policy: EndpointPolicy,
    expected_zone: ZoneId,
    authority: Box<dyn SessionAuthority>,
}

impl SessionAcceptor {
    /// Consume the endpoint policy and its sole authority owner.
    pub fn new(
        policy: EndpointPolicy,
        expected_zone: ZoneId,
        authority: Box<dyn SessionAuthority>,
    ) -> Result<Self> {
        HandshakeOffer::from(policy.clone())
            .validate()
            .map_err(SessionError::from)?;
        Ok(Self {
            policy,
            expected_zone,
            authority,
        })
    }

    /// Consume a completed session engine and mint one admitted session.
    pub async fn admit<T>(
        mut self,
        mut engine: SessionEngine<T>,
        evidence: TransportEvidence,
        now_tick: u64,
    ) -> Result<AdmittedComponentSession>
    where
        T: OwnedTransport + 'static,
    {
        let authentication = engine.take_authentication(&self.policy)?;
        let binding = authentication_binding(&self.policy, authentication)?;
        validate_transport_evidence(&self.policy, &binding, &evidence)?;
        validate_bootstrap_zone(&binding, &self.expected_zone)?;
        let (subject, lease) = self
            .authority
            .authenticate_connect(evidence, &binding, &self.expected_zone, now_tick)
            .await?;
        validate_subject(&subject, &self.expected_zone, &binding)?;
        if !lease.is_valid_at(now_tick) {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        Ok(AdmittedComponentSession {
            expected_zone: self.expected_zone,
            subject,
            lease,
            authority: self.authority,
            driver: engine.into_driver(),
        })
    }
}

impl fmt::Debug for SessionAcceptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionAcceptor(<redacted>)")
    }
}

/// Authenticated, policy-bound session capability owned by a router.
pub struct AdmittedComponentSession {
    expected_zone: ZoneId,
    subject: AuthenticatedSubjectContext,
    lease: AuthorizationLease,
    authority: Box<dyn SessionAuthority>,
    driver: SessionDriverHandle,
}

impl AdmittedComponentSession {
    /// Return the active authorization revision.
    pub const fn authorization_revision(&self) -> u64 {
        self.lease.policy_revision()
    }

    /// Authorize one exact operation and mint a non-cloneable permit.
    pub async fn authorize(
        &mut self,
        request: SessionAuthorizationRequest,
        now_tick: u64,
    ) -> Result<AuthorizedSessionOperation> {
        validate_zone(&self.subject, &self.expected_zone)?;
        let zone_scope_valid = if request.verb == SessionVerb::Relay {
            request.target_zone != self.expected_zone
                && request
                    .next_hop_zone
                    .as_ref()
                    .is_some_and(|next_hop| next_hop != &self.expected_zone)
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
    pub async fn receive_ttrpc(&self) -> Result<Vec<u8>> {
        self.driver.receive_ttrpc().await
    }

    /// Send one ttrpc frame under a consumed exact-operation permit.
    pub async fn send_authorized_ttrpc(
        &self,
        permit: AuthorizedSessionOperation,
        frame: Vec<u8>,
        now_tick: u64,
    ) -> Result<()> {
        if !permit.lease.is_valid_at(now_tick) || permit.request.verb != SessionVerb::Invoke {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        self.driver.send_ttrpc(frame).await
    }
}

impl fmt::Debug for AdmittedComponentSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedComponentSession")
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

/// Interface the exact-addressed bus must implement to take sole session ownership.
#[async_trait]
pub trait ComponentSessionRegistrar: Send {
    /// Register one authenticated session capability.
    async fn register_component_session(&mut self, session: AdmittedComponentSession)
    -> Result<()>;
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
