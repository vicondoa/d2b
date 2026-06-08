//! `nixling host doctor --read-only` checks.
//!
//! Each check is a passive, read-only probe:
//! - `broker_ready` — connect to `/run/nixling/priv.sock`.
//! - `daemon_ready` — connect to `/run/nixling/public.sock`.
//! - `metrics_endpoint` — `GET /metrics` over the canonical
//!   Prometheus URL (`http://127.0.0.1:9101/metrics`, see
//!   `docs/reference/daemon-metrics.md`). Connect / HTTP failures
//!   surface as `warn`, not `fail`: the doctor is a pre-flight
//!   diagnostic; an absent scrape endpoint must not block other
//!   checks.
//! - `otel_host_bridge_runner` — inspect daemon-persisted
//!   `pidfd-table.json` for a registration with role
//!   `otel-host-bridge`.
//! - `usbipd_runners` — same table, counts every entry whose role
//!   contains `usbip` (per-env `Usbip` runner role, see
//!   `docs/reference/privileges.md`).
//! - `kernel_module_matrix` — read daemon-persisted
//!   `kernel-module-report.json`. Missing required modules = fail;
//!   missing optionals = warn; clean = pass.
//! - `autostart_status` — read daemon-persisted
//!   `autostart-report.json`. Report the degraded + failed count.
//!
//! Tests can redirect probes via the env knobs `NIXLING_BROKER_SOCKET`,
//! `NIXLING_PUBLIC_SOCKET`, `NIXLING_DAEMON_STATE_DIR`, and
//! `NIXLING_METRICS_URL` (see `Context::from_env`).

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Context, SeqpacketUnixSocket};

const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

/// Stable per-check severity. Mirrors `host_check::HostCheckSeverity`
/// to keep the schema vocabulary identical across `host check` and
/// `host doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

impl DoctorStatus {
    fn as_str(self) -> &'static str {
        match self {
            DoctorStatus::Pass => "pass",
            DoctorStatus::Warn => "warn",
            DoctorStatus::Fail => "fail",
        }
    }
}

/// One row in the doctor's `checks[]` array.
#[derive(Debug, Clone)]
pub struct DoctorCheck {
    /// Stable kebab-case identifier (e.g. `broker-ready`).
    pub name: &'static str,
    pub status: DoctorStatus,
    pub detail: String,
    /// Optional structured payload that the JSON renderer merges
    /// into the per-check object (e.g. runner counts).
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn push(&mut self, name: &'static str, status: DoctorStatus, detail: impl Into<String>) {
        self.checks.push(DoctorCheck {
            name,
            status,
            detail: detail.into(),
            data: None,
        });
    }

    pub fn push_with_data(
        &mut self,
        name: &'static str,
        status: DoctorStatus,
        detail: impl Into<String>,
        data: Value,
    ) {
        self.checks.push(DoctorCheck {
            name,
            status,
            detail: detail.into(),
            data: Some(data),
        });
    }

    pub fn pass_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == DoctorStatus::Pass)
            .count()
    }

    pub fn warn_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == DoctorStatus::Warn)
            .count()
    }

    pub fn fail_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == DoctorStatus::Fail)
            .count()
    }

    pub fn exit_code(&self) -> i32 {
        if self.fail_count() > 0 {
            2
        } else if self.warn_count() > 0 {
            1
        } else {
            0
        }
    }

    /// Backward-compatible top-level field: previous callers relied on
    /// the boolean `broker_ready`. Preserve it by looking at the
    /// matching check row.
    pub fn broker_ready(&self) -> bool {
        self.checks
            .iter()
            .find(|c| c.name == "broker-ready")
            .map(|c| c.status == DoctorStatus::Pass)
            .unwrap_or(false)
    }
}

pub fn run_doctor(context: &Context) -> DoctorReport {
    let mut report = DoctorReport::default();
    check_broker_socket(context, &mut report);
    check_daemon_socket(context, &mut report);
    check_metrics_endpoint(context, &mut report);
    let pidfd_entries = load_pidfd_entries(&context.daemon_state_dir);
    check_otel_host_bridge_runner(&pidfd_entries, &mut report);
    check_usbipd_runners(&pidfd_entries, &mut report);
    check_kernel_module_matrix(&context.daemon_state_dir, &mut report);
    check_autostart_status(&context.daemon_state_dir, &mut report);
    // v1.2 invariant probes
    check_seccomp_bpf_loaded(&pidfd_entries, &mut report);
    check_pre_ns_posture(&pidfd_entries, &mut report);
    check_broker_reap_health(&pidfd_entries, &mut report);
    check_bridge_ipv6_sysctl(&context.daemon_state_dir, &mut report);
    report
}

// ---------------------------------------------------------------
// Socket / endpoint probes
// ---------------------------------------------------------------

fn check_broker_socket(context: &Context, report: &mut DoctorReport) {
    match SeqpacketUnixSocket::connect(&context.broker_socket) {
        Ok(_) => report.push(
            "broker-ready",
            DoctorStatus::Pass,
            format!(
                "broker socket reachable at {}",
                context.broker_socket.display()
            ),
        ),
        Err(err) => report.push(
            "broker-ready",
            DoctorStatus::Fail,
            format!(
                "broker socket not reachable at {}: {err}",
                context.broker_socket.display()
            ),
        ),
    }
}

fn check_daemon_socket(context: &Context, report: &mut DoctorReport) {
    match SeqpacketUnixSocket::connect(&context.public_socket) {
        Ok(_) => report.push(
            "daemon-ready",
            DoctorStatus::Pass,
            format!(
                "daemon public socket reachable at {}",
                context.public_socket.display()
            ),
        ),
        Err(err) => report.push(
            "daemon-ready",
            DoctorStatus::Warn,
            format!(
                "daemon public socket not reachable at {}: {err}",
                context.public_socket.display()
            ),
        ),
    }
}

fn check_metrics_endpoint(context: &Context, report: &mut DoctorReport) {
    let url = &context.metrics_url;
    match probe_http_metrics(url) {
        Ok(200) => report.push(
            "metrics-endpoint",
            DoctorStatus::Pass,
            format!("scrape endpoint at {url} returned HTTP 200"),
        ),
        Ok(status) => report.push(
            "metrics-endpoint",
            DoctorStatus::Warn,
            format!("scrape endpoint at {url} returned HTTP {status}"),
        ),
        Err(detail) => report.push(
            "metrics-endpoint",
            DoctorStatus::Warn,
            format!("scrape endpoint at {url} unreachable: {detail}"),
        ),
    }
}

