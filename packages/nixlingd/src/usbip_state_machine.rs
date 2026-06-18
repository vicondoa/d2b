//! Typed, per-busid state machine that pins the canonical bring-up order
//! for a USBIP-passthrough
//! device the daemon (via the privileged broker) attaches into a
//! target VM.
//!
//! # Why this state machine exists
//!
//! The host-side USBIP path is a chain of cooperating subsystems —
//! the `usbip-host` kernel module, a per-busid file lock under
//! `/run/nixling/locks/usbip/<busid>`, the per-env nftables
//! carve-out, the per-env usbipd backend + proxy runners, and the
//! per-busid bind operation itself. Any step out of order silently
//! corrupts state:
//!
//! * Binding before `modprobe usbip-host` succeeds returns a
//!   confusing `ENODEV` deep inside the broker call site.
//! * Skipping the per-busid lock lets two envs race for the same
//!   physical device — both win briefly, then one loses on the
//!   first I/O.
//! * Opening the firewall before withholding non-owner-env
//!   `SpawnRunner`s for the same busid leaves a brief window where
//!   another env's backend can accept the connection.
//! * Starting the proxy before the backend is up means the first
//!   guest USB transfer races readiness and looks like a `usbip:
//!   error: connect failed`.
//!
//! AGENTS.md's "Critical subsystems" row for the control plane pins the
//! canonical order as:
//!
//! ```text
//! modprobe → lock → withhold → firewall → backend → bind → proxy
//! ```
//!
//! and the stop path reverses it. This module turns that ordering
//! into a typed Rust enum + plan + executor so call sites can't
//! shuffle the steps and every failure surfaces through the typed
//! error surface as `UsbipStepFailed { busid, step, reason }`
//! (exit code 67).
//!
//! # Shape
//!
//! * [`UsbipBusidStep`] — typed enum, one variant per canonical
//!   step.
//! * [`UsbipBusidPlan`] — `{ busid, env, steps }` with the
//!   canonical order pinned by construction.
//! * [`build_usbip_plan`] — pure constructor; takes a
//!   `BundleResolver` so callers can resolve the per-env intents
//!   (firewall + bind) before invoking the executor.
//! * [`UsbipStepExecutor`] — trait one method per step. Production
//!   wires this through the broker dispatch surface; tests inject
//!   a fixture executor.
//! * [`execute_usbip_plan`] — drives the plan in order, fail-fast
//!   on the first step that returns an error. Returns a typed
//!   [`UsbipExecutionReport`] with the per-step outcome trace.
//!
//! Failures from any step are normalised to
//! [`TypedError::UsbipStepFailed`] so the daemon's public error
//! envelope is uniform regardless of which step blew up.

use std::fmt;

use nixling_core::bundle_resolver::BundleResolver;
use serde::{Deserialize, Serialize};

use crate::typed_error::TypedError;

/// One node in the canonical per-busid USBIP state machine.
///
/// The order of variants in this enum is documentation-only — the
/// actual canonical order is encoded by [`CANONICAL_STEPS`] and
/// pinned at plan construction in [`build_usbip_plan`]. Tests
/// assert that the two stay aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipBusidStep {
    /// `ModprobeIfAllowed(usbip-host)` against the trusted-bundle
    /// kernel-module matrix. MUST be first — every later step
    /// silently no-ops without the kernel symbol surface.
    Modprobe,
    /// Acquire `/run/nixling/locks/usbip/<busid>` for the target
    /// env. Held until the stop path releases it.
    Lock,
    /// Withhold non-owner-env `SpawnRunner` requests for the same
    /// busid. Closes the race window before the firewall opens.
    Withhold,
    /// Render + apply the per-env `inet nixling` firewall
    /// carve-out (`UsbipBindFirewallRule`) so the per-env proxy
    /// can accept the bind.
    Firewall,
    /// Start the per-env usbipd backend runner (`SpawnRunner`
    /// with `RunnerRole::Usbip` for `sys-<env>-usbipd`).
    Backend,
    /// Issue `UsbipBind { bus_id, vm }` so the kernel binds the
    /// physical device to the per-env usbipd backend.
    Bind,
    /// Open the per-env usbipd proxy listen socket so the target
    /// VM can attach to the now-bound device.
    Proxy,
}

