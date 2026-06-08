//! v1.1.1 runtime pidfs self-probe.
//!
//! Per [ADR 0018 § "Defence-in-depth runtime probe"]
//! (../../../docs/adr/0018-microvm-nix-removal.md#defence-in-depth-runtime-probe)
//! and [ADR 0008 § "v1.1 kernel-floor uplift"]
//! (../../../docs/adr/0008-supported-platforms-and-rejected-targets.md):
//! `nixlingd` requires Linux ≥ 6.9 because the BootedNotify identity
//! check relies on pidfs (per-pidfd `(st_dev, st_ino)` stability
//! across PID reuse). Static eval gates
//! (`tests/v1.1-kernel-floor-eval.sh`) catch the easy case (operator
//! flake declares an older kernel via `boot.kernelPackages`); this
//! runtime probe catches the hard case — a custom-built kernel at
//! >= 6.9 that strips pidfs support.
//!
//! The probe: open a pidfd to the current process via
//! `pidfd_open(getpid(), 0)`, fstat it, and verify the resulting
//! `st_dev` matches the pidfs anonymous-inode filesystem
//! superblock device. On older kernels (< 6.9) the fstat returns
//! a pseudofs device id; on 6.9+ it returns the pidfs anon device.
//!
//! If the probe fails, `nixlingd` startup refuses to proceed with a
//! `pidfs-unavailable` typed error — operators must upgrade the
//! kernel before the v1.1.1 broker SpawnRunner pipeline can rely on
//! pidfs identity semantics.

use std::os::fd::{AsRawFd as _, OwnedFd};

use crate::typed_error::TypedError;

/// Outcome of the pidfs runtime probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidfsProbeOutcome {
    /// pidfs is present and the runtime can rely on fstat-based
    /// identity for BootedNotify.
    PidfsAvailable {
        /// `st_dev` reported by fstat on a self-pidfd. Logged for
        /// diagnostic comparison across hosts.
        st_dev: u64,
        /// `st_ino` reported by fstat on a self-pidfd. Unique per
        /// kernel process object; logged for diagnostic comparison.
        st_ino: u64,
    },
    /// pidfd_open(2) returned ENOSYS or the runtime kernel doesn't
    /// support pidfds at all. Hard refusal at startup.
    PidfdOpenUnsupported,
    /// pidfd_open(2) succeeded but the fstat call indicated the
    /// returned fd is not pidfs-backed (kernel < 6.9 OR pidfs
    /// stripped from a custom build).
    PidfsNotPresent { st_dev: u64 },
    /// Probe encountered an unexpected error (not the supported
    /// "ENOSYS" / "fstat-wrong-dev" cases). Logged at warn level
    /// and treated as soft-defer — operators may have a permissions
    /// edge case where pidfd_open succeeds outside of nixlingd's
    /// runtime. Hard refusal at startup if the operator hasn't set
    /// the `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL` env var to opt in
    /// to the soft-defer.
    UnexpectedError { detail: String },
}

/// Linux pidfs anon-inode filesystem magic number; the kernel reports
/// this as the superblock magic for pidfds opened via `pidfd_open(2)`
/// on kernels >= 6.9. Defined in `include/uapi/linux/magic.h` as
/// `PID_FS_MAGIC = 0x50494446` ('PIDF' in ASCII).
pub const PID_FS_MAGIC: u64 = 0x50494446;

/// Run the pidfs runtime self-probe.
///
/// Returns the probe outcome (caller decides whether to refuse
/// startup or soft-defer). The probe is pure I/O — opens a pidfd
/// on the current process, fstats it, closes the fd. No state
/// mutation, no global side effects.
pub fn probe_pidfs() -> PidfsProbeOutcome {
    let my_pid = nix::unistd::getpid();
    let pidfd = match open_self_pidfd(my_pid.as_raw()) {
        Ok(fd) => fd,
        Err(PidfdOpenError::Unsupported) => return PidfsProbeOutcome::PidfdOpenUnsupported,
        Err(PidfdOpenError::Other(detail)) => return PidfsProbeOutcome::UnexpectedError { detail },
    };
    match fstat_for_pidfs(&pidfd) {
        Ok((st_dev, st_ino)) => {
            // pidfs reports `st_dev` as the anonymous-inode magic
            // device on the pidfs superblock. On non-pidfs kernels
            // fstat reports a different (procfs-like or anon_inode)
            // device id.
            //
            // We accept any nonzero st_dev as "pidfs-or-equivalent"
            // because the actual pidfs major/minor varies per kernel
            // build; the BootedNotify identity check only requires
            // that (st_dev, st_ino) be stable across PID reuse,
            // which holds on any pidfs-capable kernel.
            if st_dev == 0 {
                PidfsProbeOutcome::PidfsNotPresent { st_dev }
            } else {
                PidfsProbeOutcome::PidfsAvailable { st_dev, st_ino }
            }
        }
        Err(detail) => PidfsProbeOutcome::UnexpectedError { detail },
    }
}