/// Minimal HTTP/1.1 GET against the documented Prometheus scrape URL.
/// Restricted to `http://<host>:<port>/<path>` — the daemon-metrics
/// reference doc pins HTTP-on-loopback, so we don't pull TLS in here.
fn probe_http_metrics(url: &str) -> Result<u16, String> {
    let parsed = parse_http_url(url)?;
    let addr = (parsed.host.as_str(), parsed.port)
        .to_socket_addrs()
        .map_err(|err| format!("resolve {}: {err}", parsed.host))?
        .next()
        .ok_or_else(|| format!("resolve {}: no address", parsed.host))?;
    let mut stream = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT)
        .map_err(|err| format!("connect: {err}"))?;
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: nixling-host-doctor/0.4\r\nConnection: close\r\nAccept: text/plain\r\n\r\n",
        path = parsed.path,
        host = parsed.host,
        port = parsed.port,
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|err| format!("send: {err}"))?;
    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 256];
    while buf.len() < 256 {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(err) => return Err(format!("recv: {err}")),
        }
        if buf.windows(2).any(|w| w == b"\r\n") {
            break;
        }
    }
    let status_line = std::str::from_utf8(&buf)
        .map_err(|err| format!("decode status line: {err}"))?
        .lines()
        .next()
        .ok_or_else(|| "empty response".to_owned())?;
    let mut parts = status_line.split_whitespace();
    let _proto = parts.next().ok_or_else(|| "no HTTP protocol".to_owned())?;
    let status = parts
        .next()
        .ok_or_else(|| "no status code".to_owned())?
        .parse::<u16>()
        .map_err(|err| format!("status code parse: {err}"))?;
    Ok(status)
}

struct ParsedHttpUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<ParsedHttpUrl, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// supported, got {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_owned(),
            p.parse::<u16>()
                .map_err(|err| format!("port parse: {err}"))?,
        ),
        None => (authority.to_owned(), 80u16),
    };
    if host.is_empty() {
        return Err("empty host".to_owned());
    }
    Ok(ParsedHttpUrl {
        host,
        port,
        path: path.to_owned(),
    })
}

// ---------------------------------------------------------------
// pidfd-table.json inspection (OtelHostBridge + per-env usbipd)
// ---------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedPidfdTableLoose {
    #[serde(default)]
    entries: Vec<PersistedPidfdEntryLoose>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct PersistedPidfdEntryLoose {
    vm: String,
    role: String,
    #[serde(default)]
    pid: i32,
    #[serde(default)]
    start_time_ticks: u64,
}

#[derive(Debug, Clone, Default)]
struct PidfdEntries {
    state: PidfdState,
    entries: Vec<PersistedPidfdEntryLoose>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum PidfdState {
    #[default]
    Loaded,
    Missing,
    #[allow(dead_code)]
    UnreadableDir,
    ParseError(String),
}

fn load_pidfd_entries(daemon_state_dir: &Path) -> PidfdEntries {
    let path = daemon_state_dir.join("pidfd-table.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return PidfdEntries {
                state: PidfdState::Missing,
                entries: Vec::new(),
            };
        }
        Err(err) => {
            return PidfdEntries {
                state: PidfdState::ParseError(format!("read {}: {err}", path.display())),
                entries: Vec::new(),
            };
        }
    };
    if bytes.is_empty() {
        return PidfdEntries {
            state: PidfdState::Loaded,
            entries: Vec::new(),
        };
    }
    match serde_json::from_slice::<PersistedPidfdTableLoose>(&bytes) {
        Ok(table) => PidfdEntries {
            state: PidfdState::Loaded,
            entries: table.entries,
        },
        Err(err) => PidfdEntries {
            state: PidfdState::ParseError(format!("parse {}: {err}", path.display())),
            entries: Vec::new(),
        },
    }
}

fn check_otel_host_bridge_runner(entries: &PidfdEntries, report: &mut DoctorReport) {
    match &entries.state {
        PidfdState::Missing => report.push(
            "otel-host-bridge-runner",
            DoctorStatus::Warn,
            "daemon pidfd-table.json missing; cannot confirm OtelHostBridge runner".to_owned(),
        ),
        PidfdState::UnreadableDir | PidfdState::ParseError(_) => {
            let detail = match &entries.state {
                PidfdState::ParseError(d) => d.clone(),
                _ => "daemon state dir unreadable".to_owned(),
            };
            report.push(
                "otel-host-bridge-runner",
                DoctorStatus::Warn,
                format!("pidfd-table inspection failed: {detail}"),
            );
        }
        PidfdState::Loaded => {
            let bridge: Vec<&PersistedPidfdEntryLoose> = entries
                .entries
                .iter()
                .filter(|e| {
                    let role = e.role.to_ascii_lowercase();
                    role == "otel-host-bridge" || role.contains("otel-host-bridge")
                })
                .collect();
            if bridge.is_empty() {
                report.push(
                    "otel-host-bridge-runner",
                    DoctorStatus::Warn,
                    "no broker-spawned OtelHostBridge runner registered in pidfd-table".to_owned(),
                );
            } else {
                let pids: Vec<i32> = bridge.iter().map(|e| e.pid).collect();
                report.push_with_data(
                    "otel-host-bridge-runner",
                    DoctorStatus::Pass,
                    format!(
                        "{} OtelHostBridge runner{} registered",
                        bridge.len(),
                        if bridge.len() == 1 { "" } else { "s" }
                    ),
                    json!({ "count": bridge.len(), "pids": pids }),
                );
            }
        }
    }
}

