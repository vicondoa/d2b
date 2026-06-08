//! W3 s1 + P2 watchdog surface.
//!
//! Re-exports the pidfd table (the W3 s1 deliverable that the rest of
//! the daemon already depends on) and adds the **P2 virtiofsd
//! watchdog** glue that the daemon's pidfd reaper consults when a
//! registered runner exits.
//!
//! Before P2 the per-share `nixling-<vm>-virtiofsd@<share>.service`
//! ExecStopPost-style bash health check + `nixling-vfsd-watchdog@<vm>`
//! timer was the only thing that noticed virtiofsd dying mid-run.
//! P2 moves that surface inside the daemon: every virtiofsd runner the
//! daemon spawns lives in the pidfd table, and when its pidfd reports
//! exit the supervisor classifies the exit, marks the per-share mount
//! as degraded, surfaces a typed `vfsd-died` event for the audit log,
//! and (per policy) signals the dependent cloud-hypervisor runner so
//! the VM does not keep running with a broken virtiofs root.
//!
//! The per-share systemd template stays on disk for the P6 deletion
//! sweep; P2 only owns the in-daemon detection-and-degradation path.

pub use super::pidfd_table::*;

use nixling_ipc::broker_wire::RunnerRole;
use serde::{Deserialize, Serialize};

/// Operator policy that controls how the watchdog reacts to a
/// virtiofsd exit. The default mirrors today's bash watchdog: a dead
/// virtiofsd takes the VM with it, because the guest's root mount
/// path is irrecoverable once the FUSE server is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VfsdWatchdogPolicy {
    /// When `true` the watchdog asks the supervisor to stop the
    /// dependent cloud-hypervisor runner the moment virtiofsd dies
    /// unexpectedly. When `false` the share is still marked degraded
    /// and the typed `vfsd-died` audit event is still emitted, but
    /// CH is left alone for operator-driven recovery.
    pub stop_ch_on_unexpected_exit: bool,
}

impl Default for VfsdWatchdogPolicy {
    fn default() -> Self {
        Self {
            stop_ch_on_unexpected_exit: true,
        }
    }
}

/// Classified exit status of a reaped runner pidfd. Matches the two
/// `waitid` reportable terminations (exit code vs fatal signal); a
/// `None` exit code means "wait reported `WSIGNALED`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerExitInfo {
    /// Exit code if the runner returned from `main`. `0` is success.
    pub exit_code: Option<i32>,
    /// Signal number if the kernel killed the runner (SIGKILL,
    /// SIGSEGV, SIGTERM-while-uncaught, ...).
    pub signal: Option<i32>,
}

impl RunnerExitInfo {
    pub fn from_exit_code(code: i32) -> Self {
        Self {
            exit_code: Some(code),
            signal: None,
        }
    }

    pub fn from_signal(signal: i32) -> Self {
        Self {
            exit_code: None,
            signal: Some(signal),
        }
    }

    /// Returns `true` iff the runner exited cleanly: `exit_code == 0`
    /// and no killing signal. Anything else (non-zero exit, signal
    /// termination, or — defensively — both fields `None`) is an
    /// **unexpected** exit and should trigger degradation.
    pub fn is_clean(&self) -> bool {
        matches!(self.exit_code, Some(0)) && self.signal.is_none()
    }
}

/// Typed event surfaced on the supervisor's event channel when a
/// runner exit observation completes its classification. The daemon
/// loop consumes these events; tests assert on them directly so the
/// watchdog logic is verifiable without the live channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SupervisorEvent {
    /// Virtiofsd died unexpectedly. Per-share mount is marked
    /// degraded; the audit log carries the `vfsd-died` record.
    VfsdDied {
        vm: String,
        role_id: String,
        exit: RunnerExitInfo,
    },
    /// Per-share virtiofs mount degraded — the dependent CH runner
    /// cannot serve guest filesystem traffic for that share. The
    /// integrator surfaces this in `nixling status <vm>` and the
    /// per-VM degraded counter for observability.
    VfsdShareDegraded { vm: String, role_id: String },
    /// Supervisor must stop the dependent runner. The watchdog only
    /// emits this for cloud-hypervisor today (the lone consumer of
    /// the virtiofs root share); future roles will gain their own
    /// emission rules.
    StopRunnerRequested {
        vm: String,
        runner_role: RunnerRole,
        reason: String,
    },
}

/// Audit record persisted to the broker audit log so post-mortems can
/// reconstruct what the watchdog decided. The shape is deliberately
/// minimal and self-describing — the integrator wraps it into the
/// existing `OpAuditRecord` envelope when the audit-log writer
/// surfaces this event (P2-fu integrator scope; the typed event is
/// available immediately for in-process logging).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VfsdDiedAuditRecord {
    pub event: String,
    pub vm: String,
    pub role_id: String,
    pub exit: RunnerExitInfo,
    pub policy_stopped_ch: bool,
}

