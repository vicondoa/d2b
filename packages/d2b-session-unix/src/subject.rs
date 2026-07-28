use std::fmt;

use d2b_contracts::v3::{
    AuthenticatedSubjectContext, EvidenceClass, Locality, ResourceRef, ResourceUid, SessionBinding,
    ZoneId,
};
use d2b_session::{
    SessionAuthenticationBinding, SessionError, TransportEvidence,
    contract::{SessionErrorCode, TransportClass},
};

use crate::{PeerCredentials, SeqpacketSocket, StreamSocket, UnixSessionError};

enum UnixSubjectKind {
    Host,
    Guest,
}

/// Config-generated Host or Guest subject consumed by one Unix session owner.
pub struct UnixSubjectIdentity {
    kind: UnixSubjectKind,
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    zone_ref: ResourceRef,
    expected_peer: PeerCredentials,
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
        })
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
        };
        if evidence.class() != EvidenceClass::UnixPeer
            || binding.evidence_class() != EvidenceClass::UnixPeer
            || binding.transport_binding().locality() != Locality::Local
            || binding.transport_class() != self.transport_class
            || self.identity.subject_ref.resource_type().as_str() != expected_type
            || self.identity.zone_ref.name().as_str() != expected_zone.as_str()
            || evidence.binding_digest() != binding.transport_binding().binding_digest()
        {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        Ok(AuthenticatedSubjectContext::new(
            self.identity.subject_ref,
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
        ))
    }
}

impl fmt::Debug for VerifiedUnixSubject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedUnixSubject(<redacted>)")
    }
}