impl UsbipBusidStep {
    /// Stable kebab-case identifier used in typed errors,
    /// audit-log fields, and tests. Keeping this stable matters —
    /// operators grep for `usbip-step-failed: step=firewall …`
    /// across hosts.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modprobe => "modprobe",
            Self::Lock => "lock",
            Self::Withhold => "withhold",
            Self::Firewall => "firewall",
            Self::Backend => "backend",
            Self::Bind => "bind",
            Self::Proxy => "proxy",
        }
    }
}

impl fmt::Display for UsbipBusidStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical bring-up order. The plan constructor pins this; no
/// caller may rearrange it.
pub const CANONICAL_STEPS: [UsbipBusidStep; 7] = [
    UsbipBusidStep::Modprobe,
    UsbipBusidStep::Lock,
    UsbipBusidStep::Withhold,
    UsbipBusidStep::Firewall,
    UsbipBusidStep::Backend,
    UsbipBusidStep::Bind,
    UsbipBusidStep::Proxy,
];

/// Typed plan for bringing up a single busid into a single env.
///
/// `busid` and `env` are taken verbatim from the resolved bundle
/// intents — callers MUST NOT compose synthetic busids here. The
/// `steps` field is the canonical bring-up order; the stop path
/// is the same list reversed (use [`UsbipBusidPlan::stop_order`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipBusidPlan {
    pub busid: String,
    pub env: String,
    pub vm: String,
    pub steps: Vec<UsbipBusidStep>,
}

impl UsbipBusidPlan {
    /// Stop-path order: the canonical list reversed
    /// (`proxy → bind → backend → firewall → withhold → lock → modprobe`).
    ///
    /// Note that "Modprobe" at the tail is intentionally a no-op
    /// on the stop path — the kernel module stays loaded — but
    /// keeping it in the reversed list keeps the executor's
    /// stop-side dispatch table aligned with the bring-up table.
    pub fn stop_order(&self) -> Vec<UsbipBusidStep> {
        let mut steps = self.steps.clone();
        steps.reverse();
        steps
    }
}

/// Build the canonical per-busid plan.
///
/// `resolver` is consulted to assert the per-env firewall and
/// per-(env, vm, busid) bind intents the executor will later
/// dispatch actually exist in the trusted bundle. If either is
/// missing, the constructor returns a [`TypedError::UsbipStepFailed`]
/// tagged against the step whose preconditions failed
/// (`firewall` or `bind`). This is fail-fast at *plan time* so
/// no executor side-effects ever run for a malformed plan.
pub fn build_usbip_plan(
    busid: &str,
    env: &str,
    vm: &str,
    resolver: &BundleResolver,
) -> Result<UsbipBusidPlan, TypedError> {
    if busid.is_empty() {
        return Err(TypedError::UsbipStepFailed {
            busid: busid.to_owned(),
            step: UsbipBusidStep::Lock,
            reason: "bus_id is empty".to_owned(),
        });
    }
    if env.is_empty() {
        return Err(TypedError::UsbipStepFailed {
            busid: busid.to_owned(),
            step: UsbipBusidStep::Lock,
            reason: "env is empty".to_owned(),
        });
    }
    if vm.is_empty() {
        return Err(TypedError::UsbipStepFailed {
            busid: busid.to_owned(),
            step: UsbipBusidStep::Bind,
            reason: "vm is empty".to_owned(),
        });
    }

    let firewall_id = nixling_core::bundle_resolver::intent_id_usbip_firewall(env, busid);
    if resolver.find_usbip_firewall_intent(&firewall_id).is_none() {
        return Err(TypedError::UsbipStepFailed {
            busid: busid.to_owned(),
            step: UsbipBusidStep::Firewall,
            reason: format!(
                "trusted bundle has no usbip firewall intent for env={env} busid={busid}"
            ),
        });
    }

    let bind_id = nixling_core::bundle_resolver::intent_id_usbip_bind(env, vm, busid);
    if resolver.find_usbip_bind_intent(&bind_id).is_none() {
        return Err(TypedError::UsbipStepFailed {
            busid: busid.to_owned(),
            step: UsbipBusidStep::Bind,
            reason: format!(
                "trusted bundle has no usbip bind intent for env={env} vm={vm} busid={busid}"
            ),
        });
    }

    Ok(UsbipBusidPlan {
        busid: busid.to_owned(),
        env: env.to_owned(),
        vm: vm.to_owned(),
        steps: CANONICAL_STEPS.to_vec(),
    })
}

