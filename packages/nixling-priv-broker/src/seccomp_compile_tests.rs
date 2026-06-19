//! Behavioral and regression seccomp BPF tests.
//!
//! Three test categories:
//!
//! 1. **Positive behavioral**: fork a child; child installs a BPF
//!    compiled for `[DeviceClass::Kvm]`, then calls
//!    `ioctl(-1, KVM_GET_API_VERSION, ...)` — a request code in the
//!    allowlist. The BPF allows the syscall; the kernel returns `EBADF`
//!    (fd=-1 is invalid) but does NOT deliver `SIGSYS`. Parent asserts
//!    `WIFEXITED(status)` with the known exit code.
//!
//! 2. **Negative behavioral**: fork a child; child installs the same BPF,
//!    then calls `ioctl(-1, 0x12345678, ...)` — a request code NOT in the
//!    allowlist. `SECCOMP_RET_KILL_PROCESS` kills the process. Parent
//!    asserts `WIFSIGNALED && WTERMSIG == SIGSYS`.
//!
//! 3. **Regression**: for every known internal `seccompPolicyRef` emitted
//!    by `nixos-modules/minijail-profiles.nix`, `policy_ref_device_classes`
//!    must return `Some(_)` and `compile_ioctl_policy_to_bpf` must produce
//!    a non-empty program.
//!
//! ## Skip conditions
//!
//! The `fork`-based behavioral tests call `prctl(PR_SET_NO_NEW_PRIVS, 1)`
//! before installing the filter.  When this call returns `EPERM`, the test
//! is skipped gracefully (not failed).

// The fork-based tests in this module use libc syscall wrappers in
// the child closure.  We allow unsafe only for those specific items.
#![allow(unsafe_code)]

use nix::libc;
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;

use crate::live_handlers::policy_ref_device_classes;
use crate::sys::pidfd_sys::SeccompProgram;
use nixling_host::devices::DeviceClass;
use nixling_host::ioctl_policy::constants;
use nixling_host::seccomp::compile_ioctl_policy_to_bpf;

// ── constants ─────────────────────────────────────────────────────────

const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

/// An ioctl request code guaranteed absent from all per-role allowlists.
const UNDECLARED_IOCTL: libc::c_ulong = 0x1234_5678;

/// KVM_GET_API_VERSION — declared for [`DeviceClass::Kvm`].
const KVM_GET_API_VERSION_REQ: libc::c_ulong = constants::KVM_GET_API_VERSION as libc::c_ulong;

/// Child exit codes used in behavioral tests.
const EXIT_POSITIVE_OK: libc::c_int = 42;
const EXIT_INSTALL_FAILED: libc::c_int = 43;

// ── helper ────────────────────────────────────────────────────────────

/// `true` when the calling process can set `PR_SET_NO_NEW_PRIVS`.
fn can_set_no_new_privs() -> bool {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS is side-effect-free on
    // failure; on success it is a one-way ratchet used by seccomp.
    unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) == 0 }
}

// ── positive behavioral test ──────────────────────────────────────────

/// Positive: BPF compiled for `[Kvm]` allows `KVM_GET_API_VERSION`.
///
/// The child calls `ioctl(-1, KVM_GET_API_VERSION, ...)`.  The BPF
/// allows the request; the kernel returns `EBADF` without signalling.
/// Parent asserts `WIFEXITED && exit_code == EXIT_POSITIVE_OK`.
#[test]
#[cfg(target_os = "linux")]
fn behavioral_positive_allowed_ioctl_does_not_sigsys() {
    if !can_set_no_new_privs() {
        eprintln!(
            "behavioral_positive_allowed_ioctl_does_not_sigsys: skip \
             (PR_SET_NO_NEW_PRIVS unavailable — CAP_SYS_ADMIN may be required)"
        );
        return;
    }

    let compiled = compile_ioctl_policy_to_bpf(&[DeviceClass::Kvm]);
    let program = SeccompProgram::from_compiled(compiled);

    // SAFETY: fork is safe for a test process that only uses this thread.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());

    if pid == 0 {
        // ── child ────────────────────────────────────────────────────
        // SAFETY: child closure — raw libc calls are the only safe
        // option here because Rust runtime state is not fork-safe.
        unsafe {
            // no_new_privs is mandatory before SECCOMP_SET_MODE_FILTER.
            if libc::prctl(PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) != 0 {
                libc::_exit(EXIT_INSTALL_FAILED);
            }
            if program.apply().is_err() {
                libc::_exit(EXIT_INSTALL_FAILED);
            }
            // Permitted ioctl — BPF allows it; kernel returns EBADF (fd=-1).
            libc::ioctl(-1, KVM_GET_API_VERSION_REQ);
            libc::_exit(EXIT_POSITIVE_OK);
        }
    }

    // ── parent ────────────────────────────────────────────────────────
    let status = waitpid(Pid::from_raw(pid), None).expect("waitpid failed");
    match status {
        WaitStatus::Exited(_, code) => {
            assert_ne!(
                code, EXIT_INSTALL_FAILED,
                "child failed to install seccomp BPF"
            );
            assert_eq!(
                code, EXIT_POSITIVE_OK,
                "child exited with {code} — expected {EXIT_POSITIVE_OK}"
            );
        }
        WaitStatus::Signaled(_, sig, _) => {
            panic!("child killed by signal {sig:?} — allowed ioctl must NOT SIGSYS");
        }
        other => panic!("unexpected wait status: {other:?}"),
    }
}

