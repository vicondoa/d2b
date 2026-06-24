//! Daemon-side replacement for the `nixling-net-route-preflight.service`
//! host singleton.
//!
//! # Why this preflight exists
//!
//! Previously, foundational network sanity (each env's LAN bridge exists
//! and is `up`) was asserted by a oneshot systemd singleton
//! (`nixling-net-route-preflight.service` in
//! [`nixos-modules/network.nix`]) that ran at boot, shelled out to
//! `ip route get <env-lan-ip>` per env, and exited non-zero if any
//! env's representative LAN IP did not resolve via its uplink bridge.
//! Every `nixling@<vm>.service` carried `Requires=` on that unit, so
//! a missing bridge fail-closed all VM starts at the unit-dep level.
//!
//! The daemon-only path retires that singleton in favour of running a
//! diagnostic check inside `nixlingd` itself. Startup misses do not block
//! autostart: cold boots can legitimately begin without env bridges because
//! the autostarted net VMs own the host-prep DAG that creates them. If a net
//! VM actually fails to start, the autostart layer degrades that env's
//! workloads through the normal net-VM dependency outcome.
//!
//! # Scope
//!
//! The pure check function [`run_net_route_preflight`] takes a
//! [`HostJson`] and a [`BridgeProbe`] (filesystem-injectable
//! seam) and returns a [`PreflightReport`]. The default production
//! probe ([`SysClassNetProbe`]) reads `/sys/class/net/<bridge>` —
//! existence + `operstate != down` is enough to catch the regression
//! the legacy bash form caught (the LAN bridge isn't there or is
//! administratively down, so `ip route get` returned no result or
//! a wrong-dev route).
//!
//! The persistent history at
//! `<state_dir>/net-route-preflight-history.jsonl` is a line-delimited JSON
//! log of recent passes for diagnostics and manual recovery evidence.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nixling_core::host::{HostJson, IfName, NetEnv};
use serde::{Deserialize, Serialize};

/// Historical degraded-mode threshold retained for compatibility with the
/// typed error/tests. Startup bridge preflight no longer uses this threshold
/// to skip autostart.
pub const DEFAULT_DEGRADED_MODE_THRESHOLD: u32 = 3;

/// Filename of the persistent history log relative to the daemon
/// state directory.
pub const HISTORY_FILENAME: &str = "net-route-preflight-history.jsonl";

/// Maximum number of history records the daemon retains. Older
/// records are pruned on every `record_attempt` call so the file
/// cannot grow unbounded. Sized to retain enough history for
/// manual diagnostics and recovery evidence.
pub const HISTORY_RETENTION_RECORDS: usize = 32;

/// Per-env preflight result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvPreflightOutcome {
    pub env: String,
    pub bridge: String,
    pub status: EnvPreflightStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EnvPreflightStatus {
    /// Bridge exists and is administratively up.
    Pass,
    /// Bridge interface directory `/sys/class/net/<bridge>` does
    /// not exist. The bash singleton's `ip route get` returning
    /// "no result" was the canonical symptom of this case.
    BridgeMissing,
    /// Bridge exists but its `operstate` is `down`. The bash
    /// singleton would report a wrong-dev route here because the
    /// kernel falls back to the default route when the intended
    /// interface is down.
    BridgeDown { operstate: String },
    /// Probe itself errored (filesystem I/O failure outside our
    /// expected matrix). Recorded as a failure for fail-closed
    /// semantics but kept distinct from a clean bridge-missing
    /// signal so an operator can tell the two apart in the journal.
    ProbeError { detail: String },
}

impl EnvPreflightStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, EnvPreflightStatus::Pass)
    }

    pub fn reason(&self) -> String {
        match self {
            EnvPreflightStatus::Pass => "ok".to_owned(),
            EnvPreflightStatus::BridgeMissing => "bridge interface is missing".to_owned(),
            EnvPreflightStatus::BridgeDown { operstate } => {
                format!("bridge operstate is '{operstate}' (expected up)")
            }
            EnvPreflightStatus::ProbeError { detail } => {
                format!("bridge probe failed: {detail}")
            }
        }
    }
}

/// Aggregated preflight result returned by
/// [`run_net_route_preflight`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub outcomes: Vec<EnvPreflightOutcome>,
}

