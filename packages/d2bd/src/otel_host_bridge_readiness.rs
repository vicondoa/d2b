//! Typed readiness gate for the broker-spawned `OtelHostBridge` runner.
//!
//! # Why this gate exists
//!
//! The `RunnerRole::OtelHostBridge` work folded the legacy
//! `d2b-otel-host-bridge.service` host singleton into a
//! broker-`SpawnRunner` lifecycle. The broker can now start the
//! runner per the trusted bundle's intent, and `pidfd_table`
//! tracks liveness — but there was no formal *readiness* signal
//! that the daemon could block on before declaring an
//! observability VM "ready". Without that gate the per-VM start
//! response could report `overall_ok=true` while the host-side
//! OTLP forwarder was still mid-handshake (or had silently failed
//! to bind its vsock host socket), and operators would see
//! mysterious gaps in Grafana/Tempo/Loki for the first few seconds
//! of every obs VM boot.
//!
//! This module promotes that signal into a typed gate. The pure
//! [`evaluate_readiness`] takes a [`ReadinessProbe`] snapshot and
//! returns one of:
//!
//! * [`OtelHostBridgeReadiness::Ready`] — the runner has been
//!   registered in `pidfd_table` AND the obs vsock host socket
//!   exists (the side-effect-free proxy for "socket accept
//!   succeeded + first OTLP forward acknowledged"; the formal
//!   `sd_notify READY=1` channel from broker-spawned runners is a
//!   later phase, see `docs/reference/otel-host-bridge-readiness.md`).
//! * [`OtelHostBridgeReadiness::Pending { elapsed_ms }`] — one or
//!   both signals are missing but the configured deadline has not
//!   yet elapsed; the caller should sleep and retry.
//! * [`OtelHostBridgeReadiness::Failed { reason }`] — the runner
//!   pidfd registration is absent AND the per-VM observability
//!   marker indicates the runner exited; no further polling can
//!   help.
//!
//! # Trigger conditions
//!
//! The side-effecting wrapper [`await_otel_host_bridge_readiness`]
//! is invoked from the VM-start dispatcher AFTER the per-VM
//! process DAG reports `overall_ok=true`, but ONLY when:
//!
//! * `manifest._observability.enabled == true` (the operator has
//!   opted into observability), AND
//! * `request.vm == manifest._observability.vmName` (the VM being
//!   started is the observability VM itself — the OtelHostBridge
//!   relays into it).
//!
//! Workload VMs short-circuit with no I/O. The gate is also
//! skipped when observability is disabled at the manifest level.
//!
//! # Timeout + degraded-mode contract
//!
//! On timeout, the daemon falls back to **degraded mode**: the VM
//! is left running (cloud-hypervisor + virtiofsd + swtpm have
//! already accepted the boot), the response is still successful,
//! but a structured `tracing::warn!` is emitted and the typed
//! error [`crate::typed_error::TypedError::OtelHostBridgeReadinessTimeout`]
//! (exit code 65) is attached as a degraded-mode annotation in the
//! response envelope so operators can detect the condition from
//! `d2b host doctor` and the audit log.
//!
//! Operators who want a strict gate (fail the VM-start request on
//! timeout instead of degrading) can set
//! `D2B_OTEL_BRIDGE_READINESS_STRICT=1`.
//!
//! The default timeout is `30_000ms`; the
//! `D2B_OTEL_BRIDGE_READINESS_TIMEOUT_MS` env var overrides it
//! (parsed as an unsigned integer of milliseconds; invalid values
//! fall back to the default and log a warning).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::typed_error::TypedError;

/// Default deadline the VM-start dispatcher waits for the
/// OtelHostBridge readiness signal before falling back to degraded
/// mode. Overridable via
/// `D2B_OTEL_BRIDGE_READINESS_TIMEOUT_MS`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(30_000);