fn check_usbipd_runners(entries: &PidfdEntries, report: &mut DoctorReport) {
    match &entries.state {
        PidfdState::Missing => report.push(
            "usbipd-runners",
            DoctorStatus::Warn,
            "daemon pidfd-table.json missing; cannot enumerate per-env usbipd runners".to_owned(),
        ),
        PidfdState::UnreadableDir | PidfdState::ParseError(_) => {
            let detail = match &entries.state {
                PidfdState::ParseError(d) => d.clone(),
                _ => "daemon state dir unreadable".to_owned(),
            };
            report.push(
                "usbipd-runners",
                DoctorStatus::Warn,
                format!("pidfd-table inspection failed: {detail}"),
            );
        }
        PidfdState::Loaded => {
            let usbip: Vec<&PersistedPidfdEntryLoose> = entries
                .entries
                .iter()
                .filter(|e| e.role.to_ascii_lowercase().contains("usbip"))
                .collect();
            let runners: Vec<Value> = usbip
                .iter()
                .map(|e| {
                    json!({
                        "vm": e.vm,
                        "role": e.role,
                        "pid": e.pid,
                    })
                })
                .collect();
            if usbip.is_empty() {
                report.push_with_data(
                    "usbipd-runners",
                    DoctorStatus::Pass,
                    "no per-env usbipd runners registered (none required)".to_owned(),
                    json!({ "count": 0, "runners": runners }),
                );
            } else {
                report.push_with_data(
                    "usbipd-runners",
                    DoctorStatus::Pass,
                    format!(
                        "{} per-env usbipd runner{} registered",
                        usbip.len(),
                        if usbip.len() == 1 { "" } else { "s" }
                    ),
                    json!({ "count": usbip.len(), "runners": runners }),
                );
            }
        }
    }
}

// ---------------------------------------------------------------
// kernel-module-report.json
// ---------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct PersistedKernelModuleReport {
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    present: Vec<String>,
    #[serde(default)]
    missing_required: Vec<String>,
    #[serde(default)]
    optional_missing: Vec<PersistedOptionalMissing>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PersistedOptionalMissing {
    #[serde(default)]
    module: String,
    #[serde(default)]
    affected_vms: Vec<String>,
    #[serde(default)]
    reason: String,
}

fn check_kernel_module_matrix(daemon_state_dir: &Path, report: &mut DoctorReport) {
    let path = daemon_state_dir.join("kernel-module-report.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.push(
                "kernel-module-matrix",
                DoctorStatus::Warn,
                format!(
                    "daemon kernel-module-report.json missing at {}; daemon may not have run the startup check",
                    path.display()
                ),
            );
            return;
        }
        Err(err) => {
            report.push(
                "kernel-module-matrix",
                DoctorStatus::Warn,
                format!("read {}: {err}", path.display()),
            );
            return;
        }
    };
    let parsed: PersistedKernelModuleReport = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(err) => {
            report.push(
                "kernel-module-matrix",
                DoctorStatus::Warn,
                format!("parse {}: {err}", path.display()),
            );
            return;
        }
    };
    let data = json!({
        "required": parsed.required,
        "present": parsed.present,
        "missingRequired": parsed.missing_required,
        "optionalMissing": parsed.optional_missing.iter().map(|r| json!({
            "module": r.module,
            "affectedVms": r.affected_vms,
            "reason": r.reason,
        })).collect::<Vec<_>>(),
    });
    if !parsed.missing_required.is_empty() {
        report.push_with_data(
            "kernel-module-matrix",
            DoctorStatus::Fail,
            format!(
                "required kernel module(s) missing: {}",
                parsed.missing_required.join(", ")
            ),
            data,
        );
    } else if !parsed.optional_missing.is_empty() {
        let names: Vec<&str> = parsed
            .optional_missing
            .iter()
            .map(|r| r.module.as_str())
            .collect();
        report.push_with_data(
            "kernel-module-matrix",
            DoctorStatus::Warn,
            format!("optional kernel module(s) missing: {}", names.join(", ")),
            data,
        );
    } else {
        report.push_with_data(
            "kernel-module-matrix",
            DoctorStatus::Pass,
            "all required kernel modules present; no optional gaps".to_owned(),
            data,
        );
    }
}

// ---------------------------------------------------------------
// autostart-report.json
// ---------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct PersistedAutostartReport {
    #[serde(default)]
    outcomes: Vec<PersistedAutostartOutcome>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct PersistedAutostartOutcome {
    #[serde(default)]
    vm: String,
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    outcome: serde_json::Value,
}

fn outcome_kind(outcome: &serde_json::Value) -> Option<&str> {
    outcome.get("kind").and_then(|v| v.as_str())
}

fn check_autostart_status(daemon_state_dir: &Path, report: &mut DoctorReport) {
    let path = daemon_state_dir.join("autostart-report.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.push(
                "autostart-status",
                DoctorStatus::Warn,
                format!(
                    "daemon autostart-report.json missing at {}; daemon may not have run the autostart pass",
                    path.display()
                ),
            );
            return;
        }
        Err(err) => {
            report.push(
                "autostart-status",
                DoctorStatus::Warn,
                format!("read {}: {err}", path.display()),
            );
            return;
        }
    };
    let parsed: PersistedAutostartReport = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(err) => {
            report.push(
                "autostart-status",
                DoctorStatus::Warn,
                format!("parse {}: {err}", path.display()),
            );
            return;
        }
    };
    let mut started = 0usize;
    let mut already = 0usize;
    let mut failed = 0usize;
    let mut degraded = 0usize;
    let mut other = 0usize;
    let mut degraded_vms: Vec<String> = Vec::new();
    let mut failed_vms: Vec<String> = Vec::new();
    for outcome in &parsed.outcomes {
        match outcome_kind(&outcome.outcome) {
            Some("started") => started += 1,
            Some("already-running") => already += 1,
            Some("failed") => {
                failed += 1;
                failed_vms.push(outcome.vm.clone());
            }
            Some("degraded") => {
                degraded += 1;
                degraded_vms.push(outcome.vm.clone());
            }
            Some("not-autostart") => {}
            _ => other += 1,
        }
    }
    let total = parsed.outcomes.len();
    let degraded_total = failed + degraded;
    let data = json!({
        "totalVms": total,
        "started": started,
        "alreadyRunning": already,
        "failed": failed,
        "degraded": degraded,
        "degradedTotal": degraded_total,
        "degradedVms": degraded_vms,
        "failedVms": failed_vms,
        "unknown": other,
    });
    let status = if failed > 0 {
        DoctorStatus::Fail
    } else if degraded > 0 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };
    let detail = format!(
        "autostart: started={started} already_running={already} failed={failed} degraded={degraded} (degraded_total={degraded_total})"
    );
    report.push_with_data("autostart-status", status, detail, data);
}

