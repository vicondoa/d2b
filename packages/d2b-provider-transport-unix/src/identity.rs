//! Binding of one accepted Unix socket to one authenticated request.

use crate::graph_binding::KernelPeerPin;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use rustix::{
    fd::{AsFd, OwnedFd},
    net::{UCred, sockopt::get_socket_peercred},
};
use std::fmt;

/// Kernel peer credentials expected for one local transport request.
///
/// The process id is deliberately not part of this policy: it is only
/// kernel-observed evidence for the accepted socket and is never persisted in
/// the request binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedPeer {
    uid: u32,
    gid: u32,
}

impl ExpectedPeer {
    /// Construct an expected kernel uid/gid pair.
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }

    /// Return the expected uid.
    pub const fn uid(self) -> u32 {
        self.uid
    }

    /// Return the expected gid.
    pub const fn gid(self) -> u32 {
        self.gid
    }
}

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
    expected_peer: Option<ExpectedPeer>,
}

impl TransportRequestBinding {
    /// Create the routing data assigned by the authenticated Zone runtime.
    pub fn new(zone: ZoneId, subject: ResourceRef, role: BrokerRole) -> Self {
        Self {
            zone,
            subject,
            role,
            expected_peer: None,
        }
    }

    /// Bind the request to the kernel credentials expected at the endpoint.
    #[must_use]
    pub const fn with_expected_peer(mut self, expected_peer: ExpectedPeer) -> Self {
        self.expected_peer = Some(expected_peer);
        self
    }

    /// Return the optional kernel peer policy.
    pub const fn expected_peer(&self) -> Option<ExpectedPeer> {
        self.expected_peer
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
    /// Bind an accepted descriptor to a caller-supplied request binding.
    ///
    /// The kernel peer policy on this path is the one the request binding
    /// carries beside it, which is why the graph-bound path below does not
    /// use it.
    pub(crate) fn bind(
        binding: TransportRequestBinding,
        fd: OwnedFd,
    ) -> Result<Self, rustix::io::Errno> {
        let peer = get_socket_peercred(fd.as_fd())?;
        if let Some(expected) = binding.expected_peer
            && !peer_is(&peer, (expected.uid(), expected.gid()))
        {
            return Err(rustix::io::Errno::ACCESS);
        }
        Ok(Self { binding, peer, fd })
    }

    /// Bind an accepted descriptor under an admitted relationship's own pin.
    ///
    /// The pin arrives from the relationship, so the same comparison governs
    /// both paths and a caller has no field in which to substitute a peer
    /// identity of its own.
    pub(crate) fn bind_under_pin(
        binding: TransportRequestBinding,
        pin: Option<&KernelPeerPin>,
        fd: OwnedFd,
    ) -> Result<Self, rustix::io::Errno> {
        let peer = get_socket_peercred(fd.as_fd())?;
        if let Some(pin) = pin
            && !peer_is(&peer, (pin.uid(), pin.gid()))
        {
            return Err(rustix::io::Errno::ACCESS);
        }
        Ok(Self { binding, peer, fd })
    }

    pub(crate) fn into_parts(self) -> (TransportRequestBinding, UCred, OwnedFd) {
        (self.binding, self.peer, self.fd)
    }
}

/// The one kernel peer comparison both binding paths use.
fn peer_is(peer: &UCred, pinned: (u32, u32)) -> bool {
    (peer.uid.as_raw(), peer.gid.as_raw()) == pinned
}