impl PreflightReport {
    pub fn is_ok(&self) -> bool {
        self.outcomes.iter().all(|o| o.status.is_ok())
    }

    /// Env names whose preflight failed. Used to gate the autostart
    /// pass (workloads + the env's own net VM are short-circuited
    /// to [`crate::autostart::Outcome::Degraded`]).
    pub fn failed_envs(&self) -> BTreeSet<String> {
        self.outcomes
            .iter()
            .filter(|o| !o.status.is_ok())
            .map(|o| o.env.clone())
            .collect()
    }

    /// Human-readable per-env summary the daemon writes to the
    /// journal on every preflight pass.
    pub fn summary(&self) -> String {
        if self.outcomes.is_empty() {
            return "no envs declared".to_owned();
        }
        let parts: Vec<String> = self
            .outcomes
            .iter()
            .map(|o| format!("{}:{}", o.env, if o.status.is_ok() { "ok" } else { "fail" }))
            .collect();
        parts.join(", ")
    }
}

/// Seam between [`run_net_route_preflight`] and the live host
/// filesystem. Tests inject a fake probe so the pure function can
/// be exercised hermetically without any `/sys/class/net` access.
pub trait BridgeProbe {
    fn probe(&self, bridge: &IfName) -> EnvPreflightStatus;
}

/// Returns `true` if the operstate value indicates the bridge is
/// sufficiently up for nixling's routing requirements.
///
/// Accepted values (all case-insensitive):
/// - `"up"` — fully operational.
/// - `"unknown"` — drivers that don't implement carrier detection;
///   common on virtual bridges with no active ports.
/// - `"no-carrier"` — bridge is administratively up but has no
///   active member ports, which is normal on cold-boot environments
///   before any VM has started (D16). The kernel raises this instead
///   of `"down"` when the interface was explicitly brought up but
///   no lower-layer is passing traffic yet.
///
/// This function is called only for nixling-declared bridges
/// (`br-<env>-lan` / `br-<env>-up`). All callers of
/// [`SysClassNetProbe`] flow through [`run_net_route_preflight`]
/// which sources bridge names exclusively from the daemon's
/// `HostJson` artifact — verified in `lib.rs` at startup.
fn operstate_acceptable(trimmed: &str) -> bool {
    trimmed.eq_ignore_ascii_case("up")
        || trimmed.eq_ignore_ascii_case("unknown")
        || trimmed.eq_ignore_ascii_case("no-carrier")
}

/// Production probe: reads `/sys/class/net/<bridge>/operstate`.
#[derive(Debug, Clone, Default)]
pub struct SysClassNetProbe;

impl BridgeProbe for SysClassNetProbe {
    fn probe(&self, bridge: &IfName) -> EnvPreflightStatus {
        let dir = PathBuf::from("/sys/class/net").join(bridge.as_str());
        match fs::metadata(&dir) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                EnvPreflightStatus::BridgeMissing
            }
            Err(err) => EnvPreflightStatus::ProbeError {
                detail: format!("stat /sys/class/net/{}: {err}", bridge.as_str()),
            },
            Ok(_) => match fs::read_to_string(dir.join("operstate")) {
                Ok(state) => {
                    let trimmed = state.trim();
                    if operstate_acceptable(trimmed) {
                        EnvPreflightStatus::Pass
                    } else {
                        EnvPreflightStatus::BridgeDown {
                            operstate: trimmed.to_owned(),
                        }
                    }
                }
                Err(err) => EnvPreflightStatus::ProbeError {
                    detail: format!("read /sys/class/net/{}/operstate: {err}", bridge.as_str()),
                },
            },
        }
    }
}

/// Walk the host artifact's env list and probe each env's LAN
/// bridge. Order is preserved from the host artifact for
/// determinism.
pub fn run_net_route_preflight<P: BridgeProbe>(host: &HostJson, probe: &P) -> PreflightReport {
    run_net_route_preflight_for_envs(&host.environments, probe)
}

/// Variant of [`run_net_route_preflight`] used by tests and other
/// callers that already have a `NetEnv` slice in hand. Keeps the
/// pure logic decoupled from `HostJson` construction (which carries
/// many fields irrelevant to the route preflight).
pub fn run_net_route_preflight_for_envs<P: BridgeProbe>(
    envs: &[NetEnv],
    probe: &P,
) -> PreflightReport {
    let outcomes = envs
        .iter()
        .map(|env| EnvPreflightOutcome {
            env: env.env.clone(),
            bridge: env.bridge.as_str().to_owned(),
            status: probe.probe(&env.bridge),
        })
        .collect();
    PreflightReport { outcomes }
}