impl VfsdDiedAuditRecord {
    pub const EVENT_NAME: &'static str = "vfsd-died";
}

/// Output of the pure watchdog handler. The caller (the daemon's
/// pidfd reap loop) forwards `events` onto the supervisor event
/// channel and, when `audit` is `Some`, appends it to the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchdogOutcome {
    pub events: Vec<SupervisorEvent>,
    pub audit: Option<VfsdDiedAuditRecord>,
}

impl WatchdogOutcome {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.audit.is_none()
    }
}

/// Pure classification of one observed runner exit. Returns an empty
/// outcome for runners the watchdog does not own (everything except
/// `Virtiofsd`) and for clean virtiofsd exits (operator-initiated
/// stop). For unexpected virtiofsd exits this is the entry point that
/// produces:
///
/// 1. a `VfsdDied` typed event,
/// 2. a `VfsdShareDegraded` typed event for the per-share mount,
/// 3. when `policy.stop_ch_on_unexpected_exit` is true, a
///    `StopRunnerRequested { runner_role: CloudHypervisor }`
///    event so the supervisor drives CH down through its existing
///    SIGTERM→SIGKILL pidfd ladder,
/// 4. a `VfsdDiedAuditRecord` for the audit log.
pub fn handle_runner_exit(
    vm: &str,
    role_id: &str,
    role: RunnerRole,
    exit: RunnerExitInfo,
    policy: VfsdWatchdogPolicy,
) -> WatchdogOutcome {
    if role != RunnerRole::Virtiofsd {
        return WatchdogOutcome::default();
    }
    if exit.is_clean() {
        return WatchdogOutcome::default();
    }

    let mut events = Vec::with_capacity(3);
    events.push(SupervisorEvent::VfsdDied {
        vm: vm.to_owned(),
        role_id: role_id.to_owned(),
        exit,
    });
    events.push(SupervisorEvent::VfsdShareDegraded {
        vm: vm.to_owned(),
        role_id: role_id.to_owned(),
    });

    let stopped_ch = policy.stop_ch_on_unexpected_exit;
    if stopped_ch {
        events.push(SupervisorEvent::StopRunnerRequested {
            vm: vm.to_owned(),
            runner_role: RunnerRole::CloudHypervisor,
            reason: format!(
                "virtiofsd role {role_id} exited unexpectedly; root share is unrecoverable"
            ),
        });
    }

    let audit = VfsdDiedAuditRecord {
        event: VfsdDiedAuditRecord::EVENT_NAME.to_owned(),
        vm: vm.to_owned(),
        role_id: role_id.to_owned(),
        exit,
        policy_stopped_ch: stopped_ch,
    };

    WatchdogOutcome {
        events,
        audit: Some(audit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_vfsd_died(event: &SupervisorEvent, vm: &str, role_id: &str, exit: RunnerExitInfo) {
        match event {
            SupervisorEvent::VfsdDied {
                vm: ev_vm,
                role_id: ev_role,
                exit: ev_exit,
            } => {
                assert_eq!(ev_vm, vm);
                assert_eq!(ev_role, role_id);
                assert_eq!(*ev_exit, exit);
            }
            other => panic!("expected VfsdDied, got {other:?}"),
        }
    }

    fn assert_share_degraded(event: &SupervisorEvent, vm: &str, role_id: &str) {
        match event {
            SupervisorEvent::VfsdShareDegraded {
                vm: ev_vm,
                role_id: ev_role,
            } => {
                assert_eq!(ev_vm, vm);
                assert_eq!(ev_role, role_id);
            }
            other => panic!("expected VfsdShareDegraded, got {other:?}"),
        }
    }

    fn assert_stop_ch(event: &SupervisorEvent, vm: &str) {
        match event {
            SupervisorEvent::StopRunnerRequested {
                vm: ev_vm,
                runner_role,
                reason,
            } => {
                assert_eq!(ev_vm, vm);
                assert_eq!(*runner_role, RunnerRole::CloudHypervisor);
                assert!(
                    reason.contains("virtiofsd"),
                    "reason should mention virtiofsd, got {reason:?}"
                );
            }
            other => panic!("expected StopRunnerRequested, got {other:?}"),
        }
    }

    #[test]
    fn virtiofsd_nonzero_exit_degrades_and_stops_ch_by_default() {
        let exit = RunnerExitInfo::from_exit_code(1);
        let outcome = handle_runner_exit(
            "alpha",
            "virtiofsd-ro-store",
            RunnerRole::Virtiofsd,
            exit,
            VfsdWatchdogPolicy::default(),
        );

        assert_eq!(outcome.events.len(), 3);
        assert_vfsd_died(&outcome.events[0], "alpha", "virtiofsd-ro-store", exit);
        assert_share_degraded(&outcome.events[1], "alpha", "virtiofsd-ro-store");
        assert_stop_ch(&outcome.events[2], "alpha");

        let audit = outcome.audit.expect("audit record emitted");
        assert_eq!(audit.event, VfsdDiedAuditRecord::EVENT_NAME);
        assert_eq!(audit.vm, "alpha");
        assert_eq!(audit.role_id, "virtiofsd-ro-store");
        assert_eq!(audit.exit, exit);
        assert!(audit.policy_stopped_ch);
    }

    #[test]
    fn virtiofsd_signal_exit_treated_as_unexpected() {
        let exit = RunnerExitInfo::from_signal(libc::SIGKILL);
        let outcome = handle_runner_exit(
            "alpha",
            "virtiofsd-nl-meta",
            RunnerRole::Virtiofsd,
            exit,
            VfsdWatchdogPolicy::default(),
        );
        assert_eq!(outcome.events.len(), 3);
        assert_vfsd_died(&outcome.events[0], "alpha", "virtiofsd-nl-meta", exit);
        assert_share_degraded(&outcome.events[1], "alpha", "virtiofsd-nl-meta");
        assert_stop_ch(&outcome.events[2], "alpha");
        assert!(
            outcome
                .audit
                .as_ref()
                .expect("audit on signal exit")
                .policy_stopped_ch
        );
    }

    #[test]
    fn virtiofsd_clean_exit_emits_nothing() {
        let outcome = handle_runner_exit(
            "alpha",
            "virtiofsd-ro-store",
            RunnerRole::Virtiofsd,
            RunnerExitInfo::from_exit_code(0),
            VfsdWatchdogPolicy::default(),
        );
        assert!(outcome.is_empty(), "clean exit must not degrade: {outcome:?}");
    }

    #[test]
    fn non_virtiofsd_role_is_ignored_by_watchdog() {
        for role in [
            RunnerRole::CloudHypervisor,
            RunnerRole::Swtpm,
            RunnerRole::Gpu,
            RunnerRole::Audio,
        ] {
            let outcome = handle_runner_exit(
                "alpha",
                "some-role",
                role,
                RunnerExitInfo::from_exit_code(137),
                VfsdWatchdogPolicy::default(),
            );
            assert!(
                outcome.is_empty(),
                "watchdog must ignore {role:?}: {outcome:?}"
            );
        }
    }

    #[test]
    fn policy_off_keeps_ch_running_but_still_audits_and_degrades() {
        let exit = RunnerExitInfo::from_exit_code(2);
        let outcome = handle_runner_exit(
            "alpha",
            "virtiofsd-ro-store",
            RunnerRole::Virtiofsd,
            exit,
            VfsdWatchdogPolicy {
                stop_ch_on_unexpected_exit: false,
            },
        );

        assert_eq!(outcome.events.len(), 2);
        assert_vfsd_died(&outcome.events[0], "alpha", "virtiofsd-ro-store", exit);
        assert_share_degraded(&outcome.events[1], "alpha", "virtiofsd-ro-store");
        for ev in &outcome.events {
            assert!(
                !matches!(ev, SupervisorEvent::StopRunnerRequested { .. }),
                "policy off must not request CH stop"
            );
        }
        let audit = outcome.audit.expect("audit still emitted under policy-off");
        assert!(!audit.policy_stopped_ch);
    }

    #[test]
    fn audit_record_roundtrips_through_json() {
        let original = VfsdDiedAuditRecord {
            event: VfsdDiedAuditRecord::EVENT_NAME.to_owned(),
            vm: "alpha".to_owned(),
            role_id: "virtiofsd-ro-store".to_owned(),
            exit: RunnerExitInfo::from_signal(libc::SIGSEGV),
            policy_stopped_ch: true,
        };
        let json = serde_json::to_string(&original).expect("serialize audit");
        assert!(json.contains("\"event\":\"vfsd-died\""));
        let parsed: VfsdDiedAuditRecord =
            serde_json::from_str(&json).expect("deserialize audit");
        assert_eq!(parsed, original);
    }

    #[test]
    fn default_policy_stops_ch() {
        assert!(VfsdWatchdogPolicy::default().stop_ch_on_unexpected_exit);
    }

    #[test]
    fn exit_info_clean_classification() {
        assert!(RunnerExitInfo::from_exit_code(0).is_clean());
        assert!(!RunnerExitInfo::from_exit_code(1).is_clean());
        assert!(!RunnerExitInfo::from_signal(libc::SIGKILL).is_clean());
        let neither = RunnerExitInfo {
            exit_code: None,
            signal: None,
        };
        assert!(!neither.is_clean());
    }
}