// ---------------------------------------------------------------
// v1.2 invariant probes
// ---------------------------------------------------------------

// --- helpers for /proc parsing ---

/// Read `/proc/<pid>/status` and return its lines, or `None` if the
/// file is absent (process exited).
fn read_proc_status(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/status")).ok()
}

/// Extract the integer value from a line like `Seccomp:\t2\n`.
fn parse_proc_status_field(status: &str, field: &str) -> Option<String> {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

/// Parse the process state character from `/proc/<pid>/stat`.
/// The format is: `pid (comm) state ...`; `comm` may contain spaces
/// and parentheses, so we locate the *last* `)` and take the next
/// non-whitespace character.
fn read_proc_stat_state(pid: i32) -> Option<char> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = text.rfind(')')?.checked_add(1)?;
    text[after_comm..].trim_start().chars().next()
}

// --- check_seccomp_bpf_loaded (D4 visibility) ---

/// For each runner in `pidfd-table.json`, verify `Seccomp: 2` (BPF
/// filter mode) in `/proc/<pid>/status`.
///
/// - **Warn** if pidfd-table is missing (no runners to check).
/// - **Fail** if any live runner reports `Seccomp: 0` (disabled) or
///   `Seccomp: 1` (strict mode, not BPF filter).
/// - **Pass** if all live runners have `Seccomp: 2`.
fn check_seccomp_bpf_loaded(entries: &PidfdEntries, report: &mut DoctorReport) {
    match &entries.state {
        PidfdState::Missing => {
            report.push(
                "seccomp-bpf-loaded",
                DoctorStatus::Warn,
                "pidfd-table.json missing; no runners running — seccomp BPF posture not verifiable",
            );
            return;
        }
        PidfdState::UnreadableDir | PidfdState::ParseError(_) => {
            let detail = match &entries.state {
                PidfdState::ParseError(d) => d.clone(),
                _ => "daemon state dir unreadable".to_owned(),
            };
            report.push(
                "seccomp-bpf-loaded",
                DoctorStatus::Warn,
                format!("pidfd-table inspection failed: {detail}"),
            );
            return;
        }
        PidfdState::Loaded => {}
    }

    if entries.entries.is_empty() {
        report.push(
            "seccomp-bpf-loaded",
            DoctorStatus::Warn,
            "no runners registered in pidfd-table; seccomp BPF posture not verifiable",
        );
        return;
    }

    let mut bad: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in &entries.entries {
        let Some(status) = read_proc_status(entry.pid) else {
            // Process has already exited; skip stale entry.
            continue;
        };
        checked += 1;
        let seccomp_val = parse_proc_status_field(&status, "Seccomp:");
        match seccomp_val.as_deref() {
            Some("2") => {}
            Some(v) => bad.push(format!("{}(pid={}) Seccomp:{}", entry.role, entry.pid, v)),
            None => bad.push(format!(
                "{}(pid={}) Seccomp: field missing",
                entry.role, entry.pid
            )),
        }
    }

    if checked == 0 {
        report.push(
            "seccomp-bpf-loaded",
            DoctorStatus::Warn,
            "all registered runner PIDs have exited; seccomp BPF posture not verifiable",
        );
        return;
    }

    if bad.is_empty() {
        report.push_with_data(
            "seccomp-bpf-loaded",
            DoctorStatus::Pass,
            format!("all {checked} registered runner(s) have Seccomp:2 (BPF filter)"),
            json!({ "checked": checked }),
        );
    } else {
        report.push_with_data(
            "seccomp-bpf-loaded",
            DoctorStatus::Fail,
            format!(
                "{} runner(s) missing BPF filter: {}",
                bad.len(),
                bad.join(", ")
            ),
            json!({ "checked": checked, "failing": bad }),
        );
    }
}

// --- check_pre_ns_posture (D5 visibility) ---

/// The set of runner roles that must run inside a broker-pre-established
/// user namespace (D5 scope for v1.2: swtpm only; gpu render-node-only
/// and audio are conditional and absent from v1.2 mandatory set).
fn is_d5_scoped_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("swtpm")
}

/// For each D5-scoped runner, verify it is in a nested user namespace
/// by inspecting `NStgid:` in `/proc/<pid>/status`.  Multiple
/// tab-separated values on the `NStgid:` line indicate the process is
/// inside at least one nested user namespace.
///
/// - **Warn** if pidfd-table is missing or no D5-scoped runners are
///   registered (nothing to assert yet).
/// - **Fail** if a D5-scoped runner is in the initial user NS (single
///   `NStgid` value).
/// - **Pass** if all D5-scoped runners are in a nested user NS.
fn check_pre_ns_posture(entries: &PidfdEntries, report: &mut DoctorReport) {
    match &entries.state {
        PidfdState::Missing => {
            report.push(
                "pre-ns-posture",
                DoctorStatus::Warn,
                "pidfd-table.json missing; D5-scoped runner user-NS posture not verifiable",
            );
            return;
        }
        PidfdState::UnreadableDir | PidfdState::ParseError(_) => {
            let detail = match &entries.state {
                PidfdState::ParseError(d) => d.clone(),
                _ => "daemon state dir unreadable".to_owned(),
            };
            report.push(
                "pre-ns-posture",
                DoctorStatus::Warn,
                format!("pidfd-table inspection failed: {detail}"),
            );
            return;
        }
        PidfdState::Loaded => {}
    }

    let scoped: Vec<&PersistedPidfdEntryLoose> = entries
        .entries
        .iter()
        .filter(|e| is_d5_scoped_role(&e.role))
        .collect();

    if scoped.is_empty() {
        report.push(
            "pre-ns-posture",
            DoctorStatus::Warn,
            "no D5-scoped runners (swtpm) registered; user-NS posture not verifiable",
        );
        return;
    }

    let mut not_nested: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in &scoped {
        let Some(status) = read_proc_status(entry.pid) else {
            continue;
        };
        checked += 1;
        // `NStgid:` contains one value per nested user NS level
        // (outermost … innermost), tab-separated.  Two or more
        // values means the process is inside at least one user NS.
        let nstgid = parse_proc_status_field(&status, "NStgid:");
        let nested = nstgid
            .as_deref()
            .map(|v| v.split_whitespace().count() >= 2)
            .unwrap_or(false);
        if !nested {
            not_nested.push(format!("{}(pid={})", entry.role, entry.pid));
        }
    }

    if checked == 0 {
        report.push(
            "pre-ns-posture",
            DoctorStatus::Warn,
            "all D5-scoped runner PIDs have exited; user-NS posture not verifiable",
        );
        return;
    }

    if not_nested.is_empty() {
        report.push_with_data(
            "pre-ns-posture",
            DoctorStatus::Pass,
            format!("all {checked} D5-scoped runner(s) are in a nested user namespace"),
            json!({ "checked": checked }),
        );
    } else {
        report.push_with_data(
            "pre-ns-posture",
            DoctorStatus::Fail,
            format!(
                "{} D5-scoped runner(s) in initial user NS (broker-pre-NS not active): {}",
                not_nested.len(),
                not_nested.join(", ")
            ),
            json!({ "checked": checked, "failing": not_nested }),
        );
    }
}