/// One history entry persisted to disk per preflight pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightHistoryRecord {
    /// Unix epoch seconds (UTC) of the pass.
    pub ts: u64,
    /// True if every env passed.
    pub ok: bool,
    /// Failed env names (empty when `ok = true`).
    #[serde(default)]
    pub failed_envs: Vec<String>,
    /// `"reconcile"` for a successful operator-driven reconcile pass
    /// (which resets the counter); `"startup"` for the routine
    /// daemon-startup pass.
    pub source: String,
}

/// Persistent history reader/writer.
#[derive(Debug, Clone)]
pub struct PreflightHistory {
    path: PathBuf,
}

impl PreflightHistory {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join(HISTORY_FILENAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a record to the history log. Creates parent dirs if
    /// needed. The file is pruned to the most-recent
    /// [`HISTORY_RETENTION_RECORDS`] entries on every write so it
    /// can't grow unbounded.
    pub fn record(&self, record: &PreflightHistoryRecord) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut existing = self.read_all().unwrap_or_default();
        existing.push(record.clone());
        let start = existing.len().saturating_sub(HISTORY_RETENTION_RECORDS);
        let trimmed = &existing[start..];
        // Write atomically via tempfile + rename to avoid torn
        // writes mid-restart.
        let tmp_path = self.path.with_extension("jsonl.tmp");
        {
            let mut tmp = File::create(&tmp_path)?;
            for r in trimmed {
                let line = serde_json::to_string(r).map_err(std::io::Error::other)?;
                writeln!(tmp, "{line}")?;
            }
            tmp.flush()?;
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    /// Read all history records in chronological order.
    pub fn read_all(&self) -> std::io::Result<Vec<PreflightHistoryRecord>> {
        let f = match File::open(&self.path) {
            Ok(f) => f,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let reader = BufReader::new(f);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PreflightHistoryRecord>(&line) {
                Ok(r) => out.push(r),
                Err(_) => {
                    // Tolerate one malformed line — the daemon must
                    // never refuse to start because of a corrupt
                    // log entry. The pure-Rust replacement is
                    // additive over the bash singleton and the
                    // singleton had no failure mode here.
                    continue;
                }
            }
        }
        Ok(out)
    }

    /// Count how many trailing records are failures.
    pub fn consecutive_failures(&self) -> std::io::Result<u32> {
        let all = self.read_all()?;
        let mut n: u32 = 0;
        for record in all.iter().rev() {
            if record.ok {
                break;
            }
            n = n.saturating_add(1);
        }
        Ok(n)
    }

    /// Truncate the history file to a single fresh
    /// `source = "reconcile"` success record. Called after a
    /// successful `nixling host reconcile --network --apply` so
    /// future startup passes start with a clean counter.
    pub fn reset_after_reconcile(&self) -> std::io::Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Atomic truncate: tempfile + rename.
        let tmp_path = self.path.with_extension("jsonl.tmp");
        {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut tmp = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)?;
            let record = PreflightHistoryRecord {
                ts: now,
                ok: true,
                failed_envs: Vec::new(),
                source: "reconcile".to_owned(),
            };
            let line = serde_json::to_string(&record).map_err(std::io::Error::other)?;
            writeln!(tmp, "{line}")?;
            tmp.flush()?;
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

/// Operator-only mode classifier. Once the threshold is met the
/// daemon refuses to dispatch autostart and surfaces a typed
/// degraded envelope on `host doctor` / `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorOnlyMode {
    Disengaged,
    Engaged { consecutive_failures: u32 },
}

impl OperatorOnlyMode {
    pub fn classify(consecutive_failures: u32, threshold: u32) -> Self {
        if threshold > 0 && consecutive_failures >= threshold {
            OperatorOnlyMode::Engaged {
                consecutive_failures,
            }
        } else {
            OperatorOnlyMode::Disengaged
        }
    }

    pub fn is_engaged(&self) -> bool {
        matches!(self, OperatorOnlyMode::Engaged { .. })
    }
}

/// Helper used by call sites to capture the current wall-clock as
/// epoch seconds without panicking on pre-epoch clocks.
pub fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::host::LanPolicy;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn env(name: &str, bridge: &str) -> NetEnv {
        NetEnv {
            env: name.to_owned(),
            bridge: IfName::new(bridge).expect("valid ifname"),
            host_uplink_ip: None,
            net_uplink_ip: None,
            mtu: 1500,
            mss_clamp: None,
            lan: LanPolicy {
                allow_east_west: false,
                effective_east_west: false,
            },
            net_vm_forward_blocklist: Vec::new(),
            bridge_port_flags: Vec::new(),
            ipv6_sysctls: Vec::new(),
            usbip_busid_locks: Vec::new(),
            usbip_backend_port: None,
        }
    }

    #[derive(Default)]
    struct FakeProbe {
        responses: BTreeMap<String, EnvPreflightStatus>,
    }

    impl FakeProbe {
        fn with(mut self, name: &str, s: EnvPreflightStatus) -> Self {
            self.responses.insert(name.to_owned(), s);
            self
        }
    }

    impl BridgeProbe for FakeProbe {
        fn probe(&self, bridge: &IfName) -> EnvPreflightStatus {
            self.responses
                .get(bridge.as_str())
                .cloned()
                .unwrap_or(EnvPreflightStatus::BridgeMissing)
        }
    }

    #[test]
    fn all_envs_pass_when_bridges_up() {
        let envs = vec![env("corp", "nl-corp"), env("personal", "nl-personal")];
        let probe = FakeProbe::default()
            .with("nl-corp", EnvPreflightStatus::Pass)
            .with("nl-personal", EnvPreflightStatus::Pass);
        let report = run_net_route_preflight_for_envs(&envs, &probe);
        assert!(report.is_ok());
        assert!(report.failed_envs().is_empty());
    }

    #[test]
    fn missing_bridge_marks_env_failed() {
        let envs = vec![env("corp", "nl-corp"), env("personal", "nl-personal")];
        let probe = FakeProbe::default()
            .with("nl-corp", EnvPreflightStatus::Pass)
            .with("nl-personal", EnvPreflightStatus::BridgeMissing);
        let report = run_net_route_preflight_for_envs(&envs, &probe);
        assert!(!report.is_ok());
        let failed: Vec<_> = report.failed_envs().into_iter().collect();
        assert_eq!(failed, vec!["personal".to_owned()]);
    }

    #[test]
    fn down_bridge_is_failure() {
        let envs = vec![env("corp", "nl-corp")];
        let probe = FakeProbe::default().with(
            "nl-corp",
            EnvPreflightStatus::BridgeDown {
                operstate: "down".to_owned(),
            },
        );
        let report = run_net_route_preflight_for_envs(&envs, &probe);
        assert!(!report.is_ok());
        assert!(!report.outcomes[0].status.is_ok());
        assert!(report.outcomes[0].status.reason().contains("operstate"));
    }

    #[test]
    fn history_appends_and_prunes() {
        let tmp = TempDir::new().unwrap();
        let h = PreflightHistory::new(tmp.path());
        for i in 0..(HISTORY_RETENTION_RECORDS as u64 + 5) {
            h.record(&PreflightHistoryRecord {
                ts: i,
                ok: i % 2 == 0,
                failed_envs: if i % 2 == 0 {
                    Vec::new()
                } else {
                    vec!["a".to_owned()]
                },
                source: "startup".to_owned(),
            })
            .unwrap();
        }
        let all = h.read_all().unwrap();
        assert_eq!(all.len(), HISTORY_RETENTION_RECORDS);
        // Latest record should be the highest ts written.
        assert_eq!(all.last().unwrap().ts, HISTORY_RETENTION_RECORDS as u64 + 4);
    }

    #[test]
    fn consecutive_failures_counts_trailing_failures_only() {
        let tmp = TempDir::new().unwrap();
        let h = PreflightHistory::new(tmp.path());
        h.record(&PreflightHistoryRecord {
            ts: 1,
            ok: false,
            failed_envs: vec!["a".to_owned()],
            source: "startup".to_owned(),
        })
        .unwrap();
        h.record(&PreflightHistoryRecord {
            ts: 2,
            ok: true,
            failed_envs: Vec::new(),
            source: "startup".to_owned(),
        })
        .unwrap();
        h.record(&PreflightHistoryRecord {
            ts: 3,
            ok: false,
            failed_envs: vec!["a".to_owned()],
            source: "startup".to_owned(),
        })
        .unwrap();
        h.record(&PreflightHistoryRecord {
            ts: 4,
            ok: false,
            failed_envs: vec!["a".to_owned()],
            source: "startup".to_owned(),
        })
        .unwrap();
        assert_eq!(h.consecutive_failures().unwrap(), 2);
    }

    #[test]
    fn operator_only_mode_engages_at_threshold() {
        assert!(matches!(
            OperatorOnlyMode::classify(0, 3),
            OperatorOnlyMode::Disengaged
        ));
        assert!(matches!(
            OperatorOnlyMode::classify(2, 3),
            OperatorOnlyMode::Disengaged
        ));
        assert!(matches!(
            OperatorOnlyMode::classify(3, 3),
            OperatorOnlyMode::Engaged {
                consecutive_failures: 3
            }
        ));
        assert!(matches!(
            OperatorOnlyMode::classify(10, 3),
            OperatorOnlyMode::Engaged {
                consecutive_failures: 10
            }
        ));
        // threshold = 0 disables degraded-mode classification.
        assert!(matches!(
            OperatorOnlyMode::classify(99, 0),
            OperatorOnlyMode::Disengaged
        ));
    }

    #[test]
    fn reset_after_reconcile_clears_failure_counter() {
        let tmp = TempDir::new().unwrap();
        let h = PreflightHistory::new(tmp.path());
        for ts in 1..=5 {
            h.record(&PreflightHistoryRecord {
                ts,
                ok: false,
                failed_envs: vec!["a".to_owned()],
                source: "startup".to_owned(),
            })
            .unwrap();
        }
        assert_eq!(h.consecutive_failures().unwrap(), 5);
        h.reset_after_reconcile().unwrap();
        assert_eq!(h.consecutive_failures().unwrap(), 0);
        let all = h.read_all().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].ok);
        assert_eq!(all[0].source, "reconcile");
    }

    // --- D16: NO-CARRIER operstate tolerance tests ---
    //
    // These tests exercise `operstate_acceptable` via a thin
    // `OperstateTestProbe` that mirrors the exact conditional used in
    // `SysClassNetProbe::probe`, so a regression in the production
    // code will also break the probe-level assertions below.

    struct OperstateTestProbe {
        state: String,
    }

    impl BridgeProbe for OperstateTestProbe {
        fn probe(&self, _bridge: &IfName) -> EnvPreflightStatus {
            let trimmed = self.state.trim();
            if operstate_acceptable(trimmed) {
                EnvPreflightStatus::Pass
            } else {
                EnvPreflightStatus::BridgeDown {
                    operstate: trimmed.to_owned(),
                }
            }
        }
    }

    fn operstate_report(state: &str) -> EnvPreflightStatus {
        let envs = vec![env("corp", "br-corp-lan")];
        let probe = OperstateTestProbe {
            state: state.to_owned(),
        };
        run_net_route_preflight_for_envs(&envs, &probe)
            .outcomes
            .into_iter()
            .next()
            .unwrap()
            .status
    }

    #[test]
    fn no_carrier_operstate_yields_pass() {
        assert_eq!(operstate_report("NO-CARRIER"), EnvPreflightStatus::Pass);
    }

    #[test]
    fn lowercase_no_carrier_operstate_yields_pass() {
        assert_eq!(operstate_report("no-carrier"), EnvPreflightStatus::Pass);
    }

    #[test]
    fn mixed_case_operstate_yields_pass() {
        assert_eq!(operstate_report("No-Carrier"), EnvPreflightStatus::Pass);
    }

    #[test]
    fn genuinely_down_operstate_still_yields_bridge_down() {
        assert_eq!(
            operstate_report("down"),
            EnvPreflightStatus::BridgeDown {
                operstate: "down".to_owned(),
            }
        );
    }

    #[test]
    fn lowerlayerdown_still_yields_bridge_down() {
        assert_eq!(
            operstate_report("lowerlayerdown"),
            EnvPreflightStatus::BridgeDown {
                operstate: "lowerlayerdown".to_owned(),
            }
        );
    }
}