/// Side-effecting dispatch trait. Production wires this through
/// the broker; tests inject a fixture that records call order
/// and can fail a chosen step.
///
/// Each method MUST be idempotent — replays of the same plan
/// after a partial failure are expected.
pub trait UsbipStepExecutor {
    fn modprobe(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn acquire_lock(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn withhold_non_owners(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn apply_firewall(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn start_backend(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn bind(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
    fn start_proxy(&mut self, plan: &UsbipBusidPlan) -> Result<(), String>;
}

/// Per-step outcome recorded during execution. Successful steps
/// land in `completed`; the first failure lands in `failed` and
/// execution halts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsbipExecutionReport {
    pub busid: String,
    pub env: String,
    pub vm: String,
    pub completed: Vec<UsbipBusidStep>,
    pub failed: Option<(UsbipBusidStep, String)>,
}

impl UsbipExecutionReport {
    /// `true` iff every canonical step completed.
    pub fn is_ok(&self) -> bool {
        self.failed.is_none() && self.completed.len() == CANONICAL_STEPS.len()
    }
}

/// Drive the plan top-to-bottom, fail-fast on the first error.
///
/// On failure, the executor's prior successful steps stay
/// recorded in `completed` so the stop-path / reconciler can
/// undo them in reverse order. The error is returned as a typed
/// [`TypedError::UsbipStepFailed`] tagged with the exact step
/// that blew up; the caller can lift it into the public error
/// envelope unchanged.
pub fn execute_usbip_plan<E: UsbipStepExecutor>(
    plan: &UsbipBusidPlan,
    executor: &mut E,
) -> Result<UsbipExecutionReport, (UsbipExecutionReport, TypedError)> {
    let mut report = UsbipExecutionReport {
        busid: plan.busid.clone(),
        env: plan.env.clone(),
        vm: plan.vm.clone(),
        completed: Vec::with_capacity(plan.steps.len()),
        failed: None,
    };

    for step in &plan.steps {
        let result = match step {
            UsbipBusidStep::Modprobe => executor.modprobe(plan),
            UsbipBusidStep::Lock => executor.acquire_lock(plan),
            UsbipBusidStep::Withhold => executor.withhold_non_owners(plan),
            UsbipBusidStep::Firewall => executor.apply_firewall(plan),
            UsbipBusidStep::Backend => executor.start_backend(plan),
            UsbipBusidStep::Bind => executor.bind(plan),
            UsbipBusidStep::Proxy => executor.start_proxy(plan),
        };
        match result {
            Ok(()) => report.completed.push(*step),
            Err(reason) => {
                report.failed = Some((*step, reason.clone()));
                let err = TypedError::UsbipStepFailed {
                    busid: plan.busid.clone(),
                    step: *step,
                    reason,
                };
                return Err((report, err));
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording executor that fails on a chosen step.
    struct FixtureExecutor {
        calls: Vec<UsbipBusidStep>,
        fail_at: Option<(UsbipBusidStep, &'static str)>,
    }

    impl FixtureExecutor {
        fn ok() -> Self {
            Self {
                calls: Vec::new(),
                fail_at: None,
            }
        }
        fn failing(step: UsbipBusidStep, reason: &'static str) -> Self {
            Self {
                calls: Vec::new(),
                fail_at: Some((step, reason)),
            }
        }
        fn dispatch(&mut self, step: UsbipBusidStep) -> Result<(), String> {
            self.calls.push(step);
            if let Some((target, reason)) = self.fail_at
                && target == step
            {
                return Err(reason.to_owned());
            }
            Ok(())
        }
    }

    impl UsbipStepExecutor for FixtureExecutor {
        fn modprobe(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Modprobe)
        }
        fn acquire_lock(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Lock)
        }
        fn withhold_non_owners(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Withhold)
        }
        fn apply_firewall(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Firewall)
        }
        fn start_backend(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Backend)
        }
        fn bind(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Bind)
        }
        fn start_proxy(&mut self, _: &UsbipBusidPlan) -> Result<(), String> {
            self.dispatch(UsbipBusidStep::Proxy)
        }
    }

    fn synthetic_plan() -> UsbipBusidPlan {
        UsbipBusidPlan {
            busid: "1-2".to_owned(),
            env: "work".to_owned(),
            vm: "yk".to_owned(),
            steps: CANONICAL_STEPS.to_vec(),
        }
    }

    #[test]
    fn canonical_order_is_pinned() {
        let plan = synthetic_plan();
        assert_eq!(
            plan.steps,
            vec![
                UsbipBusidStep::Modprobe,
                UsbipBusidStep::Lock,
                UsbipBusidStep::Withhold,
                UsbipBusidStep::Firewall,
                UsbipBusidStep::Backend,
                UsbipBusidStep::Bind,
                UsbipBusidStep::Proxy,
            ],
        );
    }

    #[test]
    fn stop_order_reverses_bring_up() {
        let plan = synthetic_plan();
        let stop = plan.stop_order();
        assert_eq!(
            stop,
            vec![
                UsbipBusidStep::Proxy,
                UsbipBusidStep::Bind,
                UsbipBusidStep::Backend,
                UsbipBusidStep::Firewall,
                UsbipBusidStep::Withhold,
                UsbipBusidStep::Lock,
                UsbipBusidStep::Modprobe,
            ],
        );
    }

    #[test]
    fn step_as_str_is_stable_kebab_case() {
        assert_eq!(UsbipBusidStep::Modprobe.as_str(), "modprobe");
        assert_eq!(UsbipBusidStep::Lock.as_str(), "lock");
        assert_eq!(UsbipBusidStep::Withhold.as_str(), "withhold");
        assert_eq!(UsbipBusidStep::Firewall.as_str(), "firewall");
        assert_eq!(UsbipBusidStep::Backend.as_str(), "backend");
        assert_eq!(UsbipBusidStep::Bind.as_str(), "bind");
        assert_eq!(UsbipBusidStep::Proxy.as_str(), "proxy");
    }

    #[test]
    fn execute_happy_path_calls_every_step_in_order() {
        let plan = synthetic_plan();
        let mut exec = FixtureExecutor::ok();
        let report = execute_usbip_plan(&plan, &mut exec).expect("happy path succeeds");
        assert!(report.is_ok());
        assert_eq!(report.completed, CANONICAL_STEPS.to_vec());
        assert_eq!(exec.calls, CANONICAL_STEPS.to_vec());
        assert!(report.failed.is_none());
    }

    /// Per-step failure surfaces as a typed error tagged with the
    /// exact step. Execution halts immediately (no later steps run)
    /// and prior steps stay in `report.completed` so the stop-path
    /// reconciler can undo them.
    #[test]
    fn each_step_failure_surfaces_typed_error() {
        for &step in &CANONICAL_STEPS {
            let plan = synthetic_plan();
            let mut exec = FixtureExecutor::failing(step, "fixture: synthetic failure");
            let (report, err) =
                execute_usbip_plan(&plan, &mut exec).expect_err("failure path should return Err");

            match err {
                TypedError::UsbipStepFailed {
                    busid,
                    step: failed_step,
                    reason,
                } => {
                    assert_eq!(busid, "1-2");
                    assert_eq!(failed_step, step);
                    assert!(reason.contains("synthetic failure"), "reason: {reason}");
                }
                other => panic!("expected UsbipStepFailed, got {other:?}"),
            }

            assert_eq!(report.failed.as_ref().map(|(s, _)| *s), Some(step));

            let idx = CANONICAL_STEPS.iter().position(|s| *s == step).unwrap();
            assert_eq!(report.completed, CANONICAL_STEPS[..idx].to_vec());

            assert_eq!(exec.calls.last(), Some(&step));
            assert_eq!(exec.calls.len(), idx + 1, "executor must halt on failure");
        }
    }

    #[test]
    fn typed_error_envelope_carries_exit_code_67() {
        let err = TypedError::UsbipStepFailed {
            busid: "1-2".to_owned(),
            step: UsbipBusidStep::Firewall,
            reason: "nft apply refused".to_owned(),
        };
        let env = err.to_envelope();
        assert_eq!(env.kind, "usbip-step-failed");
        assert_eq!(env.exit_code, 67);
        assert!(env.message.contains("1-2"));
        assert!(env.message.contains("firewall"));
        assert!(env.remediation.contains("modprobe → lock → withhold"));
    }
}