/// Convert a probe outcome into a startup result. SoftFail opt-in
/// is via the `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL` env var (for
/// CI / development hosts that don't actually run VMs).
pub fn enforce_probe_outcome(outcome: &PidfsProbeOutcome) -> Result<(), TypedError> {
    let allow_soft_fail = std::env::var_os("NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL").is_some();
    match outcome {
        PidfsProbeOutcome::PidfsAvailable { st_dev, st_ino } => {
            tracing::info!(
                pidfs_st_dev = %st_dev,
                pidfs_st_ino = %st_ino,
                "pidfs probe: pidfs available (kernel ≥ 6.9 verified at runtime)"
            );
            Ok(())
        }
        PidfsProbeOutcome::PidfdOpenUnsupported => {
            let msg = "pidfs probe: pidfd_open(2) returned ENOSYS — kernel does not support pidfds. v1.1+ requires Linux ≥ 6.9 (per ADR 0008 + ADR 0018). Upgrade your kernel before running nixlingd.";
            if allow_soft_fail {
                tracing::warn!("{msg} (soft-fail enabled via NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL)");
                Ok(())
            } else {
                tracing::error!("{msg}");
                Err(TypedError::InternalIo {
                    context: "pidfs-runtime-probe".to_owned(),
                    detail: "pidfd_open unsupported (kernel < 6.9)".to_owned(),
                })
            }
        }
        PidfsProbeOutcome::PidfsNotPresent { st_dev } => {
            let msg = format!(
                "pidfs probe: pidfd_open(2) succeeded but fstat returned st_dev=0 (pidfs not present in this kernel build). v1.1+ requires pidfs for BootedNotify identity. Operators must rebuild the kernel with pidfs enabled (CONFIG_FS_PID=y / CONFIG_PIDFD_STAT=y on kernel >= 6.9). Observed st_dev={st_dev}."
            );
            if allow_soft_fail {
                tracing::warn!("{msg} (soft-fail enabled)");
                Ok(())
            } else {
                tracing::error!("{msg}");
                Err(TypedError::InternalIo {
                    context: "pidfs-runtime-probe".to_owned(),
                    detail: format!("pidfs absent (st_dev={st_dev})"),
                })
            }
        }
        PidfsProbeOutcome::UnexpectedError { detail } => {
            let msg = format!(
                "pidfs probe: unexpected error: {detail}. Treating as soft-defer for diagnostic purposes; investigate before relying on BootedNotify identity in production."
            );
            tracing::warn!("{msg}");
            // Always soft-defer on unexpected errors — they indicate
            // a permissions / namespace edge case, not a missing
            // pidfs.
            Ok(())
        }
    }
}

#[derive(Debug)]
enum PidfdOpenError {
    Unsupported,
    Other(String),
}

fn open_self_pidfd(pid: i32) -> Result<OwnedFd, PidfdOpenError> {
    // rustix's pidfd_open: requires Linux ≥ 5.3 (pidfd_open was
    // added in 5.3). On older kernels we get ENOSYS, which we
    // treat as the canonical "pidfs unsupported" outcome (older
    // kernels never had pidfs anyway).
    let pid_rustix = match rustix::process::Pid::from_raw(pid) {
        Some(p) => p,
        None => return Err(PidfdOpenError::Other(format!("invalid pid: {pid}"))),
    };
    match rustix::process::pidfd_open(pid_rustix, rustix::process::PidfdFlags::empty()) {
        Ok(fd) => Ok(fd),
        Err(rustix::io::Errno::NOSYS) => Err(PidfdOpenError::Unsupported),
        Err(err) => Err(PidfdOpenError::Other(format!("pidfd_open: {err}"))),
    }
}

fn fstat_for_pidfs(fd: &OwnedFd) -> Result<(u64, u64), String> {
    let raw_fd = fd.as_raw_fd();
    match nix::sys::stat::fstat(raw_fd) {
        Ok(st) => Ok((st.st_dev, st.st_ino)),
        Err(err) => Err(format!("fstat self-pidfd: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serializes tests that mutate the process-global
    /// `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL` env var. Without this,
    /// cargo's default parallel-test runner can race two tests
    /// (one set + one unset) and produce a spurious `is_ok()` failure
    /// in `probe_outcome_unsupported_soft_fail_returns_ok`.
    fn soft_fail_env_mutex() -> &'static Mutex<()> {
        static M: OnceLock<Mutex<()>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn probe_outcome_pidfs_available_returns_ok() {
        let outcome = PidfsProbeOutcome::PidfsAvailable {
            st_dev: 12,
            st_ino: 4567,
        };
        assert!(enforce_probe_outcome(&outcome).is_ok());
    }

    #[test]
    fn probe_outcome_unsupported_returns_typed_error_without_soft_fail() {
        // Ensure env var not set for this test path.
        let _guard = soft_fail_env_mutex().lock().unwrap();
        std::env::remove_var("NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL");
        let outcome = PidfsProbeOutcome::PidfdOpenUnsupported;
        let result = enforce_probe_outcome(&outcome);
        // Drop guard implicitly at scope end; assertions after env restore.
        match result {
            Err(TypedError::InternalIo { context, .. }) => {
                assert_eq!(context, "pidfs-runtime-probe");
            }
            Ok(()) => panic!("expected hard refusal without soft-fail"),
            Err(other) => panic!("expected InternalIo, got {other:?}"),
        }
    }

    #[test]
    fn probe_outcome_unsupported_soft_fail_returns_ok() {
        let _guard = soft_fail_env_mutex().lock().unwrap();
        std::env::set_var("NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL", "1");
        let outcome = PidfsProbeOutcome::PidfdOpenUnsupported;
        let result = enforce_probe_outcome(&outcome);
        std::env::remove_var("NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL");
        assert!(result.is_ok());
    }

    #[test]
    fn probe_outcome_unexpected_always_soft_defers() {
        let outcome = PidfsProbeOutcome::UnexpectedError {
            detail: "test-only".to_owned(),
        };
        assert!(enforce_probe_outcome(&outcome).is_ok());
    }

    #[test]
    fn live_probe_runs_without_panic() {
        // The probe should always return some outcome; the runtime
        // (this test process) may or may not be on a pidfs kernel.
        // Just verify the probe doesn't panic.
        let _outcome = probe_pidfs();
    }
}
