//! Same-user compositor attachment portal.

use d2b_contracts::v3::ResourceRef;
use std::collections::BTreeMap;

use crate::AttachmentGrantHandle;

/// Portal admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalError {
    /// The delivering peer is not the enrolled same-user supervisor.
    PeerMismatch,
    /// The requested User reference does not match the portal.
    UserMismatch,
    /// The session already holds an attachment.
    SessionExists,
    /// The portal reached its active-session bound.
    Capacity,
    /// The requested session is not active.
    UnknownSession,
}

impl core::fmt::Display for PortalError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::PeerMismatch => "display-portal-peer-mismatch",
            Self::UserMismatch => "display-portal-user-mismatch",
            Self::SessionExists => "display-portal-session-exists",
            Self::Capacity => "display-portal-capacity",
            Self::UnknownSession => "display-portal-session-unknown",
        })
    }
}

impl std::error::Error for PortalError {}

/// Opaque per-session compositor attachment grant.
#[derive(PartialEq, Eq)]
pub struct PortalGrant {
    session_digest: [u8; 32],
    handle: AttachmentGrantHandle,
}

impl PortalGrant {
    #[allow(dead_code)]
    pub(crate) fn into_parts(self) -> ([u8; 32], AttachmentGrantHandle) {
        (self.session_digest, self.handle)
    }
}

/// Supervisor-issued identity binding for one display portal grant.
pub struct PortalSessionBinding([u8; 32]);

impl PortalSessionBinding {
    /// Construct a binding at the Core/Supervisor boundary.
    #[allow(dead_code)]
    pub(crate) const fn from_supervisor(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for PortalGrant {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PortalGrant(<redacted>)")
    }
}

/// User-session compositor portal.
pub struct DisplayUserPortal {
    user_ref: ResourceRef,
    supervisor_uid: u32,
    max_sessions: usize,
    active: BTreeMap<String, ()>,
}

impl DisplayUserPortal {
    /// Construct a portal bound to one authenticated user supervisor.
    pub fn new(
        user_ref: ResourceRef,
        supervisor_uid: u32,
        max_sessions: usize,
    ) -> Result<Self, PortalError> {
        if user_ref.resource_type().as_str() != "User" {
            return Err(PortalError::UserMismatch);
        }
        if max_sessions == 0 {
            return Err(PortalError::Capacity);
        }
        Ok(Self {
            user_ref,
            supervisor_uid,
            max_sessions,
            active: BTreeMap::new(),
        })
    }

    /// Issue one opaque grant after the same-user check.
    pub fn issue_grant(
        &mut self,
        session_binding: PortalSessionBinding,
        requested_user: &ResourceRef,
        peer_uid: u32,
        handle: AttachmentGrantHandle,
    ) -> Result<PortalGrant, PortalError> {
        if peer_uid != self.supervisor_uid {
            return Err(PortalError::PeerMismatch);
        }
        if requested_user != &self.user_ref {
            return Err(PortalError::UserMismatch);
        }
        let session_digest = Self::hex_digest(session_binding.0);
        if self.active.contains_key(&session_digest) {
            return Err(PortalError::SessionExists);
        }
        if self.active.len() >= self.max_sessions {
            return Err(PortalError::Capacity);
        }
        let grant = PortalGrant {
            session_digest: session_binding.0,
            handle,
        };
        self.active.insert(session_digest, ());
        Ok(grant)
    }

    fn hex_digest(bytes: [u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Revoke a grant after the corresponding Process is gone.
    pub fn revoke(&mut self, session_digest: &str) -> Result<(), PortalError> {
        self.active
            .remove(session_digest)
            .map(|_| ())
            .ok_or(PortalError::UnknownSession)
    }

    /// Revoke a session grant idempotently during finalization.
    pub fn revoke_idempotent(&mut self, session_digest: &str) -> bool {
        self.active.remove(session_digest).is_some()
    }

    /// Return the number of active session grants.
    pub fn active_sessions(&self) -> usize {
        self.active.len()
    }
}

impl core::fmt::Debug for DisplayUserPortal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DisplayUserPortal")
            .field("active_sessions", &self.active.len())
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_grants_require_peer_and_revoke_idempotently() {
        let user = ResourceRef::parse("User/alice").unwrap();
        let mut portal = DisplayUserPortal::new(user.clone(), 1000, 1).unwrap();
        assert_eq!(
            portal.issue_grant(
                PortalSessionBinding::from_supervisor([2; 32]),
                &user,
                1001,
                AttachmentGrantHandle::from_supervisor([1; 32]),
            ),
            Err(PortalError::PeerMismatch)
        );
        assert!(
            portal
                .issue_grant(
                    PortalSessionBinding::from_supervisor([2; 32]),
                    &user,
                    1000,
                    AttachmentGrantHandle::from_supervisor([1; 32]),
                )
                .is_ok()
        );
        let session_digest = DisplayUserPortal::hex_digest([2; 32]);
        assert!(portal.revoke_idempotent(&session_digest));
        assert!(!portal.revoke_idempotent(&session_digest));
    }

    #[test]
    fn portal_rejects_zero_capacity_as_capacity_error() {
        assert!(matches!(
            DisplayUserPortal::new(ResourceRef::parse("User/alice").unwrap(), 1000, 0),
            Err(PortalError::Capacity)
        ));
    }
}