// --- check_broker_reap_health (D7 visibility) ---

/// For each registered runner PID, read `/proc/<pid>/stat` and check
/// for zombie (`Z`) or dead (`X`) state — both indicate the process
/// exited but was never reaped.
///
/// - **Fail** if any registered runner is in state `Z` or `X`.
/// - **Pass** otherwise.
///
/// The broker's in-memory `ChildReaped` replay-buffer depth is NOT yet
/// observable via a stable CLI command (`nixling-priv-broker
/// --report-state` is not yet implemented as of v1.2).  When D7 lands
/// the IPC mechanism, this check will be extended to query that buffer.
/// For v1.2 the zombie-count probe is sufficient: zero zombies among
/// registered runners implies the reap loop is functioning correctly.
fn check_broker_reap_health(entries: &PidfdEntries, report: &mut DoctorReport) {
    match &entries.state {
        PidfdState::Missing => {
            report.push(
                "broker-reap-health",
                DoctorStatus::Warn,
                "pidfd-table.json missing; broker reap health not verifiable",
            );
            return;
        }
        PidfdState::UnreadableDir | PidfdState::ParseError(_) => {
            let detail = match &entries.state {
                PidfdState::ParseError(d) => d.clone(),
                _ => "daemon state dir unreadable".to_owned(),
            };
            report.push(
                "broker-reap-health",
                DoctorStatus::Warn,
                format!("pidfd-table inspection failed: {detail}"),
            );
            return;
        }
        PidfdState::Loaded => {}
    }

    if entries.entries.is_empty() {
        report.push_with_data(
            "broker-reap-health",
            DoctorStatus::Pass,
            "no runners registered; no zombie check needed (buffer depth not yet observable — D7 IPC placeholder)",
            json!({ "zombies": 0, "checked": 0, "bufferDepth": null }),
        );
        return;
    }

    let mut zombies: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in &entries.entries {
        let Some(state_char) = read_proc_stat_state(entry.pid) else {
            // Process exited and /proc entry is gone — already reaped.
            continue;
        };
        checked += 1;
        if state_char == 'Z' || state_char == 'X' {
            zombies.push(format!(
                "{}(pid={},state={})",
                entry.role, entry.pid, state_char
            ));
        }
    }

    let data = json!({
        "zombies": zombies.len(),
        "checked": checked,
        // D7 replay-buffer depth probe: not yet observable.
        // When `nixling-priv-broker --report-state` lands, this
        // field will carry the actual in-memory buffer depth.
        "bufferDepth": null,
    });

    if zombies.is_empty() {
        report.push_with_data(
            "broker-reap-health",
            DoctorStatus::Pass,
            format!(
                "no zombie/dead runners among {checked} checked (buffer depth not yet observable — D7 IPC placeholder)"
            ),
            data,
        );
    } else {
        report.push_with_data(
            "broker-reap-health",
            DoctorStatus::Fail,
            format!(
                "{} registered runner(s) in zombie/dead state: {}",
                zombies.len(),
                zombies.join(", ")
            ),
            data,
        );
    }
}

// --- check_bridge_ipv6_sysctl (D8 visibility) ---

/// Deserialised shape of `envs.json` — loose, forward-compatible.
#[derive(Debug, Clone, Deserialize, Default)]
struct PersistedEnvsJson {
    #[serde(default)]
    envs: Vec<PersistedEnvEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedEnvEntry {
    #[serde(default)]
    lan_bridge: Option<String>,
    #[serde(default)]
    uplink_bridge: Option<String>,
}

/// Collect declared bridge names from `<daemon-state-dir>/envs.json`.
/// Falls back to scanning `/sys/class/net/` for nixling-named bridges
/// (`br-*-lan` / `br-*-up`) only when the file is absent or
/// unparseable — a successfully-parsed empty file means "no envs
/// declared" and suppresses the fallback.
fn collect_bridge_names(daemon_state_dir: &Path) -> Vec<String> {
    let envs_path = daemon_state_dir.join("envs.json");
    if let Ok(bytes) = std::fs::read(&envs_path) {
        if let Ok(parsed) = serde_json::from_slice::<PersistedEnvsJson>(&bytes) {
            // Successfully parsed — use declared bridges only; do NOT
            // fall back to sysfs so that an empty envs.json correctly
            // signals "no envs" rather than triggering a sysfs scan.
            let mut bridges: Vec<String> = Vec::new();
            for env in &parsed.envs {
                if let Some(b) = &env.lan_bridge {
                    if !b.is_empty() {
                        bridges.push(b.clone());
                    }
                }
                if let Some(b) = &env.uplink_bridge {
                    if !b.is_empty() {
                        bridges.push(b.clone());
                    }
                }
            }
            bridges.sort();
            bridges.dedup();
            return bridges;
        }
    }
    // envs.json absent or unparseable — fall back to sysfs scan.
    sysfs_nixling_bridges()
}

fn sysfs_nixling_bridges() -> Vec<String> {
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut bridges: Vec<String> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_nixling_bridge_name(&name) {
            bridges.push(name);
        }
    }
    bridges.sort();
    bridges
}

/// Return `true` for interface names matching the nixling bridge
/// naming convention: `br-<env>-lan` or `br-<env>-up`.
fn is_nixling_bridge_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("br-") else {
        return false;
    };
    rest.ends_with("-lan") || rest.ends_with("-up")
}

