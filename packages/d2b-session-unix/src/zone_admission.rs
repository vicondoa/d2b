//! Bootstrap Zone admission over verified Unix peer evidence.
//!
//! This is deliberately a closed bootstrap seam.  It accepts only the two
//! compiled Provider subjects and compares the expected service-manager UID
//! with the UID read from `SO_PEERCRED`.  No caller-supplied subject,
//! principal, role, or Zone name is accepted as an authority claim.

use std::fmt;

use d2b_contracts_resource::v3::{ResourceRef, ZoneId};

use crate::VerifiedUnixPeer;

/// The two compiled bootstrap Provider identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapProvider {
    /// The fixed core controller Provider.
    SystemCore,
    /// The fixed minijail Provider.
    SystemMinijail,
}

impl BootstrapProvider {
    /// The exact local Provider resource name.
    pub const fn resource_name(self) -> &'static str {
        match self {
            Self::SystemCore => "system-core",
            Self::SystemMinijail => "system-minijail",
        }
    }

    /// The exact local Provider resource reference.
    pub fn resource_ref(self) -> ResourceRef {
        ResourceRef::parse(&format!("Provider/{}", self.resource_name()))
            .expect("compiled bootstrap Provider ref is valid")
    }
}

/// Typed fail-closed Zone admission errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneAdmissionError {
    PeerUidMismatch,
    InvalidPeerUid,
}

impl fmt::Display for ZoneAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PeerUidMismatch => "zone-bootstrap-peer-uid-mismatch",
            Self::InvalidPeerUid => "zone-bootstrap-peer-uid-invalid",
        })
    }
}

impl std::error::Error for ZoneAdmissionError {}

/// Authenticated bootstrap identity after peer evidence is consumed.
///
/// This value contains no socket, descriptor, path, or mutable policy.  It is
/// a routing identity only; the resource API still performs the exact
/// bootstrap method/type authorization check.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneBootstrapIdentity {
    zone: ZoneId,
    provider: BootstrapProvider,
    peer_uid: u32,
}

impl ZoneBootstrapIdentity {
    /// Verify one kernel-observed peer against a fixed bootstrap Provider.
    pub fn verify(
        peer: VerifiedUnixPeer,
        expected_uid: u32,
        zone: ZoneId,
        provider: BootstrapProvider,
    ) -> Result<Self, ZoneAdmissionError> {
        if expected_uid == 0 {
            tracing::warn!(
                provider = provider.resource_name(),
                expected_uid = expected_uid,
                "zone bootstrap admission refused: expected peer UID is invalid"
            );
            return Err(ZoneAdmissionError::InvalidPeerUid);
        }
        let observed_uid = peer.credentials().uid().as_raw();
        if observed_uid != expected_uid {
            tracing::warn!(
                provider = provider.resource_name(),
                expected_uid = expected_uid,
                observed_uid = observed_uid,
                "zone bootstrap admission refused: peer UID mismatch"
            );
            return Err(ZoneAdmissionError::PeerUidMismatch);
        }
        Ok(Self {
            zone,
            provider,
            peer_uid: observed_uid,
        })
    }

    /// Borrow the local Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the compiled Provider subject class.
    pub const fn provider(&self) -> BootstrapProvider {
        self.provider
    }

    /// Return the observed UID for the next trusted admission layer.
    pub const fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    /// Return the fixed Provider resource reference.
    pub fn subject_ref(&self) -> ResourceRef {
        self.provider.resource_ref()
    }
}

impl fmt::Debug for ZoneBootstrapIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZoneBootstrapIdentity(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};
    use rustix::process::getuid;

    /// One kernel-verified peer carrying the current process credentials.
    fn verified_peer() -> VerifiedUnixPeer {
        let (left, _right) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let socket = crate::SeqpacketSocket::from_owned(left).unwrap();
        VerifiedUnixPeer::verify_seqpacket(&socket).unwrap()
    }

    fn zone() -> ZoneId {
        ZoneId::parse("work").unwrap()
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn verify_rejects_zero_expected_uid() {
        // A zero expected UID is never a valid bootstrap admission target:
        // the fixed service-manager UID must be a real non-root account.
        assert_eq!(
            ZoneBootstrapIdentity::verify(
                verified_peer(),
                0,
                zone(),
                BootstrapProvider::SystemCore,
            ),
            Err(ZoneAdmissionError::InvalidPeerUid)
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn verify_rejects_mismatched_peer_uid() {
        // The kernel-observed peer UID must equal the fixed expected UID;
        // any other peer is refused even though the credentials are real.
        let current_uid = getuid().as_raw();
        let wrong_uid = current_uid.checked_add(1).unwrap_or(current_uid - 1);
        assert_ne!(wrong_uid, 0);
        assert_ne!(wrong_uid, current_uid);
        assert_eq!(
            ZoneBootstrapIdentity::verify(
                verified_peer(),
                wrong_uid,
                zone(),
                BootstrapProvider::SystemMinijail,
            ),
            Err(ZoneAdmissionError::PeerUidMismatch)
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn verify_accepts_matching_peer_uid() {
        // The happy path is only exercisable when the test process is not
        // root: a zero expected UID is itself refused.
        let current_uid = getuid().as_raw();
        if current_uid == 0 {
            return;
        }
        let identity = ZoneBootstrapIdentity::verify(
            verified_peer(),
            current_uid,
            zone(),
            BootstrapProvider::SystemCore,
        )
        .expect("matching peer UID admits");
        assert_eq!(identity.peer_uid(), current_uid);
        assert_eq!(identity.zone(), &zone());
        assert_eq!(identity.provider(), BootstrapProvider::SystemCore);
        assert_eq!(
            identity.subject_ref(),
            BootstrapProvider::SystemCore.resource_ref()
        );
    }
}
