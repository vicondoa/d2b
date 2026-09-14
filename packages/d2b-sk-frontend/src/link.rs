//! The Guest's one connection to the allocator.
//!
//! The frontend is not the allocator's client by design: it supplies the
//! connect step and nothing else, and the toolkit's base drives the enrollment
//! and the session over whatever this returns. Here that transport is the
//! contract's native-vsock framing
//! ([`d2b_session_unix::FramedVsockTransport`]) over the hypervisor host's
//! AF_VSOCK endpoint.

use d2b_provider_toolkit::{GuestError, GuestLink, GuestLinkFuture};
use d2b_session::OwnedTransport;
use tokio_vsock::{VMADDR_CID_HOST, VsockAddr, VsockStream};

/// AF_VSOCK CID of the hypervisor host (`VMADDR_CID_HOST`).
pub const VSOCK_HOST_CID: u32 = VMADDR_CID_HOST;

/// Default VSOCK port for the d2b security-key Guest session.
pub const SK_VSOCK_PORT: u32 = 14320;

/// The allocator endpoint one Guest agent connects to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VsockAllocatorLink {
    cid: u32,
    port: u32,
}

impl VsockAllocatorLink {
    /// Bind the link to one allocator endpoint.
    pub const fn new(cid: u32, port: u32) -> Self {
        Self { cid, port }
    }

    /// Bind the link to the hypervisor host at one port.
    pub const fn host(port: u32) -> Self {
        Self::new(VSOCK_HOST_CID, port)
    }

    /// The allocator CID this link connects to.
    pub const fn cid(&self) -> u32 {
        self.cid
    }

    /// The allocator port this link connects to.
    pub const fn port(&self) -> u32 {
        self.port
    }
}

impl GuestLink for VsockAllocatorLink {
    fn connect(&self) -> GuestLinkFuture {
        let address = VsockAddr::new(self.cid, self.port);
        Box::pin(async move {
            let stream = VsockStream::connect(address)
                .await
                .map_err(|_| GuestError::LinkUnavailable)?;
            let transport: Box<dyn OwnedTransport> =
                Box::new(d2b_session_unix::FramedVsockTransport::new(stream));
            Ok(transport)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_endpoint_is_the_hypervisor_host() {
        let link = VsockAllocatorLink::host(SK_VSOCK_PORT);
        assert_eq!(link.cid(), 2, "VMADDR_CID_HOST is 2");
        assert_eq!(link.port(), 14320);
        assert_eq!(VSOCK_HOST_CID, 2);
    }
}
