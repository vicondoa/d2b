//! W3 H2 real-host integration: drive [`RealPidfdSpawner`] against the
//! kernel and assert the documented invariants:
//!
//! - CLOEXEC is set on the returned pidfd (post-spawn `fcntl(F_GETFD)`).
//! - The reconciliation path agrees with the spawn-time start_time.
//! - Re-running reconciliation with a wrong `expected_start_time`
//!   returns `pidfd-reconciliation-drift`.
//!
//! Gated behind `cfg(target_os = "linux")` because both `clone3` and
//! `pidfd_open` are Linux-only.

#![cfg(target_os = "linux")]

use std::os::fd::AsRawFd;
use std::path::PathBuf;

use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nixling_priv_broker::ops::pidfd::{
    PidfdMethod, PidfdOpError, PidfdPayload, PidfdSpawner, RealPidfdSpawner, StartTime,
};

fn cloexec_set(fd: i32) -> bool {
    let flags = fcntl(fd, FcntlArg::F_GETFD).expect("fcntl F_GETFD");
    FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC)
}

#[test]
fn real_spawner_pidfd_is_cloexec_and_start_time_round_trips() {
    let spawner = RealPidfdSpawner::new();
    // We use a small but real argv. The child will exec /bin/true (or
    // just exit cleanly) — what we care about is the pidfd contract,
    // not the child program's behavior.
    let payload = PidfdPayload {
        argv: vec!["/bin/true".into()],
        vm_id: "alpha".into(),
        role_id: "ch".into(),
        state_dir: PathBuf::from("/var/lib/nixling/vms/alpha"),
    };
    let (handle, fd, method) = match spawner.spawn(payload) {
        Ok(v) => v,
        // Some sandboxed CI environments forbid clone/fork entirely;
        // skip rather than fail in that case.
        Err(PidfdOpError::Clone3Failed { detail }) if detail.contains("EPERM") => {
            eprintln!("skipping real_spawner: clone3 EPERM in this env: {detail}");
            return;
        }
        Err(err) => panic!("spawn failed: {err}"),
    };
    assert!(handle.cloexec, "handle.cloexec should be set");
    assert!(
        cloexec_set(fd.as_raw_fd()),
        "FD_CLOEXEC must be live on the returned pidfd"
    );
    assert!(handle.start_time_ticks > 0);
    assert!(matches!(
        method,
        PidfdMethod::Clone3 | PidfdMethod::ForkPidfdOpen
    ));

    // Reconciliation against the same pid + observed start time must
    // succeed.
    let (reconciled, _fd2) = spawner
        .reconcile(handle.pid, StartTime(handle.start_time_ticks))
        .expect("reconcile happy path");
    assert_eq!(reconciled.pid, handle.pid);
    assert_eq!(reconciled.start_time_ticks, handle.start_time_ticks);

    // Drifted start time → refusal.
    let err = spawner
        .reconcile(handle.pid, StartTime(handle.start_time_ticks + 999_999))
        .unwrap_err();
    match err {
        PidfdOpError::ReconciliationStartTimeMismatch { .. } => {}
        other => panic!("expected start-time drift, got {other:?}"),
    }
}
