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

use std::os::fd::AsRawFd;

use d2b_priv_broker::fd_passing::{recv_fds, send_fds};
#[cfg(feature = "fake-backends")]
use d2b_priv_broker::ops::pidfd::test_harness::FakePidfdSpawner;
#[cfg(feature = "fake-backends")]
use d2b_priv_broker::ops::pidfd::{PidfdPayload, PidfdSpawner, assert_cloexec};
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

fn cloexec_set(fd: i32) -> bool {
    let flags = fcntl(fd, FcntlArg::F_GETFD).expect("fcntl F_GETFD");
    FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC)
}

#[test]
#[cfg(feature = "fake-backends")]
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
#[cfg(feature = "fake-backends")]
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

#[test]
fn paired_pidfds_keep_controller_then_broker_attachment_order() {
    let mut controller = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut broker = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let controller_pid = controller.id();
    let broker_pid = broker.id();
    let controller_fd = rustix::process::pidfd_open(
        rustix::process::Pid::from_raw(controller_pid as i32).unwrap(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap();
    let broker_fd = rustix::process::pidfd_open(
        rustix::process::Pid::from_raw(broker_pid as i32).unwrap(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap();
    let (tx, rx) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .unwrap();
    send_fds(
        tx.as_raw_fd(),
        b"realm-pair",
        &[controller_fd.as_raw_fd(), broker_fd.as_raw_fd()],
    )
    .unwrap();
    let (_, received) = recv_fds(rx.as_raw_fd()).unwrap();
    assert_eq!(received.len(), 2);

    let fdinfo_pid = |fd: i32| {
        std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}"))
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    };
    assert_eq!(fdinfo_pid(received[0]), controller_pid);
    assert_eq!(fdinfo_pid(received[1]), broker_pid);
    for fd in received {
        assert!(cloexec_set(fd));
        nix::unistd::close(fd).unwrap();
    }
    let _ = controller.kill();
    let _ = broker.kill();
    let _ = controller.wait();
    let _ = broker.wait();
}

#[test]
fn paired_pidfd_handoff_is_gated_by_first_packet_evidence() {
    let service = include_str!("../src/allocator_service.rs");
    let host = include_str!("../../d2b-host/src/realm_children.rs");
    let authenticate = service
        .find("authenticate_spawned_pair(pending.pair()")
        .expect("Spawn authenticates both children");
    let handoff = service
        .find("VerifiedPidfdAttachments::from_spawned(pair)")
        .expect("evidence-bearing paired pidfd handoff");
    let disarm = service
        .find("pending.disarm();")
        .expect("pending child guard disarms after handoff validation");
    assert!(
        authenticate < handoff && handoff < disarm,
        "pidfds must not enter SCM_RIGHTS before child credential evidence, and the cancellation guard must remain armed through handoff validation"
    );
    assert!(host.contains(".verify_first_packet_credentials(expected)"));
    assert!(host.contains("PidfdEvidence::new("));
    assert!(service.contains("evidence: PidfdEvidence,"));
    assert!(service.contains("PidfdIdentityPolicy::new("));
    assert!(service.contains("&self.pidfd"));
    assert!(service.contains("VerifiedPidfdAttachments(REDACTED)"));
    assert!(
        !host.contains("PeerCredentials::from_ucred"),
        "allocator code must not construct raw peer credentials"
    );
}