/// Default poll interval used by the side-effecting wrapper while
/// waiting for the readiness signal. Tests may pass a smaller
/// value via [`ReadinessWaitConfig`].
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Env var overriding [`DEFAULT_TIMEOUT`].
pub const TIMEOUT_ENV: &str = "D2B_OTEL_BRIDGE_READINESS_TIMEOUT_MS";

/// Env var that promotes degraded-mode timeout into a hard failure.
pub const STRICT_ENV: &str = "D2B_OTEL_BRIDGE_READINESS_STRICT";

/// Test-only global config override. Set by tests that need a deterministic
/// readiness config without mutating process env vars (which requires unsafe
/// in Rust 1.81+). In production builds this cell is never initialised.
#[cfg(test)]
static TEST_CONFIG_OVERRIDE: std::sync::OnceLock<
    std::sync::Mutex<Option<ReadinessWaitConfig>>,
> = std::sync::OnceLock::new();

/// Install a test-only readiness config override.  Pass `None` to clear it.
/// The override is consulted by [`ReadinessWaitConfig::for_dispatch`] in
/// `#[cfg(test)]` builds only.
#[cfg(test)]
pub fn set_test_readiness_config(cfg: Option<ReadinessWaitConfig>) {
    let cell = TEST_CONFIG_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None));
    *cell.lock().expect("test readiness config mutex") = cfg;
}

/// Pure verdict from [`evaluate_readiness`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum OtelHostBridgeReadiness {
    /// Both the runner pidfd registration and the obs vsock host
    /// socket are present. The OtelHostBridge is forwarding OTLP.
    Ready,
    /// At least one signal is missing but the deadline has not
    /// elapsed; the caller should sleep and retry.
    Pending {
        /// Milliseconds the gate has been polling for. `0` on the
        /// first iteration.
        elapsed_ms: u128,
    },
    /// Hard refusal: the runner is provably absent (its pidfd was
    /// never registered AND its exit marker says it stopped). No
    /// amount of further polling will help.
    Failed {
        /// Stable, redaction-safe summary suitable for the public
        /// daemon envelope.
        reason: String,
    },
}

/// Snapshot fed into the pure evaluator. The side-effecting
/// wrapper populates this from `pidfd_table` + filesystem stat
/// calls; tests build it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessProbe {
    /// `true` iff `pidfd_table` has a registration for the
    /// `RunnerRole::OtelHostBridge` role keyed at the obs VM.
    pub pidfd_registered: bool,
    /// `true` iff the obs vsock host socket file
    /// (`_observability.obsVsockHostSocket`) exists on disk. The
    /// runner binds + listens on this path; presence means
    /// `accept(2)` is ready.
    pub vsock_host_socket_present: bool,
    /// `true` iff a per-VM "exit" marker file exists, indicating
    /// the runner started but then died. Used to short-circuit
    /// `Pending` into `Failed`. May be `false` even when the
    /// runner has never started.
    pub runner_exit_marker_present: bool,
    /// Milliseconds elapsed since the gate began polling.
    pub elapsed_ms: u128,
    /// Deadline; once `elapsed_ms >= timeout_ms` the verdict is
    /// `Failed { reason = "timeout" }`.
    pub timeout_ms: u128,
}

/// Pure readiness evaluator. No I/O; trivially unit-testable.
pub fn evaluate_readiness(probe: &ReadinessProbe) -> OtelHostBridgeReadiness {
    if probe.pidfd_registered && probe.vsock_host_socket_present {
        return OtelHostBridgeReadiness::Ready;
    }
    if !probe.pidfd_registered && probe.runner_exit_marker_present {
        return OtelHostBridgeReadiness::Failed {
            reason: "runner exited before readiness signal".to_owned(),
        };
    }
    if probe.elapsed_ms >= probe.timeout_ms {
        return OtelHostBridgeReadiness::Failed {
            reason: "timeout".to_owned(),
        };
    }
    OtelHostBridgeReadiness::Pending {
        elapsed_ms: probe.elapsed_ms,
    }
}

