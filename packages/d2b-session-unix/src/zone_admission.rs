//! Bootstrap Zone admission over verified Unix peer evidence.
//!
//! This is deliberately a closed bootstrap seam.  It accepts only the two
//! compiled Provider subjects and compares the expected service-manager UID
//! with the UID read from `SO_PEERCRED`.  No caller-supplied subject,
//! principal, role, or Zone name is accepted as an authority claim.

use std::fmt;

use d2b_contracts::v3::{ResourceRef, ZoneId};

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
    ZoneInvalid,
}

impl fmt::Display for ZoneAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PeerUidMismatch => "zone-bootstrap-peer-uid-mismatch",
            Self::InvalidPeerUid => "zone-bootstrap-peer-uid-invalid",
            Self::ZoneInvalid => "zone-bootstrap-zone-invalid",
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
            return Err(ZoneAdmissionError::InvalidPeerUid);
        }
        let observed_uid = peer.credentials().uid().as_raw();
        if observed_uid != expected_uid {
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
