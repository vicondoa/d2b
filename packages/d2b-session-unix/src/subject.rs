use std::fmt;

use d2b_session::{
    SessionError,
    contract::{SessionErrorCode, TransportClass},
};

use crate::{PeerCredentials, SeqpacketSocket, StreamSocket, UnixSessionError};

/// Kernel-verified Unix peer evidence without caller-authored subject claims.
pub struct VerifiedUnixPeer {
    observed_peer: PeerCredentials,
    transport_class: TransportClass,
}

impl VerifiedUnixPeer {
    /// Read peer credentials from one seqpacket endpoint.
    pub fn verify_seqpacket(socket: &SeqpacketSocket) -> Result<Self, UnixSessionError> {
        Ok(Self {
            observed_peer: socket.acceptor_peer_credentials()?,
            transport_class: TransportClass::UnixSeqpacket,
        })
    }

    /// Read peer credentials from one stream endpoint.
    pub fn verify_stream(socket: &StreamSocket) -> Result<Self, UnixSessionError> {
        Ok(Self {
            observed_peer: socket.acceptor_peer_credentials()?,
            transport_class: TransportClass::UnixStream,
        })
    }

    /// Return the kernel-observed peer credentials.
    pub const fn credentials(&self) -> PeerCredentials {
        self.observed_peer
    }

    /// Verify that the evidence is consumed only by its originating transport.
    pub fn validate_transport(&self, transport_class: TransportClass) -> d2b_session::Result<()> {
        if self.transport_class != transport_class {
            return Err(SessionError::new(SessionErrorCode::SubjectMismatch));
        }
        Ok(())
    }
}

impl fmt::Debug for VerifiedUnixPeer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedUnixPeer(<redacted>)")
    }
}
