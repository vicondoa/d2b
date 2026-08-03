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
    UnexpectedFdCount { expected: usize, actual: usize },
    MissingCloexec,
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
        // `MSG_CMSG_CLOEXEC` is part of receipt, not a post-receipt repair:
        // setting FD_CLOEXEC with F_SETFD after recvmsg races a concurrent
        // fork+exec in the receiving process.
        let message = recvmsg::<()>(sock, &mut iov, Some(&mut cmsg), MsgFlags::MSG_CMSG_CLOEXEC)
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
        let stat = match fstat(*fd) {
            Ok(stat) => stat,
            Err(_) => {
                close_received_fds(&fds);
                return Err(FdPassingError::IOError);
            }
        };
        let key = (stat.st_dev, stat.st_ino, stat.st_mode);
        if !seen.insert(key) {
            close_received_fds(&fds);
            return Err(FdPassingError::DuplicateFdInSingleSend);
        }
        if !cloexec_is_set(*fd) {
            close_received_fds(&fds);
            return Err(FdPassingError::MissingCloexec);
        }
    }

    Ok((payload[..bytes].to_vec(), fds))
}

/// Receive exactly one descriptor. The operation-specific broker response
/// uses this helper so a response carrying zero or multiple descriptors
/// cannot be mistaken for a valid owned database handoff.
pub fn recv_one_fd(sock: RawFd) -> Result<(Vec<u8>, RawFd), FdPassingError> {
    let (payload, fds) = recv_fds(sock)?;
    if fds.len() != 1 {
        let actual = fds.len();
        close_received_fds(&fds);
        return Err(FdPassingError::UnexpectedFdCount {
            expected: 1,
            actual,
        });
    }
    Ok((payload, fds[0]))
}

fn cloexec_is_set(fd: RawFd) -> bool {
    fcntl(fd, FcntlArg::F_GETFD)
        .map(|flags| FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC))
        .unwrap_or(false)
}

fn close_received_fds(fds: &[RawFd]) {
    for fd in fds {
        let _ = close(*fd);
    }
}

fn io_error(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::process::Command;
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
        assert!(
            cloexec_is_set(received[0]),
            "SCM_RIGHTS receipt must atomically set FD_CLOEXEC"
        );

        write(&write_end, b"ok").expect("pipe write");
        let mut buf = [0_u8; 2];
        read(received[0], &mut buf).expect("pipe read through passed fd");
        assert_eq!(&buf, b"ok");
        close(received[0]).expect("close received fd");
    }

    #[test]
    fn scm_rights_receipt_fd_does_not_inherit_across_exec() {
        let _guard = fd_test_lock();
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let (read_end, _write_end) = pipe().expect("pipe");

        send_fds(left.as_raw_fd(), b"exec", &[read_end.as_raw_fd()]).expect("send fd");
        let (_payload, fd) = recv_one_fd(right.as_raw_fd()).expect("receive exactly one fd");
        assert!(cloexec_is_set(fd), "received fd must have FD_CLOEXEC");

        let mut child = Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("exec inheritance probe");
        let child_fd = format!("/proc/{}/fd/{fd}", child.id());
        let child_comm = format!("/proc/{}/comm", child.id());
        let mut inherited = true;
        let mut observed_exec = false;
        for _ in 0..50 {
            if let Ok(comm) = std::fs::read_to_string(&child_comm)
                && comm.trim() == "sleep"
            {
                observed_exec = true;
                inherited = std::path::Path::new(&child_fd).exists();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        close(fd).expect("close received fd");
        assert!(observed_exec, "exec probe did not reach sleep");
        assert!(!inherited, "received database-like fd leaked across exec");
    }

    #[test]
    fn recv_one_fd_rejects_zero_and_multiple_descriptors() {
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        send_fds(left.as_raw_fd(), b"none", &[]).expect("send payload");
        assert_eq!(
            recv_one_fd(right.as_raw_fd()).expect_err("zero fds must fail"),
            FdPassingError::MissingPassedFd
        );

        let (read_end, _write_end) = pipe().expect("pipe");
        send_fds(
            left.as_raw_fd(),
            b"many",
            &[read_end.as_raw_fd(), read_end.as_raw_fd()],
        )
        .expect("send duplicate fds");
        assert_eq!(
            recv_one_fd(right.as_raw_fd()).expect_err("duplicate fds must fail"),
            FdPassingError::DuplicateFdInSingleSend
        );
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
