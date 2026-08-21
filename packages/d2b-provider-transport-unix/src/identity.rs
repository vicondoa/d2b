//! Binding of one accepted Unix socket to one authenticated request.

use d2b_contracts_resource::v3::{
    ResourceRef,
    ZoneId,
};
use rustix::{
    fd::{AsFd, OwnedFd},
    net::{UCred, sockopt::get_socket_peercred},
};
use std::fmt;

/// Broker authority that is permitted to request an accepted-peer pidfd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerRole {
    /// The Zone controller performing one local transport operation.
    ZoneController,
    /// The transport service completing one accepted request.
    TransportService,
}

/// Caller-independent routing data for one authenticated request.
///
/// The binding is retained with the accepted socket only. It has no accessors
/// for identity evidence, descriptors, or file descriptors.
#[derive(Clone, PartialEq, Eq)]
pub struct TransportRequestBinding {
    zone: ZoneId,
    subject: ResourceRef,
    role: BrokerRole,
}

impl TransportRequestBinding {
    /// Create the routing data assigned by the authenticated Zone runtime.
    pub fn new(zone: ZoneId, subject: ResourceRef, role: BrokerRole) -> Self {
        Self {
            zone,
            subject,
            role,
        }
    }
}

impl fmt::Debug for TransportRequestBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransportRequestBinding(REDACTED)")
    }
}

/// One accepted file descriptor together with its kernel peer credentials.
pub(crate) struct AcceptedTransport {
    binding: TransportRequestBinding,
    peer: UCred,
    fd: OwnedFd,
}

impl AcceptedTransport {
    pub(crate) fn bind(
        binding: TransportRequestBinding,
        fd: OwnedFd,
    ) -> Result<Self, rustix::io::Errno> {
        let peer = get_socket_peercred(fd.as_fd())?;
        Ok(Self { binding, peer, fd })
    }

    pub(crate) fn into_parts(self) -> (TransportRequestBinding, UCred, OwnedFd) {
        (self.binding, self.peer, self.fd)
    }
}
