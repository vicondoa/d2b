//! The session-host ↔ window-frontend transport.
//!
//! `AF_UNIX` + **`SOCK_SEQPACKET`**: one bounded datagram per frame. A stream
//! socket would let reads coalesce or split, which can pair a frame with the
//! wrong ancillary descriptors — a corruption class we design out rather than
//! test for.
//!
//! The socket is an inherited `socketpair`, never a filesystem path, so no other
//! process can connect to it and receive retained framebuffer descriptors.
//!
//! Everything here is safe Rust: `rustix` provides the `SCM_RIGHTS` ancillary
//! APIs. (The crate as a whole is `unsafe_code = "deny"` with one audited
//! module, `serve::sys`, which this transport does not use.)
//!
//! **Status:** implemented and unit-tested, but not yet the live Phase-1 path.
//! See `DESIGN.md`.

use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, SocketFlags, SocketType, socketpair,
};

use super::dto::{MAX_FRAME_BYTES, MAX_FRAME_FDS};

/// Ancillary space for the maximum descriptor count we ever send.
const CMSG_SPACE: usize = rustix::cmsg_space!(ScmRights(MAX_FRAME_FDS));

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("io error")]
    Io(#[from] rustix::io::Errno),
    #[error("frame encode/decode failed")]
    Codec,
    #[error("frame exceeded the size limit")]
    FrameTooLarge,
    #[error("too many descriptors attached")]
    TooManyFds,
    #[error("expected {expected} descriptors, received {received}")]
    FdCountMismatch { expected: usize, received: usize },
    #[error("message was truncated")]
    Truncated,
    #[error("peer closed the connection")]
    PeerClosed,
}

/// Create the connected pair. One end is retained by the session host, the other
/// is handed to the frontend as its stdin.
pub fn pair() -> Result<(OwnedFd, OwnedFd), TransportError> {
    Ok(socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?)
}

/// One end of the frontend channel.
#[derive(Debug)]
pub struct Channel {
    sock: OwnedFd,
}

impl Channel {
    pub fn new(sock: OwnedFd) -> Self {
        Self { sock }
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.sock.as_fd()
    }