/// Read-only inputs the side-effecting wrapper needs to take a
/// single snapshot. Implemented for the production
/// `ServerState`-backed path in `lib.rs`; the tests pass a
/// fake.
pub trait OtelHostBridgeProbeSource {
    /// Is the OtelHostBridge runner currently registered in
    /// `pidfd_table` for the obs VM?
    fn pidfd_registered(&self) -> bool;
    /// Does the obs vsock host socket file exist?
    fn vsock_host_socket_present(&self) -> bool;
    /// Does the runner's exit-marker file exist? (Optional;
    /// implementations that don't yet write an exit marker may
    /// return `false` unconditionally.)
    fn runner_exit_marker_present(&self) -> bool {
        false
    }
}

/// Configuration for [`await_otel_host_bridge_readiness`].
#[derive(Debug, Clone)]
pub struct ReadinessWaitConfig {
    pub timeout: Duration,
    pub poll_interval: Duration,
    pub strict: bool,
}

impl Default for ReadinessWaitConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            strict: false,
        }
    }
}

impl ReadinessWaitConfig {
    /// Build a config from `D2B_OTEL_BRIDGE_READINESS_TIMEOUT_MS`
    /// + `D2B_OTEL_BRIDGE_READINESS_STRICT`. Invalid timeout
    ///   values fall back to [`DEFAULT_TIMEOUT`] with a warning.
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var(TIMEOUT_ENV).ok().as_deref(),
            std::env::var(STRICT_ENV).ok().as_deref(),
        )
    }

    /// The config to use inside `dispatch_broker_vm_start`.
    ///
    /// In production builds this is identical to [`Self::from_env`].
    /// In `#[cfg(test)]` builds it first checks the
    /// [`TEST_CONFIG_OVERRIDE`] cell so tests can inject a deterministic
    /// config without mutating process-global env vars (which requires
    /// `unsafe` in Rust 1.81+).
    pub fn for_dispatch() -> Self {
        #[cfg(test)]
        if let Some(cell) = TEST_CONFIG_OVERRIDE.get() {
            if let Ok(guard) = cell.lock() {
                if let Some(cfg) = guard.clone() {
                    return cfg;
                }
            }
        }
        Self::from_env()
    }

    /// Build a config from already-resolved raw override strings. The
    /// [`Self::from_env`] reader supplies the process-env values; tests
    /// pass values directly so they never mutate process-global env.
    pub fn from_values(timeout_raw: Option<&str>, strict_raw: Option<&str>) -> Self {
        let mut cfg = Self::default();
        if let Some(raw) = timeout_raw {
            match raw.parse::<u64>() {
                Ok(ms) => cfg.timeout = Duration::from_millis(ms),
                Err(error) => tracing::warn!(
                    env = TIMEOUT_ENV,
                    raw = %raw,
                    error = %error,
                    "ignoring invalid timeout env override; using default",
                ),
            }
        }
        cfg.strict = strict_raw.map(|v| v == "1").unwrap_or(false);
        cfg
    }
}

/// Outcome of [`await_otel_host_bridge_readiness`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessWaitOutcome {
    /// Gate closed cleanly; observability is up.
    Ready { elapsed_ms: u128 },
    /// Gate timed out OR runner exited before readiness. The
    /// caller decides whether to surface this as a hard refusal
    /// (strict mode) or a degraded-mode warning (default).
    DegradedTimeout {
        vm: String,
        elapsed_ms: u128,
        reason: String,
    },
}

impl ReadinessWaitOutcome {
    /// Build the typed-error envelope that the strict-mode caller
    /// returns to the client. In degraded mode this is logged but
    /// not surfaced as an error.
    pub fn to_typed_error(&self) -> Option<TypedError> {
        match self {
            Self::Ready { .. } => None,
            Self::DegradedTimeout { vm, elapsed_ms, .. } => {
                Some(TypedError::OtelHostBridgeReadinessTimeout {
                    vm: vm.clone(),
                    elapsed_ms: *elapsed_ms,
                })
            }
        }
    }
}

