//! Integration: pidfd handoff over `SCM_RIGHTS`.
//!
//! Drives the broker's existing [`d2b_priv_broker::fd_passing`]
//! helpers end-to-end against a real `socketpair(SOCK_SEQPACKET)`,
//! using a pidfd-shaped stand-in fd from
//! [`d2b_priv_broker::ops::pidfd::test_harness`].
//!
//! Assertions:
//!
//! 1. The pidfd produced by the fake spawner is `CLOEXEC`.
//! 2. The SCM_RIGHTS transport delivers exactly one fd to the
//!    receiver.
//! 3. The receiver-side fd remains valid after the broker copy is
//!    dropped (i.e. it really is a freshly-duplicated kernel handle,
//!    not the broker's own descriptor).
//! 4. The `O_CLOEXEC` flag survives the transport (this is a kernel
//!    invariant for SCM_RIGHTS — we check it on the receiving side).

#![cfg(feature = "fake-backends")]

use std::os::fd::AsRawFd;

use d2b_priv_broker::fd_passing::{recv_fds, send_fds};
use d2b_priv_broker::ops::pidfd::test_harness::FakePidfdSpawner;
use d2b_priv_broker::ops::pidfd::{PidfdPayload, PidfdSpawner, assert_cloexec};
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

fn cloexec_set(fd: i32) -> bool {
    let flags = fcntl(fd, FcntlArg::F_GETFD).expect("fcntl F_GETFD");
    FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC)
}

#[test]
fn scm_rights_transports_pidfd_with_cloexec_preserved() {
    let spawner = FakePidfdSpawner::default();
    let (handle, broker_fd, _method) = spawner
        .spawn(PidfdPayload {
            argv: vec!["/sbin/cloud-hypervisor".into()],
            vm_id: "alpha".into(),
            role_id: "ch".into(),
            state_dir: "/var/lib/d2b/vms/alpha".into(),
        })
        .expect("spawn");
    assert_cloexec(&handle).expect("spawner sets CLOEXEC");
    assert!(
        cloexec_set(broker_fd.as_raw_fd()),
        "stand-in pipe fd should be O_CLOEXEC"
    );

    let (tx, rx) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");

    let payload = b"pidfd-handoff\x00";
    send_fds(tx.as_raw_fd(), payload, &[broker_fd.as_raw_fd()]).expect("send_fds");
    // Drop the broker copy after the kernel has duplicated it into the
    // socket buffer to prove the receiver gets its own kernel handle.
    drop(broker_fd);

    let (body, fds) = recv_fds(rx.as_raw_fd()).expect("recv_fds");
    assert_eq!(body, payload);
    assert_eq!(fds.len(), 1);
    let received_raw = fds[0];
    assert!(
        cloexec_set(received_raw),
        "SCM_RIGHTS-delivered fd must remain CLOEXEC after transport"
    );

    // Close the freshly-delivered fd with a safe wrapper; the
    // surrounding crate's `#![deny(unsafe_code)]` forbids us from
    // promoting the raw fd into an `OwnedFd` ourselves.
    nix::unistd::close(received_raw).expect("close received fd");

    drop(tx);
    drop(rx);
}

#[test]
fn reconciliation_refuses_start_time_drift() {
    use d2b_priv_broker::ops::pidfd::StartTime;
    let s = FakePidfdSpawner::default();
    let err = s
        .reconcile_drift(4242, StartTime(1_000_000), StartTime(2_000_000))
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("start-time drifted"),
        "unexpected error: {msg}"
    );
}
