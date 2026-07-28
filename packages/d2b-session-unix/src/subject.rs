use std::fmt;

use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ControllerGeneration, EvidenceClass, Locality, ResourceGeneration,
    ResourceRef, ResourceUid, SessionBinding, ZoneId,
};
use d2b_session::{
    SessionAuthenticationBinding, SessionError, TransportEvidence,
    contract::{SessionErrorCode, TransportClass},
};

use crate::{PeerCredentials, SeqpacketSocket, StreamSocket, UnixSessionError};

enum UnixSubjectKind {
    Host,
    Guest,
    Provider,
}

/// Config-generated Host or Guest subject consumed by one Unix session owner.
pub struct UnixSubjectIdentity {
    kind: UnixSubjectKind,
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    zone_ref: ResourceRef,
    expected_peer: PeerCredentials,
    provider_ref: Option<ResourceRef>,
    provider_generation: Option<ResourceGeneration>,
    controller_generation: Option<ControllerGeneration>,
}

impl UnixSubjectIdentity {
    /// Construct a Host subject mapping.
    pub fn host(
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: ResourceRef,
        expected_peer: PeerCredentials,
    ) -> d2b_session::Result<Self> {
        Self::new(
            UnixSubjectKind::Host,
            subject_ref,
            subject_uid,
            zone_ref,
            expected_peer,
        )
    }

    /// Construct a Guest subject mapping.
    pub fn guest(
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: ResourceRef,
        expected_peer: PeerCredentials,
    ) -> d2b_session::Result<Self> {
        Self::new(
            UnixSubjectKind::Guest,
            subject_ref,
            subject_uid,
            zone_ref,
            expected_peer,
        )
    }

    fn new(
        kind: UnixSubjectKind,
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: ResourceRef,
        expected_peer: PeerCredentials,
    ) -> d2b_session::Result<Self> {
        let expected_type = match kind {
            UnixSubjectKind::Host => "Host",
            UnixSubjectKind::Guest => "Guest",
            UnixSubjectKind::Provider => "Provider",
        };
        if subject_ref.resource_type().as_str() != expected_type
            || zone_ref.resource_type().as_str() != "Zone"
        {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        Ok(Self {
            kind,
            subject_ref,
            subject_uid,
            zone_ref,
            expected_peer,
            provider_ref: None,
            provider_generation: None,
            controller_generation: None,
        })
    }

    /// Construct a Provider subject mapping.
    pub fn provider(
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: ResourceRef,
        expected_peer: PeerCredentials,
        provider_generation: ResourceGeneration,
    ) -> d2b_session::Result<Self> {
        let mut identity = Self::new(
            UnixSubjectKind::Provider,
            subject_ref,
            subject_uid,
            zone_ref,
            expected_peer,
        )?;
        identity.provider_ref = Some(identity.subject_ref.clone());
        identity.provider_generation = Some(provider_generation);
        Ok(identity)
    }

    /// Bind a Provider selected by trusted adapter configuration.
    pub fn with_provider(
        mut self,
        provider_ref: ResourceRef,
        generation: ResourceGeneration,
    ) -> d2b_session::Result<Self> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        self.provider_ref = Some(provider_ref);
        self.provider_generation = Some(generation);
        Ok(self)
    }

    /// Bind a controller generation supplied by trusted adapter configuration.
    pub fn with_controller_generation(mut self, generation: ControllerGeneration) -> Self {
        self.controller_generation = Some(generation);
        self
    }

    /// Consume this identity while verifying one seqpacket endpoint.
    pub fn verify_seqpacket(
        self,
        socket: &SeqpacketSocket,
    ) -> Result<VerifiedUnixSubject, UnixSessionError> {
        self.verify(
            socket.acceptor_peer_credentials()?,
            TransportClass::UnixSeqpacket,
        )
    }

    /// Consume this identity while verifying one stream endpoint.
    pub fn verify_stream(
        self,
        socket: &StreamSocket,
    ) -> Result<VerifiedUnixSubject, UnixSessionError> {
        self.verify(
            socket.acceptor_peer_credentials()?,
            TransportClass::UnixStream,
        )
    }

    fn verify(
        self,
        observed_peer: PeerCredentials,
        transport_class: TransportClass,
    ) -> Result<VerifiedUnixSubject, UnixSessionError> {
        if observed_peer != self.expected_peer {
            return Err(UnixSessionError::CredentialMismatch);
        }
        Ok(VerifiedUnixSubject {
            identity: self,
            observed_peer,
            transport_class,
        })
    }
}

impl fmt::Debug for UnixSubjectIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UnixSubjectIdentity(<redacted>)")
    }
}

/// Single-owner Unix peer proof and config-generated subject mapping.
pub struct VerifiedUnixSubject {
    identity: UnixSubjectIdentity,
    observed_peer: PeerCredentials,
    transport_class: TransportClass,
}

impl VerifiedUnixSubject {
    /// Borrow the config-generated subject reference retained by this proof.
    pub fn subject_ref(&self) -> &ResourceRef {
        &self.identity.subject_ref
    }

    /// Verify that the proof is consumed only by its originating transport.
    pub fn validate_transport(&self, transport_class: TransportClass) -> d2b_session::Result<()> {
        if self.transport_class != transport_class {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        Ok(())
    }

    /// Consume verified peer evidence and mint one immutable subject context.
    pub fn bind(
        self,
        evidence: &TransportEvidence,
        binding: &SessionAuthenticationBinding,
        expected_zone: &ZoneId,
    ) -> d2b_session::Result<AuthenticatedSubjectContext> {
        let _ = self.observed_peer;
        let expected_type = match self.identity.kind {
            UnixSubjectKind::Host => "Host",
            UnixSubjectKind::Guest => "Guest",
            UnixSubjectKind::Provider => "Provider",
        };
        self.validate_transport(binding.transport_class())?;
        if evidence.class() != EvidenceClass::UnixPeer
            || binding.evidence_class() != EvidenceClass::UnixPeer
            || binding.transport_binding().locality() != Locality::Local
            || self.identity.subject_ref.resource_type().as_str() != expected_type
            || self.identity.zone_ref.name().as_str() != expected_zone.as_str()
            || evidence.binding_digest() != binding.transport_binding().binding_digest()
        {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        let subject_ref = self.identity.subject_ref;
        let mut context = AuthenticatedSubjectContext::new(
            subject_ref.clone(),
            self.identity.subject_uid,
            self.identity.zone_ref,
            EvidenceClass::UnixPeer,
            binding.purpose().clone(),
            binding.service().clone(),
            SessionBinding::new(
                binding.schema_fingerprint().clone(),
                binding.transport_binding().clone(),
                binding.reconnect_generation(),
                binding.transcript_hash().clone(),
            ),
        );
        if let (Some(provider_ref), Some(provider_generation)) = (
            self.identity.provider_ref,
            self.identity.provider_generation,
        ) {
            context = context
                .with_provider_ref(provider_ref)
                .with_provider_generation(provider_generation);
        }
        if let Some(controller_generation) = self.identity.controller_generation {
            context = context.with_controller_generation(controller_generation);
        }
        Ok(context)
    }
}

impl fmt::Debug for VerifiedUnixSubject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedUnixSubject(<redacted>)")
    }
}