/// Side-effecting wrapper. Polls `source` every
/// `config.poll_interval` until [`evaluate_readiness`] returns
/// `Ready` or `Failed` (the latter — including timeout — yields
/// `DegradedTimeout` for the caller).
///
/// `now` is injected so tests can fake the clock.
pub fn await_otel_host_bridge_readiness<S, F>(
    vm: &str,
    source: &S,
    config: &ReadinessWaitConfig,
    mut sleep: F,
    started_at: Instant,
) -> ReadinessWaitOutcome
where
    S: OtelHostBridgeProbeSource,
    F: FnMut(Duration),
{
    let timeout_ms = u128::from(config.timeout.as_millis() as u64);
    loop {
        let elapsed_ms = started_at.elapsed().as_millis();
        let probe = ReadinessProbe {
            pidfd_registered: source.pidfd_registered(),
            vsock_host_socket_present: source.vsock_host_socket_present(),
            runner_exit_marker_present: source.runner_exit_marker_present(),
            elapsed_ms,
            timeout_ms,
        };
        match evaluate_readiness(&probe) {
            OtelHostBridgeReadiness::Ready => {
                tracing::info!(
                    vm = %vm,
                    elapsed_ms,
                    "otel-host-bridge readiness gate satisfied",
                );
                return ReadinessWaitOutcome::Ready { elapsed_ms };
            }
            OtelHostBridgeReadiness::Failed { reason } => {
                tracing::warn!(
                    vm = %vm,
                    elapsed_ms,
                    reason = %reason,
                    strict = config.strict,
                    "otel-host-bridge readiness gate did not close; observability is degraded",
                );
                return ReadinessWaitOutcome::DegradedTimeout {
                    vm: vm.to_owned(),
                    elapsed_ms,
                    reason,
                };
            }
            OtelHostBridgeReadiness::Pending { .. } => {
                sleep(config.poll_interval);
            }
        }
    }
}

/// Production [`OtelHostBridgeProbeSource`] implementation backed
/// by the `PidfdTable` + filesystem `stat(2)`. Lives next to the
/// pure module so the integration site in `lib.rs` is a one-liner.
pub struct PidfdAndSocketProbeSource<'a> {
    pub pidfd_table: &'a crate::supervisor::pidfd_table::PidfdTable,
    pub vm: &'a str,
    pub runner_role_id: &'a str,
    pub vsock_host_socket: PathBuf,
    pub exit_marker: Option<PathBuf>,
}

impl OtelHostBridgeProbeSource for PidfdAndSocketProbeSource<'_> {
    fn pidfd_registered(&self) -> bool {
        self.pidfd_table.contains(self.vm, self.runner_role_id)
    }
    fn vsock_host_socket_present(&self) -> bool {
        path_exists(&self.vsock_host_socket)
    }
    fn runner_exit_marker_present(&self) -> bool {
        self.exit_marker
            .as_ref()
            .map(|p| path_exists(p))
            .unwrap_or(false)
    }
}