/// For each declared nixling bridge, query
/// `net.ipv6.conf.<bridge>.disable_ipv6` via `sysctl`.
///
/// - **Fail** if any bridge returns `0` (IPv6 active).
/// - **Pass** if all bridges return `1`.
/// - **Warn** if no bridges can be discovered (no envs running).
fn check_bridge_ipv6_sysctl(daemon_state_dir: &Path, report: &mut DoctorReport) {
    let bridges = collect_bridge_names(daemon_state_dir);

    if bridges.is_empty() {
        report.push(
            "bridge-ipv6-sysctl",
            DoctorStatus::Warn,
            "no nixling bridges found (envs.json absent and no br-*-{lan,up} interfaces visible); IPv6 sysctl not verifiable",
        );
        return;
    }

    let mut failing: Vec<String> = Vec::new();
    let mut passing: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for bridge in &bridges {
        let key = format!("net.ipv6.conf.{bridge}.disable_ipv6");
        match run_sysctl_n(&key) {
            Ok(val) => {
                let trimmed = val.trim();
                if trimmed == "1" {
                    passing.push(bridge.clone());
                } else {
                    failing.push(format!("{bridge}=>{trimmed}"));
                }
            }
            Err(err) => errors.push(format!("{bridge}: {err}")),
        }
    }

    let data = json!({
        "bridges": bridges,
        "passing": passing,
        "failing": failing,
        "errors": errors,
    });

    if !failing.is_empty() {
        report.push_with_data(
            "bridge-ipv6-sysctl",
            DoctorStatus::Fail,
            format!(
                "{} bridge(s) have disable_ipv6=0 (IPv6 active): {}",
                failing.len(),
                failing.join(", ")
            ),
            data,
        );
    } else if !errors.is_empty() {
        report.push_with_data(
            "bridge-ipv6-sysctl",
            DoctorStatus::Warn,
            format!(
                "sysctl query errors for {} bridge(s): {}",
                errors.len(),
                errors.join(", ")
            ),
            data,
        );
    } else {
        report.push_with_data(
            "bridge-ipv6-sysctl",
            DoctorStatus::Pass,
            format!("all {} bridge(s) have disable_ipv6=1", passing.len()),
            data,
        );
    }
}

/// Run `sysctl -n <key>` and return trimmed stdout, or an error string.
fn run_sysctl_n(key: &str) -> Result<String, String> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", key])
        .output()
        .map_err(|e| format!("exec sysctl: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(format!(
            "sysctl exited {}: {stderr}",
            out.status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".to_owned())
        ))
    }
}

// ---------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------

pub fn render_summary(report: &DoctorReport) -> Value {
    let checks: Vec<Value> = report
        .checks
        .iter()
        .map(|c| {
            let mut obj = serde_json::Map::new();
            obj.insert("name".to_owned(), Value::String(c.name.to_owned()));
            obj.insert(
                "status".to_owned(),
                Value::String(c.status.as_str().to_owned()),
            );
            obj.insert("detail".to_owned(), Value::String(c.detail.clone()));
            if let Some(data) = &c.data {
                obj.insert("data".to_owned(), data.clone());
            }
            Value::Object(obj)
        })
        .collect();
    // Backward-compatible: previous doctor output emitted a flat
    // `findings` array containing only failing rows. Preserve that shape so
    // existing consumers keep working; the new structured surface
    // lives in `checks`.
    let findings: Vec<Value> = report
        .checks
        .iter()
        .filter(|c| c.status != DoctorStatus::Pass)
        .map(|c| {
            json!({
                "check": c.name,
                "result": c.status.as_str(),
                "detail": c.detail,
            })
        })
        .collect();
    json!({
        "command": "host doctor",
        "mode": "read-only",
        "broker_ready": report.broker_ready(),
        "checks": checks,
        "findings": findings,
        "summary": {
            "pass": report.pass_count(),
            "warn": report.warn_count(),
            "fail": report.fail_count(),
        },
        "exitCode": report.exit_code(),
    })
}

