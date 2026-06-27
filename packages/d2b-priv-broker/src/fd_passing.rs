use std::collections::HashSet;
use std::io;
use std::os::fd::RawFd;

use nix::cmsg_space;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::socket::{ControlMessage, ControlMessageOwned, MsgFlags, recvmsg, sendmsg};
use nix::sys::stat::fstat;
use nix::unistd::close;
use std::io::{IoSlice, IoSliceMut};

#[derive(Debug, PartialEq, Eq)]
pub enum FdPassingError {
    MissingPassedFd,
    DuplicateFdInSingleSend,
    IOError,
}

#[derive(Debug, Default)]
pub struct FdRegistry {
    owned: Vec<RawFd>,
}

impl FdRegistry {
    pub fn register(&mut self, fd: RawFd) {
        self.owned.push(fd);
    }

    pub fn clear(&mut self) {
        for fd in self.owned.drain(..) {
            let _ = close(fd);
        }
    }
}

impl Drop for FdRegistry {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug)]
pub struct FdLease {
    fd: Option<RawFd>,
}

impl FdLease {
    pub fn new(fd: RawFd) -> Self {
        Self { fd: Some(fd) }
    }

    pub fn raw(&self) -> Option<RawFd> {
        self.fd
    }

    pub fn release(&mut self) -> Option<RawFd> {
        self.fd.take()
    }
}

impl Drop for FdLease {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            let _ = close(fd);
        }
    }
}

pub fn send_fds(sock: RawFd, payload: &[u8], fds: &[RawFd]) -> io::Result<()> {
    let iov = [IoSlice::new(payload)];
    let sent = if fds.is_empty() {
        sendmsg::<()>(sock, &iov, &[], MsgFlags::empty(), None)
    } else {
        let cmsgs = [ControlMessage::ScmRights(fds)];
        sendmsg::<()>(sock, &iov, &cmsgs, MsgFlags::empty(), None)
    }
    .map_err(io_error)?;
    if sent != payload.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short SCM_RIGHTS send",
        ));
    }
    Ok(())
}

pub fn recv_fds(sock: RawFd) -> Result<(Vec<u8>, Vec<RawFd>), FdPassingError> {
    let mut payload = [0_u8; 256];
    let mut iov = [IoSliceMut::new(&mut payload)];
    let mut cmsg = cmsg_space!([RawFd; 8]);
    let (bytes, fds) = {
        let message = recvmsg::<()>(sock, &mut iov, Some(&mut cmsg), MsgFlags::empty())
            .map_err(|_| FdPassingError::IOError)?;
        let bytes = message.bytes;
        let mut fds = Vec::new();
        if let Ok(iter) = message.cmsgs() {
            for cmsg in iter {
                if let ControlMessageOwned::ScmRights(rights) = cmsg {
                    fds.extend(rights);
                }
            }
        }
        (bytes, fds)
    };

    if fds.is_empty() {
        return Err(FdPassingError::MissingPassedFd);
    }

    let mut seen = HashSet::new();
    for fd in &fds {
        let stat = fstat(*fd).map_err(|_| FdPassingError::IOError)?;
        let key = (stat.st_dev, stat.st_ino, stat.st_mode);
        if !seen.insert(key) {
            for received in fds {
                let _ = close(received);
            }
            return Err(FdPassingError::DuplicateFdInSingleSend);
        }
        set_cloexec(*fd).map_err(|_| FdPassingError::IOError)?;
    }

    Ok((payload[..bytes].to_vec(), fds))
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let current = fcntl(fd, FcntlArg::F_GETFD).map_err(io_error)?;
    let flags = FdFlag::from_bits_truncate(current) | FdFlag::FD_CLOEXEC;
    fcntl(fd, FcntlArg::F_SETFD(flags)).map_err(io_error)?;
    Ok(())
}

fn io_error(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use nix::unistd::{pipe, read, write};

    fn fd_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("fd test lock")
    }

    fn dup_high(fd: RawFd) -> RawFd {
        // Avoid false negatives from unrelated concurrently-running tests that
        // may reuse a freshly closed low fd number before this test can assert
        // the lease/registry closed it.
        for min_fd in [512, 256, 128, 64] {
            if let Ok(duplicated) = fcntl(fd, FcntlArg::F_DUPFD_CLOEXEC(min_fd)) {
                return duplicated;
            }
        }
        fcntl(fd, FcntlArg::F_DUPFD_CLOEXEC(0)).expect("F_DUPFD_CLOEXEC")
    }

    #[test]
    fn scm_rights_fd_lifecycle_accepts_and_returns_pipe_fd() {
        let _guard = fd_test_lock();
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let (read_end, write_end) = pipe().expect("pipe");

        send_fds(left.as_raw_fd(), b"fd", &[read_end.as_raw_fd()]).expect("send fd");
        let (payload, received) = recv_fds(right.as_raw_fd()).expect("recv fd");
        assert_eq!(payload, b"fd");
        assert_eq!(received.len(), 1);

        write(&write_end, b"ok").expect("pipe write");
        let mut buf = [0_u8; 2];
        read(received[0], &mut buf).expect("pipe read through passed fd");
        assert_eq!(&buf, b"ok");
        close(received[0]).expect("close received fd");
    }

    #[test]
    fn scm_rights_fd_lifecycle_refuses_duplicate_fd_send() {
        let _guard = fd_test_lock();
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let (read_end, _write_end) = pipe().expect("pipe");

        send_fds(
            left.as_raw_fd(),
            b"dup",
            &[read_end.as_raw_fd(), read_end.as_raw_fd()],
        )
        .expect("send dup fds");
        let error = recv_fds(right.as_raw_fd()).expect_err("duplicate fd should fail");
        assert_eq!(error, FdPassingError::DuplicateFdInSingleSend);
    }

    #[test]
    fn scm_rights_fd_lifecycle_closes_own_copy_on_error() {
        let _guard = fd_test_lock();
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let (read_end, _write_end) = pipe().expect("pipe");
        let broker_copy = dup_high(read_end.as_raw_fd());

        {
            let lease = FdLease::new(broker_copy);
            send_fds(left.as_raw_fd(), b"err", &[lease.raw().expect("lease raw")])
                .expect("send leased fd");
        }

        assert!(fcntl(broker_copy, FcntlArg::F_GETFD).is_err());
        let (_payload, received) = recv_fds(right.as_raw_fd()).expect("recv leased fd");
        for fd in received {
            close(fd).expect("close received fd");
        }
    }

    #[test]
    fn scm_rights_fd_lifecycle_cleans_up_on_broker_restart() {
        let _guard = fd_test_lock();
        let (read_end, _write_end) = pipe().expect("pipe");
        let tracked = dup_high(read_end.as_raw_fd());
        let mut registry = FdRegistry::default();
        registry.register(tracked);
        registry.clear();
        assert!(fcntl(tracked, FcntlArg::F_GETFD).is_err());
    }
}