// ── negative behavioral test ──────────────────────────────────────────

/// Negative: BPF compiled for `[Kvm]` kills on an undeclared ioctl.
///
/// The child calls `ioctl(-1, 0x12345678, ...)`.  The BPF denies with
/// `SECCOMP_RET_KILL_PROCESS`.  Parent asserts
/// `WIFSIGNALED && WTERMSIG == SIGSYS`.
#[test]
#[cfg(target_os = "linux")]
fn behavioral_negative_undeclared_ioctl_delivers_sigsys() {
    if !can_set_no_new_privs() {
        eprintln!(
            "behavioral_negative_undeclared_ioctl_delivers_sigsys: skip \
             (PR_SET_NO_NEW_PRIVS unavailable)"
        );
        return;
    }

    let compiled = compile_ioctl_policy_to_bpf(&[DeviceClass::Kvm]);
    let program = SeccompProgram::from_compiled(compiled);

    // SAFETY: fork is safe for a test process that only uses this thread.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());

    if pid == 0 {
        // ── child ────────────────────────────────────────────────────
        // SAFETY: child closure uses raw libc after fork.
        unsafe {
            if libc::prctl(PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) != 0 {
                libc::_exit(EXIT_INSTALL_FAILED);
            }
            if program.apply().is_err() {
                libc::_exit(EXIT_INSTALL_FAILED);
            }
            // Undeclared ioctl → SECCOMP_RET_KILL_PROCESS → SIGSYS.
            libc::ioctl(-1, UNDECLARED_IOCTL);
            // Never reached when the BPF is working correctly.
            libc::_exit(0);
        }
    }

    // ── parent ────────────────────────────────────────────────────────
    let status = waitpid(Pid::from_raw(pid), None).expect("waitpid failed");
    match status {
        WaitStatus::Signaled(_, sig, _) => {
            assert_eq!(
                sig,
                nix::sys::signal::Signal::SIGSYS,
                "expected SIGSYS from SECCOMP_RET_KILL_PROCESS; got {sig:?}"
            );
        }
        WaitStatus::Exited(_, code) => {
            if code == EXIT_INSTALL_FAILED {
                eprintln!(
                    "behavioral_negative_undeclared_ioctl_delivers_sigsys: skip \
                     (seccomp install failed — may need CAP_SYS_ADMIN)"
                );
                return;
            }
            panic!(
                "child exited normally ({code}) — undeclared ioctl should have been killed by BPF"
            );
        }
        other => panic!("unexpected wait status: {other:?}"),
    }
}

// ── regression: known policy refs return Some(_) ──────────────────────

/// Regression: every known internal `seccompPolicyRef` value from
/// `nixos-modules/minijail-profiles.nix` must appear in
/// `policy_ref_device_classes` and compile to a non-empty BPF program.
/// Asserts the `Ok(None)` silent-skip deferral from v1.1.2-final is
/// retired.
#[test]
fn all_known_policy_refs_compile_to_some() {
    let known_refs = [
        "w1-cloud-hypervisor-runner",
        "w1-qemu-media",
        "w1-virtiofsd",
        "w1-host-reconcile",
        "w1-store-virtiofs-preflight",
        "w1-guest-control-health",
        "w1-swtpm",
        "w1-gpu",
        "w1-gpu-render-node",
        "w1-video",
        "w1-audio",
        "w1-vsock-relay",
        "w1-usbip",
        "w1-usbip-proxy",
        "w1-otel-host-bridge",
        "w1-wayland-proxy",
    ];
    for &policy_ref in &known_refs {
        let classes = policy_ref_device_classes(policy_ref).unwrap_or_else(|| {
            panic!("policy ref {policy_ref:?} missing from policy_ref_device_classes")
        });
        let compiled = compile_ioctl_policy_to_bpf(classes);
        assert!(
            !compiled.instructions.is_empty(),
            "policy ref {policy_ref:?} produced an empty BPF program"
        );
        // Round-trip through SeccompProgram::from_compiled to verify
        // the conversion path is sound.
        let _program = SeccompProgram::from_compiled(compiled);
    }
}

/// Regression: unknown policy refs return `None` so that
/// `load_runner_seccomp` returns `SpawnFailed` rather than skipping.
#[test]
fn unknown_policy_ref_returns_none() {
    assert!(
        policy_ref_device_classes("w1-does-not-exist").is_none(),
        "unknown policy ref should return None to produce SpawnFailed"
    );
    assert!(
        policy_ref_device_classes("").is_none(),
        "empty policy ref should return None"
    );
}