    /// Send one frame with its descriptors attached.
    ///
    /// The payload and the descriptors travel in the same datagram, so they can
    /// never be paired incorrectly.
    pub fn send<T: serde::Serialize>(
        &self,
        msg: &T,
        fds: &[BorrowedFd<'_>],
    ) -> Result<(), TransportError> {
        if fds.len() > MAX_FRAME_FDS {
            return Err(TransportError::TooManyFds);
        }
        let payload = postcard::to_allocvec(msg).map_err(|_| TransportError::Codec)?;
        if payload.len() > MAX_FRAME_BYTES {
            return Err(TransportError::FrameTooLarge);
        }

        let mut space = [MaybeUninit::<u8>::uninit(); CMSG_SPACE];
        let mut anc = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() && !anc.push(SendAncillaryMessage::ScmRights(fds)) {
            return Err(TransportError::TooManyFds);
        }

        rustix::net::sendmsg(
            &self.sock,
            &[std::io::IoSlice::new(&payload)],
            &mut anc,
            SendFlags::NOSIGNAL,
        )?;
        Ok(())
    }

    /// Receive one frame, requiring exactly `expected_fds` descriptors.
    ///
    /// Any surplus descriptor is closed rather than leaked, and a mismatch is an
    /// error: a malformed peer must not be able to exhaust our descriptor table
    /// or slip an unexpected descriptor past us.
    pub fn recv<T: serde::de::DeserializeOwned>(
        &self,
        expected_fds: usize,
    ) -> Result<(T, Vec<OwnedFd>), TransportError> {
        let mut buf = vec![0u8; MAX_FRAME_BYTES];
        let mut space = [MaybeUninit::<u8>::uninit(); CMSG_SPACE];
        let mut anc = RecvAncillaryBuffer::new(&mut space);

        let ret = rustix::net::recvmsg(
            &self.sock,
            &mut [std::io::IoSliceMut::new(&mut buf)],
            &mut anc,
            // CMSG_CLOEXEC so a received descriptor can never leak through a
            // later exec, even transiently.
            RecvFlags::CMSG_CLOEXEC,
        )?;

        // Collect first, so descriptors are owned (and thus closed on any
        // subsequent early return) before we validate anything.
        let mut fds = Vec::new();
        for m in anc.drain() {
            if let RecvAncillaryMessage::ScmRights(iter) = m {
                fds.extend(iter);
            }
        }

        if ret.bytes == 0 && fds.is_empty() {
            return Err(TransportError::PeerClosed);
        }
        if ret.flags.contains(rustix::net::ReturnFlags::TRUNC)
            || ret.flags.contains(rustix::net::ReturnFlags::CTRUNC)
        {
            return Err(TransportError::Truncated);
        }
        if fds.len() != expected_fds {
            return Err(TransportError::FdCountMismatch {
                expected: expected_fds,
                received: fds.len(),
            });
        }

        let msg: T = postcard::from_bytes(&buf[..ret.bytes]).map_err(|_| TransportError::Codec)?;
        Ok((msg, fds))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::ids::{BufferUseId, Generation, SurfaceId};
    use crate::model::ledger::DownRef;
    use crate::wire::dto::ToHost;
    use std::os::fd::AsRawFd;

    fn a_ref() -> DownRef {
        DownRef {
            generation: Generation(1),
            surface: SurfaceId(2),
            use_id: BufferUseId(3),
            seq: 4,
        }
    }

    fn scratch_fd() -> OwnedFd {
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
            .unwrap()
            .0
    }

    #[test]
    fn frame_round_trips_without_fds() {
        let (a, b) = pair().unwrap();
        let (tx, rx) = (Channel::new(a), Channel::new(b));
        let msg = ToHost::HostCommitted { r#ref: a_ref() };
        tx.send(&msg, &[]).unwrap();
        let (got, fds): (ToHost, _) = rx.recv(0).unwrap();
        assert_eq!(got, msg);
        assert!(fds.is_empty());
    }

    #[test]
    fn frame_round_trips_with_fds() {
        let (a, b) = pair().unwrap();
        let (tx, rx) = (Channel::new(a), Channel::new(b));
        let f = scratch_fd();
        tx.send(&ToHost::Bye, &[f.as_fd()]).unwrap();
        let (got, fds): (ToHost, _) = rx.recv(1).unwrap();
        assert_eq!(got, ToHost::Bye);
        assert_eq!(fds.len(), 1);
        // A genuinely different descriptor number referring to the same object.
        assert_ne!(fds[0].as_raw_fd(), f.as_raw_fd());
    }

    /// A frame carrying more descriptors than the receiver expects must fail —
    /// and must not leak the surplus.
    #[test]
    fn unexpected_fd_count_is_rejected() {
        let (a, b) = pair().unwrap();
        let (tx, rx) = (Channel::new(a), Channel::new(b));
        let f = scratch_fd();
        tx.send(&ToHost::Bye, &[f.as_fd()]).unwrap();
        let err = rx.recv::<ToHost>(0).unwrap_err();
        assert!(matches!(
            err,
            TransportError::FdCountMismatch {
                expected: 0,
                received: 1
            }
        ));
    }

    #[test]
    fn too_many_fds_is_refused_before_send() {
        let (a, _b) = pair().unwrap();
        let tx = Channel::new(a);
        let fds: Vec<OwnedFd> = (0..MAX_FRAME_FDS + 1).map(|_| scratch_fd()).collect();
        let borrowed: Vec<BorrowedFd<'_>> = fds.iter().map(|f| f.as_fd()).collect();
        assert!(matches!(
            tx.send(&ToHost::Bye, &borrowed).unwrap_err(),
            TransportError::TooManyFds
        ));
    }

    #[test]
    fn peer_close_is_reported() {
        let (a, b) = pair().unwrap();
        let rx = Channel::new(a);
        drop(b);
        assert!(matches!(
            rx.recv::<ToHost>(0).unwrap_err(),
            TransportError::PeerClosed
        ));
    }

    /// Datagram boundaries are preserved: two sends are never coalesced into one
    /// receive. This is the property a stream socket would not give us.
    #[test]
    fn datagram_boundaries_are_preserved() {
        let (a, b) = pair().unwrap();
        let (tx, rx) = (Channel::new(a), Channel::new(b));
        tx.send(&ToHost::Hello { proto: 1 }, &[]).unwrap();
        tx.send(&ToHost::Bye, &[]).unwrap();
        let (first, _): (ToHost, _) = rx.recv(0).unwrap();
        let (second, _): (ToHost, _) = rx.recv(0).unwrap();
        assert_eq!(first, ToHost::Hello { proto: 1 });
        assert_eq!(second, ToHost::Bye);
    }
}