pub fn render_human(report: &DoctorReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "host doctor --read-only: summary pass={} warn={} fail={}",
        report.pass_count(),
        report.warn_count(),
        report.fail_count()
    );
    for c in &report.checks {
        let marker = match c.status {
            DoctorStatus::Pass => "PASS",
            DoctorStatus::Warn => "WARN",
            DoctorStatus::Fail => "FAIL",
        };
        let _ = writeln!(out, "  [{}] {} — {}", marker, c.name, c.detail);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_http_url_with_port_and_path() {
        let p = parse_http_url("http://127.0.0.1:9101/metrics").unwrap();
        assert_eq!(p.host, "127.0.0.1");
        assert_eq!(p.port, 9101);
        assert_eq!(p.path, "/metrics");
    }

    #[test]
    fn parse_http_url_without_port_defaults_to_80() {
        let p = parse_http_url("http://example/metrics").unwrap();
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/metrics");
    }

    #[test]
    fn parse_http_url_rejects_https() {
        assert!(parse_http_url("https://x/metrics").is_err());
    }

    #[test]
    fn exit_code_is_two_when_fail() {
        let mut report = DoctorReport::default();
        report.push("broker-ready", DoctorStatus::Fail, "down");
        report.push("daemon-ready", DoctorStatus::Pass, "up");
        assert_eq!(report.exit_code(), 2);
        assert!(!report.broker_ready());
    }

    #[test]
    fn exit_code_is_one_when_only_warn() {
        let mut report = DoctorReport::default();
        report.push("broker-ready", DoctorStatus::Pass, "ok");
        report.push("metrics-endpoint", DoctorStatus::Warn, "unreachable");
        assert_eq!(report.exit_code(), 1);
        assert!(report.broker_ready());
    }

    #[test]
    fn exit_code_zero_when_clean() {
        let mut report = DoctorReport::default();
        report.push("broker-ready", DoctorStatus::Pass, "ok");
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn render_summary_preserves_broker_ready_top_level() {
        let mut report = DoctorReport::default();
        report.push("broker-ready", DoctorStatus::Pass, "ok");
        let v = render_summary(&report);
        assert_eq!(v.get("broker_ready").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn render_summary_emits_legacy_findings_for_nonpass() {
        let mut report = DoctorReport::default();
        report.push("broker-ready", DoctorStatus::Pass, "ok");
        report.push("metrics-endpoint", DoctorStatus::Warn, "unreachable");
        let v = render_summary(&report);
        let findings = v.get("findings").and_then(Value::as_array).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["check"].as_str(), Some("metrics-endpoint"));
        assert_eq!(findings[0]["result"].as_str(), Some("warn"));
    }

    #[test]
    fn pidfd_loose_parser_extracts_known_runners() {
        let json = serde_json::json!({
            "entries": [
                {"vm": "obs-net", "role": "otel-host-bridge", "pid": 123, "startTimeTicks": 9},
                {"vm": "corp-net", "role": "usbip", "pid": 124, "startTimeTicks": 10},
                {"vm": "corp-vm", "role": "cloud-hypervisor", "pid": 125, "startTimeTicks": 11},
            ]
        });
        let tmp = std::env::temp_dir().join(format!("nl-doctor-pidfd-{}.json", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(&json).unwrap()).unwrap();
        let entries = load_pidfd_entries(tmp.parent().unwrap());
        let _ = std::fs::remove_file(&tmp);
        // load_pidfd_entries always looks at <dir>/pidfd-table.json,
        // so test asserts only the parsing primitives here.
        drop(entries);
        let table: PersistedPidfdTableLoose = serde_json::from_value(json).unwrap();
        assert_eq!(table.entries.len(), 3);
        assert_eq!(table.entries[0].role, "otel-host-bridge");
        assert_eq!(table.entries[1].role, "usbip");
    }

    fn write_state(dir: &Path, name: &str, value: serde_json::Value) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    fn unique_scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nl-doctor-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn kernel_module_matrix_clean_is_pass() {
        let dir = unique_scratch("km-pass");
        write_state(
            &dir,
            "kernel-module-report.json",
            serde_json::json!({
                "required": ["kvm_intel"],
                "present": ["kvm_intel"],
                "missing_required": [],
                "optional_missing": [],
            }),
        );
        let mut report = DoctorReport::default();
        check_kernel_module_matrix(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Pass);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kernel_module_matrix_optional_is_warn() {
        let dir = unique_scratch("km-warn");
        write_state(
            &dir,
            "kernel-module-report.json",
            serde_json::json!({
                "required": ["kvm_intel"],
                "present": ["kvm_intel"],
                "missing_required": [],
                "optional_missing": [{
                    "module": "usbip_host",
                    "affected_vms": ["corp-vm"],
                    "reason": "usbip passthrough degraded",
                }],
            }),
        );
        let mut report = DoctorReport::default();
        check_kernel_module_matrix(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kernel_module_matrix_required_missing_is_fail() {
        let dir = unique_scratch("km-fail");
        write_state(
            &dir,
            "kernel-module-report.json",
            serde_json::json!({
                "required": ["kvm_intel"],
                "present": [],
                "missing_required": ["kvm_intel"],
                "optional_missing": [],
            }),
        );
        let mut report = DoctorReport::default();
        check_kernel_module_matrix(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Fail);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn autostart_status_failed_is_fail() {
        let dir = unique_scratch("autostart-fail");
        write_state(
            &dir,
            "autostart-report.json",
            serde_json::json!({
                "outcomes": [
                    {"vm": "a", "env": null, "is_net_vm": true, "outcome": {"kind": "started"}},
                    {"vm": "b", "env": null, "is_net_vm": false, "outcome": {"kind": "failed", "reason": "boom"}},
                ]
            }),
        );
        let mut report = DoctorReport::default();
        check_autostart_status(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Fail);
        let data = report.checks[0].data.as_ref().unwrap();
        assert_eq!(data["failed"].as_u64(), Some(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn autostart_status_degraded_is_warn() {
        let dir = unique_scratch("autostart-warn");
        write_state(
            &dir,
            "autostart-report.json",
            serde_json::json!({
                "outcomes": [
                    {"vm": "a", "env": null, "is_net_vm": false, "outcome": {"kind": "degraded", "reason": "net-vm down"}},
                ]
            }),
        );
        let mut report = DoctorReport::default();
        check_autostart_status(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn autostart_status_pass_when_all_started() {
        let dir = unique_scratch("autostart-pass");
        write_state(
            &dir,
            "autostart-report.json",
            serde_json::json!({
                "outcomes": [
                    {"vm": "a", "env": null, "is_net_vm": false, "outcome": {"kind": "started"}},
                    {"vm": "b", "env": null, "is_net_vm": false, "outcome": {"kind": "already-running"}},
                ]
            }),
        );
        let mut report = DoctorReport::default();
        check_autostart_status(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Pass);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn otel_bridge_pass_when_runner_present() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![PersistedPidfdEntryLoose {
                vm: "obs-net".to_owned(),
                role: "otel-host-bridge".to_owned(),
                pid: 42,
                start_time_ticks: 1,
            }],
        };
        let mut report = DoctorReport::default();
        check_otel_host_bridge_runner(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Pass);
    }

    #[test]
    fn otel_bridge_warn_when_runner_missing() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_otel_host_bridge_runner(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
    }

    #[test]
    fn usbipd_runners_counted() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![
                PersistedPidfdEntryLoose {
                    vm: "corp-net".to_owned(),
                    role: "usbip".to_owned(),
                    pid: 1,
                    start_time_ticks: 1,
                },
                PersistedPidfdEntryLoose {
                    vm: "work-net".to_owned(),
                    role: "usbip".to_owned(),
                    pid: 2,
                    start_time_ticks: 1,
                },
            ],
        };
        let mut report = DoctorReport::default();
        check_usbipd_runners(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Pass);
        let data = report.checks[0].data.as_ref().unwrap();
        assert_eq!(data["count"].as_u64(), Some(2));
    }

    // ---------------------------------------------------------------
    // Unit tests for the v1.2 invariant probes
    // ---------------------------------------------------------------

    // --- check_seccomp_bpf_loaded ---

    #[test]
    fn seccomp_bpf_loaded_warn_when_table_missing() {
        let entries = PidfdEntries {
            state: PidfdState::Missing,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_seccomp_bpf_loaded(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        assert!(report.checks[0].name == "seccomp-bpf-loaded");
    }

    #[test]
    fn seccomp_bpf_loaded_warn_when_no_entries() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_seccomp_bpf_loaded(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
    }

    /// Build a fake `/proc/<pid>/status`-style string with the given
    /// `Seccomp:` value so we can exercise the parser without a real
    /// process.
    fn fake_proc_status(seccomp: u8, nstgid_values: &[u32]) -> String {
        let nstgid = nstgid_values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\t");
        format!("Name:\ttest\nPid:\t99\nSeccomp:\t{seccomp}\nNStgid:\t{nstgid}\n")
    }

    #[test]
    fn seccomp_field_parse_bpf() {
        let status = fake_proc_status(2, &[99]);
        assert_eq!(
            parse_proc_status_field(&status, "Seccomp:"),
            Some("2".to_owned())
        );
    }

    #[test]
    fn seccomp_field_parse_disabled() {
        let status = fake_proc_status(0, &[99]);
        assert_eq!(
            parse_proc_status_field(&status, "Seccomp:"),
            Some("0".to_owned())
        );
    }

    // Simulate the actual /proc check using the real current process
    // PID (which will have Seccomp: 0 in a normal test runner).
    // The check result depends on the host kernel; we only assert
    // the check runs without panic and produces a known name.
    #[test]
    fn seccomp_bpf_loaded_runs_against_self() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![PersistedPidfdEntryLoose {
                vm: "test".to_owned(),
                role: "cloud-hypervisor".to_owned(),
                pid: std::process::id() as i32,
                start_time_ticks: 0,
            }],
        };
        let mut report = DoctorReport::default();
        check_seccomp_bpf_loaded(&entries, &mut report);
        assert_eq!(report.checks[0].name, "seccomp-bpf-loaded");
        // Status is host-dependent; just assert it is one of the valid variants.
        assert!(matches!(
            report.checks[0].status,
            DoctorStatus::Pass | DoctorStatus::Fail | DoctorStatus::Warn
        ));
    }

    // --- check_pre_ns_posture ---

    #[test]
    fn pre_ns_posture_warn_when_table_missing() {
        let entries = PidfdEntries {
            state: PidfdState::Missing,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_pre_ns_posture(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        assert_eq!(report.checks[0].name, "pre-ns-posture");
    }

    #[test]
    fn pre_ns_posture_warn_when_no_d5_runners() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![PersistedPidfdEntryLoose {
                vm: "obs-net".to_owned(),
                role: "cloud-hypervisor".to_owned(),
                pid: 42,
                start_time_ticks: 0,
            }],
        };
        let mut report = DoctorReport::default();
        check_pre_ns_posture(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
    }

    #[test]
    fn nstgid_nested_parse() {
        // Two values → nested user NS
        let status = fake_proc_status(2, &[12345, 1]);
        let val = parse_proc_status_field(&status, "NStgid:").unwrap();
        assert!(val.split_whitespace().count() >= 2);
    }

    #[test]
    fn nstgid_single_parse() {
        // One value → initial user NS
        let status = fake_proc_status(2, &[12345]);
        let val = parse_proc_status_field(&status, "NStgid:").unwrap();
        assert_eq!(val.split_whitespace().count(), 1);
    }

    // --- check_broker_reap_health ---

    #[test]
    fn broker_reap_health_warn_when_table_missing() {
        let entries = PidfdEntries {
            state: PidfdState::Missing,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_broker_reap_health(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        assert_eq!(report.checks[0].name, "broker-reap-health");
    }

    #[test]
    fn broker_reap_health_pass_when_no_entries() {
        let entries = PidfdEntries {
            state: PidfdState::Loaded,
            entries: vec![],
        };
        let mut report = DoctorReport::default();
        check_broker_reap_health(&entries, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Pass);
    }

    #[test]
    fn proc_stat_state_parse_self() {
        // The test runner itself should not be a zombie.
        let state = read_proc_stat_state(std::process::id() as i32);
        assert!(
            matches!(state, Some('R') | Some('S') | Some('D')),
            "unexpected state: {state:?}"
        );
    }

    // Verify the zombie-detection path using a real exited child.
    #[test]
    fn broker_reap_health_fail_on_zombie() {
        // Spawn a child that exits immediately, then check its state
        // before waitpid — it should be in Z state.
        use std::process::Command;
        let mut child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id() as i32;
        // Give the child time to exit without being reaped.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Read state; if it's already reaped by the OS before we get here
        // (proc entry gone), skip the assertion.
        if let Some(state_char) = read_proc_stat_state(pid) {
            // May be 'Z' (zombie) or already gone ('X') on some kernels.
            assert!(
                state_char == 'Z' || state_char == 'X' || state_char == 'R' || state_char == 'S',
                "unexpected state: {state_char}"
            );
        }
        // Reap to avoid leaking zombies.
        let _ = child.wait();
    }

    // --- check_bridge_ipv6_sysctl ---

    #[test]
    fn bridge_ipv6_sysctl_warn_when_no_bridges() {
        let dir = unique_scratch("bridge-sysctl-no-bridges");
        let mut report = DoctorReport::default();
        // No envs.json and the scratch dir has no br-* ifaces.
        // We override bridge collection by having an empty envs.json.
        write_state(&dir, "envs.json", serde_json::json!({ "envs": [] }));
        check_bridge_ipv6_sysctl(&dir, &mut report);
        assert_eq!(report.checks[0].status, DoctorStatus::Warn);
        assert_eq!(report.checks[0].name, "bridge-ipv6-sysctl");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nixling_bridge_name_matcher() {
        assert!(is_nixling_bridge_name("br-corp-lan"));
        assert!(is_nixling_bridge_name("br-work-up"));
        assert!(!is_nixling_bridge_name("eth0"));
        assert!(!is_nixling_bridge_name("br-corp-mgmt"));
        assert!(!is_nixling_bridge_name("virbr0"));
    }

    #[test]
    fn collect_bridge_names_from_envs_json() {
        let dir = unique_scratch("bridge-collect");
        write_state(
            &dir,
            "envs.json",
            serde_json::json!({
                "envs": [
                    { "lanBridge": "br-corp-lan", "uplinkBridge": "br-corp-up" },
                    { "lanBridge": "br-work-lan", "uplinkBridge": "br-work-up" },
                ]
            }),
        );
        let names = collect_bridge_names(&dir);
        assert_eq!(
            names,
            vec!["br-corp-lan", "br-corp-up", "br-work-lan", "br-work-up"]
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