fn path_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn probe(
        pidfd: bool,
        sock: bool,
        exit: bool,
        elapsed_ms: u128,
        timeout_ms: u128,
    ) -> ReadinessProbe {
        ReadinessProbe {
            pidfd_registered: pidfd,
            vsock_host_socket_present: sock,
            runner_exit_marker_present: exit,
            elapsed_ms,
            timeout_ms,
        }
    }

    #[test]
    fn ready_when_both_signals_present() {
        let p = probe(true, true, false, 0, 30_000);
        assert_eq!(evaluate_readiness(&p), OtelHostBridgeReadiness::Ready);
    }

    #[test]
    fn pending_when_only_pidfd_present() {
        let p = probe(true, false, false, 500, 30_000);
        assert_eq!(
            evaluate_readiness(&p),
            OtelHostBridgeReadiness::Pending { elapsed_ms: 500 }
        );
    }

    #[test]
    fn pending_when_only_socket_present() {
        let p = probe(false, true, false, 250, 30_000);
        assert_eq!(
            evaluate_readiness(&p),
            OtelHostBridgeReadiness::Pending { elapsed_ms: 250 }
        );
    }

    #[test]
    fn failed_when_runner_exit_marker_and_no_pidfd() {
        let p = probe(false, false, true, 50, 30_000);
        assert_eq!(
            evaluate_readiness(&p),
            OtelHostBridgeReadiness::Failed {
                reason: "runner exited before readiness signal".to_owned()
            }
        );
    }

    #[test]
    fn failed_with_timeout_when_deadline_exceeded() {
        let p = probe(false, false, false, 30_000, 30_000);
        assert_eq!(
            evaluate_readiness(&p),
            OtelHostBridgeReadiness::Failed {
                reason: "timeout".to_owned()
            }
        );
    }

    #[test]
    fn failed_takes_precedence_over_timeout_when_marker_present() {
        let p = probe(false, false, true, 60_000, 30_000);
        assert_eq!(
            evaluate_readiness(&p),
            OtelHostBridgeReadiness::Failed {
                reason: "runner exited before readiness signal".to_owned()
            }
        );
    }

    #[test]
    fn ready_takes_precedence_even_at_deadline() {
        // If both signals fire on the very tick the deadline
        // hits, we MUST report Ready, not Timeout.
        let p = probe(true, true, false, 30_000, 30_000);
        assert_eq!(evaluate_readiness(&p), OtelHostBridgeReadiness::Ready);
    }

    /// Test fake: returns canned snapshots from a queue, allowing
    /// the test to drive the wrapper through the Pending → Ready
    /// transition.
    struct FakeSource {
        sequence: Vec<(bool, bool, bool)>,
        index: Cell<usize>,
    }

    impl FakeSource {
        fn new(sequence: Vec<(bool, bool, bool)>) -> Self {
            Self {
                sequence,
                index: Cell::new(0),
            }
        }
        fn step(&self) -> (bool, bool, bool) {
            let i = self.index.get();
            let snap = self.sequence[i.min(self.sequence.len() - 1)];
            self.index.set(i + 1);
            snap
        }
    }

    impl OtelHostBridgeProbeSource for FakeSource {
        fn pidfd_registered(&self) -> bool {
            self.step().0
        }
        fn vsock_host_socket_present(&self) -> bool {
            // step() advanced; peek at last
            let i = self.index.get().saturating_sub(1);
            self.sequence[i.min(self.sequence.len() - 1)].1
        }
        fn runner_exit_marker_present(&self) -> bool {
            let i = self.index.get().saturating_sub(1);
            self.sequence[i.min(self.sequence.len() - 1)].2
        }
    }

    #[test]
    fn wrapper_returns_ready_when_signals_eventually_fire() {
        let source = FakeSource::new(vec![
            (false, false, false),
            (true, false, false),
            (true, true, false),
        ]);
        let cfg = ReadinessWaitConfig {
            timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(1),
            strict: false,
        };
        let outcome =
            await_otel_host_bridge_readiness("obs", &source, &cfg, |_d| {}, Instant::now());
        assert!(matches!(outcome, ReadinessWaitOutcome::Ready { .. }));
        assert!(outcome.to_typed_error().is_none());
    }

    #[test]
    fn wrapper_surfaces_degraded_timeout_when_signals_never_fire() {
        let source = FakeSource::new(vec![(false, false, false)]);
        let cfg = ReadinessWaitConfig {
            timeout: Duration::from_millis(0),
            poll_interval: Duration::from_millis(1),
            strict: false,
        };
        let outcome =
            await_otel_host_bridge_readiness("obs", &source, &cfg, |_d| {}, Instant::now());
        match &outcome {
            ReadinessWaitOutcome::DegradedTimeout { vm, reason, .. } => {
                assert_eq!(vm, "obs");
                assert_eq!(reason, "timeout");
            }
            other => panic!("expected DegradedTimeout, got {other:?}"),
        }
        let err = outcome.to_typed_error().expect("typed error in degraded");
        assert_eq!(err.exit_code(), 65);
        assert_eq!(err.kind(), "otel-host-bridge-readiness-timeout");
    }

    #[test]
    fn wrapper_surfaces_runner_exit_marker_as_degraded() {
        let source = FakeSource::new(vec![(false, false, true)]);
        let cfg = ReadinessWaitConfig {
            timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(1),
            strict: false,
        };
        let outcome =
            await_otel_host_bridge_readiness("obs", &source, &cfg, |_d| {}, Instant::now());
        match &outcome {
            ReadinessWaitOutcome::DegradedTimeout { reason, .. } => {
                assert_eq!(reason, "runner exited before readiness signal");
            }
            other => panic!("expected DegradedTimeout, got {other:?}"),
        }
    }

    #[test]
    fn from_values_falls_back_on_invalid_timeout() {
        let cfg = ReadinessWaitConfig::from_values(Some("not-a-number"), None);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
        assert!(!cfg.strict);
    }

    #[test]
    fn from_values_honors_strict_flag() {
        let cfg = ReadinessWaitConfig::from_values(Some("1234"), Some("1"));
        assert_eq!(cfg.timeout, Duration::from_millis(1234));
        assert!(cfg.strict);
    }

    /// Verify the JSON shape of the typed-error envelope that
    /// `dispatch_broker_vm_start` inserts as `degraded` in the success
    /// response when the OtelHostBridge readiness gate times out in
    /// non-strict mode. This pins the contract between
    /// `ReadinessWaitOutcome::to_typed_error()` and the JSON structure that
    /// operators and tooling (e.g. `d2b host doctor`) parse from the
    /// envelope so the shape can never silently drift.
    #[test]
    fn degraded_timeout_to_envelope_value_has_expected_json_shape() {
        let outcome = ReadinessWaitOutcome::DegradedTimeout {
            vm: "obs".to_owned(),
            elapsed_ms: 5_000,
            reason: "timeout".to_owned(),
        };
        let typed_err = outcome
            .to_typed_error()
            .expect("DegradedTimeout must produce a typed error");
        let envelope = typed_err.to_envelope_value();

        assert_eq!(
            envelope.get("kind").and_then(|v| v.as_str()),
            Some("otel-host-bridge-readiness-timeout"),
            "degraded.kind must be the stable otel-host-bridge-readiness-timeout string"
        );
        // ErrorEnvelope uses camelCase serialization.
        assert_eq!(
            envelope.get("exitCode").and_then(|v| v.as_u64()),
            Some(65),
            "degraded.exitCode must be 65"
        );
        assert!(
            envelope.get("message").and_then(|v| v.as_str()).is_some(),
            "degraded.message must be present"
        );
        assert!(
            envelope.get("remediation").and_then(|v| v.as_str()).is_some(),
            "degraded.remediation must be present"
        );
        // The envelope must NOT contain opaque internal fields like paths or
        // raw reason strings; it is returned to CLI clients as structured data.
        assert!(
            envelope.get("vm").is_none(),
            "degraded envelope must not expose the vm field directly (use message text instead)"
        );
    }

    /// Verify that `ReadinessWaitOutcome::Ready` produces no typed error
    /// (and therefore no `degraded` field in the success envelope).
    #[test]
    fn ready_outcome_produces_no_typed_error() {
        let outcome = ReadinessWaitOutcome::Ready { elapsed_ms: 120 };
        assert!(
            outcome.to_typed_error().is_none(),
            "Ready outcome must not produce a typed error"
        );
    }
}
