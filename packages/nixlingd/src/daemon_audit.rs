//! Daemon-side JSONL audit events emitted by `nixlingd` for transitions
//! not covered by the broker's `OpAuditRecord` stream.
//!
//! Events are written to
//! `{daemon_state_dir}/daemon-events-{YYYY-MM-DD}.jsonl` (daemon-owned,
//! separate from the broker's `broker-{date}.jsonl` files). Each line is
//! a self-contained JSON object carrying `ts_ms` + `source` + a
//! per-variant `event` object.
//!
//! # Additive-only contract
//!
//! `DaemonEvent` is `#[non_exhaustive]`. New variants MAY be added in
//! any release; existing variants MUST NOT be renamed or removed. Field
//! names use `snake_case` (matching the `#[serde(rename_all = "snake_case")]`
//! attribute). This mirrors the broker audit's forward-compat posture.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Closed detached exec audit action.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DetachedExecAuditAction {
    Create,
    Cancel,
}

/// Closed detached exec audit result.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DetachedExecAuditResult {
    Created,
    Cancelling,
    AlreadyTerminal,
    Error,
}

/// Closed reason a vm-start runner node fast-failed before readiness.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VmStartRunnerExitReason {
    /// The spawned runner terminated before its readiness signal fired.
    RunnerExited,
    /// The runner's PID was reused by a different process (start-time
    /// drift) — our runner is gone.
    RunnerReused,
}

/// Closed, bounded runner exit kind mirrored from the broker reap
/// notification. Carries no high-cardinality detail.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerExitKind {
    Exited,
    Signaled,
    Killed,
}

/// Daemon-side audit event variants.
///
/// Additive-only: new variants may be added; existing ones must not be
/// renamed or removed. `#[non_exhaustive]` enforces this at the type level.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum DaemonEvent {
    /// Emitted when the api-ready phase of a VM start does not converge
    /// within the configured timeout in strict split-readiness mode.
    ApiReadyTimeout {
        /// VM name (matches the `vmStart` request).
        vm: String,
        /// Role id of the runner node whose api-ready probe timed out.
        runner: String,
        /// Configured timeout that elapsed, in whole seconds.
        elapsed_secs: u64,
        /// Split-readiness mode: `"strict"` or `"no-wait-api"`.
        mode: String,
    },
    /// Emitted when an authenticated `vm exec` owner session is established
    /// (after admin authz + capability negotiation, before any op proxy).
    ///
    /// Leak-safe by construction: carries ONLY the VM name, the admin peer
    /// uid, and the negotiated tty shape. The opaque session handle, argv,
    /// env, cwd, and any stdio bytes are NEVER recorded.
    GuestControlExecEstablished {
        /// VM name the exec session targets.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that opened the session.
        peer_uid: u32,
        /// Whether a PTY was negotiated for the session.
        tty: bool,
    },
    /// Emitted when a previously-established `vm exec` owner session ends
    /// (owner disconnect, command terminal, or teardown).
    ///
    /// Leak-safe: carries ONLY the VM name and the admin peer uid. No
    /// session handle, exit status bytes, argv, env, cwd, or stdio.
    GuestControlExecTerminated {
        /// VM name the exec session targeted.
        vm: String,
        /// Admin peer uid (from `SO_PEERCRED`) that owned the session.
        peer_uid: u32,
    },
    /// Emitted when a detached `vm exec -d` create succeeds.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, closed
    /// action/result enums, and the opaque guest exec id. argv, env, cwd, and
    /// retained output bytes are never recorded.
    GuestControlExecDetachedCreate {
        vm: String,
        peer_uid: u32,
        action: DetachedExecAuditAction,
        result: DetachedExecAuditResult,
        exec_id: String,
    },
    /// Emitted when a detached `vm exec <vm> kill <id>` cancel path is
    /// attempted.
    ///
    /// Leak-safe: carries ONLY the VM name, admin peer uid, closed
    /// action/result enums, and the opaque target exec id. No argv, env, cwd,
    /// or log bytes.
    GuestControlExecDetachedKill {
        vm: String,
        peer_uid: u32,
        action: DetachedExecAuditAction,
        result: DetachedExecAuditResult,
        exec_id: String,
    },
    /// Emitted when a `vm start` long-lived runner node fast-fails because
    /// the spawned runner terminated (or its PID was reused) BEFORE its
    /// readiness signal fired — the `tpm.enable` first-run wedge fix.
    ///
    /// Bounded by construction: carries ONLY the VM name, the closed
    /// `role_id` of the failed node, a closed reason kind, the optional
    /// closed broker exit kind/code/signal, and the elapsed wall-clock
    /// milliseconds. No node-reason string, pid, or path label is
    /// recorded.
    VmStartRunnerExited {
        /// VM name (matches the `vmStart` request).
        vm: String,
        /// Role id of the runner node that exited (e.g. `swtpm`,
        /// `ch-runner`).
        role_id: String,
        /// Closed reason kind: exited vs PID-reused.
        reason_kind: VmStartRunnerExitReason,
        /// Bounded broker exit kind, when a reap status was buffered.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_kind: Option<RunnerExitKind>,
        /// Exit code, when `exit_kind == "exited"`.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        /// Signal number, when `exit_kind` is `signaled`/`killed`.
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_signal: Option<i32>,
        /// Wall-clock milliseconds from DAG dispatch to fast-fail.
        elapsed_ms: u64,
    },
}

/// JSONL audit-log writer for daemon-side events.
///
/// - **Production**: use [`DaemonAuditLog::new`]; events are appended to
///   the day's `daemon-events-{YYYY-MM-DD}.jsonl` file inside the
///   daemon-state directory.
/// - **Tests that don't care about audit output**: use
///   [`DaemonAuditLog::no_op`]; all writes are silently discarded.
///
/// Appends are serialized behind a single in-process writer mutex so
/// concurrent connection-handler threads cannot interleave bytes within
/// a JSONL line. Retention is enforced best-effort: stale
/// `daemon-events-*.jsonl` files older than [`AUDIT_RETENTION_DAYS`] are
/// pruned on open and again whenever a write crosses a day boundary
/// (the file name itself provides day-boundary rotation).
#[derive(Debug)]
pub struct DaemonAuditLog {
    state_dir: Option<PathBuf>,
    writer: Arc<Mutex<AuditWriterState>>,
    #[cfg(test)]
    pub(crate) captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

/// Default retention window for daemon audit JSONL files (days).
pub const AUDIT_RETENTION_DAYS: i64 = 14;

/// Serialization + day-boundary bookkeeping for the single audit appender.
#[derive(Debug, Default)]
struct AuditWriterState {
    /// UTC date string (`YYYY-MM-DD`) of the most recent append. Used to
    /// detect a day-boundary crossing so retention pruning re-runs.
    last_date: Option<String>,
}

impl DaemonAuditLog {
    /// Production constructor. Events are appended to the day's JSONL
    /// file under `state_dir`. Prunes stale logs once on open
    /// (best-effort).
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        prune_old_audit_logs(&state_dir, AUDIT_RETENTION_DAYS);
        Self {
            state_dir: Some(state_dir),
            writer: Arc::new(Mutex::new(AuditWriterState::default())),
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// No-op constructor for tests that do not exercise audit output.
    pub fn no_op() -> Self {
        Self {
            state_dir: None,
            writer: Arc::new(Mutex::new(AuditWriterState::default())),
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// Serialize and append one `DaemonEvent` JSONL line.
    ///
    /// This method is best-effort: callers MUST NOT abort the surrounding
    /// operation on audit failure. They should log the error (if any) and
    /// continue.
    ///
    /// The actual file append is serialized behind a single writer mutex
    /// so concurrent handler threads produce a valid, line-atomic JSONL
    /// stream. A day-boundary crossing triggers best-effort retention
    /// pruning of stale `daemon-events-*.jsonl` files.
    pub fn write_event(&self, event: &DaemonEvent) -> io::Result<()> {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let record = serde_json::json!({
            "ts_ms": ts_ms,
            "source": "nixlingd",
            "event": serde_json::to_value(event)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        });
        let mut line = serde_json::to_string(&record)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        #[cfg(test)]
        {
            self.captured
                .lock()
                .map_err(|_| io::Error::other("DaemonAuditLog capture mutex poisoned"))?
                .push(line.trim_end_matches('\n').to_owned());
        }

        if let Some(ref state_dir) = self.state_dir {
            let mut writer = self
                .writer
                .lock()
                .map_err(|_| io::Error::other("DaemonAuditLog writer mutex poisoned"))?;
            let today = utc_date_string();
            // First write of the process or a day-boundary crossing:
            // re-run retention pruning (best-effort) before appending.
            if writer.last_date.as_deref() != Some(today.as_str()) {
                prune_old_audit_logs(state_dir, AUDIT_RETENTION_DAYS);
                writer.last_date = Some(today.clone());
            }
            write_jsonl_line_for_date(state_dir, &today, &line)?;
        }
        Ok(())
    }
}

fn write_jsonl_line_for_date(state_dir: &Path, today: &str, line: &str) -> io::Result<()> {
    let path = state_dir.join(format!("daemon-events-{today}.jsonl"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())
}

/// Best-effort retention: delete `daemon-events-YYYY-MM-DD.jsonl` files
/// whose date is older than `retention_days` before today (UTC). All
/// errors are swallowed — retention must never abort an audit write.
fn prune_old_audit_logs(state_dir: &Path, retention_days: i64) {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let cutoff = ymd_from_unix(now_secs - retention_days * 86_400);
    let Ok(entries) = fs::read_dir(state_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(date) = name
            .strip_prefix("daemon-events-")
            .and_then(|rest| rest.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if let Some(parsed) = parse_ymd(date)
            && parsed < cutoff
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Parse a `YYYY-MM-DD` stamp into a comparable `(year, month, day)`
/// tuple. Returns `None` on any malformed component.
fn parse_ymd(date: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date.split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

fn utc_date_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, m, d) = ymd_from_unix(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Civil-from-days algorithm (Howard Hinnant, public domain). Avoids
/// pulling in a chrono / time crate just for date stamping.
fn ymd_from_unix(unix: i64) -> (i32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Write the api-ready state for a VM to
/// `{daemon_state_dir}/{vm}/api-ready.json`.
///
/// Called from `dispatch_broker_vm_start` after the DAG executor returns
/// a `DagRunReport` with a non-`None` `api_ready` field, so that
/// `nixling vm status <vm>` can report the live state instead of
/// hard-coding `None`.
///
/// File format: `{"apiReady": <value>}` where `<value>` mirrors the
/// [`supervisor::dag::ApiReadyState`] wire encoding:
/// - `"yes"` | `"pending"` | `"timeout"` for simple states
/// - `{"error": "<reason>"}` for error states
///
/// Best-effort: a write failure is logged via `tracing::warn!` but MUST
/// NOT abort the surrounding vm-start response.
pub fn write_vm_api_ready_state(
    daemon_state_dir: &Path,
    vm: &str,
    api_ready_value: serde_json::Value,
) -> io::Result<()> {
    let dir = daemon_state_dir.join(vm);
    fs::create_dir_all(&dir)?;
    let path = dir.join("api-ready.json");
    let content = serde_json::json!({ "apiReady": api_ready_value });
    let bytes =
        serde_json::to_vec(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // Atomic tmp+rename so the reader never sees a partial file.
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_ready_timeout_event_writes_jsonl_and_captures() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = DaemonAuditLog::new(dir.path());

        // Trigger a fake api-ready timeout event.
        log.write_event(&DaemonEvent::ApiReadyTimeout {
            vm: "vm-a".to_owned(),
            runner: "ch-runner".to_owned(),
            elapsed_secs: 60,
            mode: "strict".to_owned(),
        })
        .expect("write api-ready-timeout event");

        // Assert the in-memory captured record has the expected fields.
        let records = log.captured.lock().expect("lock captured");
        assert_eq!(
            records.len(),
            1,
            "expected exactly one captured audit record"
        );
        let record: serde_json::Value =
            serde_json::from_str(&records[0]).expect("parse captured audit record as JSON");

        assert_eq!(
            record.get("source").and_then(|v| v.as_str()),
            Some("nixlingd"),
            "source field must be 'nixlingd'",
        );
        let event = record.get("event").expect("event field must be present");
        assert_eq!(
            event.get("kind").and_then(|v| v.as_str()),
            Some("api_ready_timeout"),
            "event.kind must be 'api_ready_timeout'",
        );
        assert_eq!(
            event.get("vm").and_then(|v| v.as_str()),
            Some("vm-a"),
            "event.vm must match",
        );
        assert_eq!(
            event.get("runner").and_then(|v| v.as_str()),
            Some("ch-runner"),
            "event.runner must match",
        );
        assert_eq!(
            event.get("elapsed_secs").and_then(|v| v.as_u64()),
            Some(60),
            "event.elapsed_secs must match",
        );
        assert_eq!(
            event.get("mode").and_then(|v| v.as_str()),
            Some("strict"),
            "event.mode must be 'strict'",
        );

        // Also confirm the JSONL file was written to disk.
        let day_files: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("daemon-events-")
            })
            .collect();
        assert_eq!(
            day_files.len(),
            1,
            "expected exactly one daily daemon-events JSONL file"
        );
        // Read back and verify the line on disk matches the captured record.
        let path = day_files[0].path();
        let content = std::fs::read_to_string(&path).expect("read daemon-events file");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "expected one JSONL line in the file");
        let disk_record: serde_json::Value =
            serde_json::from_str(lines[0]).expect("parse disk JSONL line");
        assert_eq!(
            disk_record
                .get("event")
                .and_then(|e| e.get("kind"))
                .and_then(|v| v.as_str()),
            Some("api_ready_timeout"),
        );
    }

    #[test]
    fn exec_lifecycle_events_are_leak_safe() {
        // The exec establish + terminate audit events carry ONLY
        // leak-safe fields (vm, peer_uid, tty). A planted sentinel standing in
        // for a session handle / argv / env / cwd must never appear, and the
        // serialized event must expose no unexpected key.
        const SENTINEL: &str = "SENTINEL-handle-argv-env-cwd-9b2f";
        let log = DaemonAuditLog::no_op();

        log.write_event(&DaemonEvent::GuestControlExecEstablished {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            tty: true,
        })
        .expect("write established event");
        log.write_event(&DaemonEvent::GuestControlExecTerminated {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
        })
        .expect("write terminated event");

        let records = log.captured.lock().expect("lock captured");
        assert_eq!(records.len(), 2, "expected two captured lifecycle records");

        for line in records.iter() {
            assert!(
                !line.contains(SENTINEL),
                "exec lifecycle audit leaked a sentinel: {line}"
            );
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse captured lifecycle record");
            assert_eq!(
                record.get("source").and_then(|v| v.as_str()),
                Some("nixlingd")
            );
            let event = record.get("event").expect("event object");
            let obj = event.as_object().expect("event is an object");
            // Closed key set: kind + the leak-safe fields only. No `session`,
            // `handle`, `argv`, `env`, `cwd`, or stdio keys may appear.
            for key in obj.keys() {
                assert!(
                    matches!(key.as_str(), "kind" | "vm" | "peer_uid" | "tty"),
                    "exec lifecycle audit exposed an unexpected key {key:?}: {line}"
                );
            }
            assert_eq!(event.get("vm").and_then(|v| v.as_str()), Some("corp-vm"));
            assert_eq!(event.get("peer_uid").and_then(|v| v.as_u64()), Some(1000));
        }

        let established = serde_json::from_str::<serde_json::Value>(&records[0])
            .expect("parse established record");
        assert_eq!(
            established["event"]["kind"].as_str(),
            Some("guest_control_exec_established")
        );
        assert_eq!(established["event"]["tty"].as_bool(), Some(true));

        let terminated = serde_json::from_str::<serde_json::Value>(&records[1])
            .expect("parse terminated record");
        assert_eq!(
            terminated["event"]["kind"].as_str(),
            Some("guest_control_exec_terminated")
        );
        // The terminate event has no tty field (only vm + peer_uid).
        assert!(terminated["event"].get("tty").is_none());
    }

    #[test]
    fn detached_exec_audit_events_are_leak_safe() {
        const SENTINEL: &str = "SENTINEL-argv-env-cwd-log-bytes-2d7b";
        let log = DaemonAuditLog::no_op();

        log.write_event(&DaemonEvent::GuestControlExecDetachedCreate {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: DetachedExecAuditAction::Create,
            result: DetachedExecAuditResult::Created,
            exec_id: "exec-opaque-1".to_owned(),
        })
        .expect("write detached create event");
        log.write_event(&DaemonEvent::GuestControlExecDetachedKill {
            vm: "corp-vm".to_owned(),
            peer_uid: 1000,
            action: DetachedExecAuditAction::Cancel,
            result: DetachedExecAuditResult::Cancelling,
            exec_id: "exec-opaque-1".to_owned(),
        })
        .expect("write detached kill event");

        let records = log.captured.lock().expect("lock captured");
        assert_eq!(records.len(), 2, "expected two detached audit records");

        for line in records.iter() {
            assert!(
                !line.contains(SENTINEL),
                "detached exec audit leaked a sentinel: {line}"
            );
            for forbidden in [
                "\"argv\"",
                "\"env\"",
                "\"cwd\"",
                "\"stdout\"",
                "\"stderr\"",
                "\"log_bytes\"",
            ] {
                assert!(
                    !line.contains(forbidden),
                    "detached exec audit exposed forbidden field {forbidden:?}: {line}"
                );
            }
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse detached audit record");
            let event = record.get("event").expect("event object");
            let obj = event.as_object().expect("event is an object");
            for key in obj.keys() {
                assert!(
                    matches!(
                        key.as_str(),
                        "kind" | "vm" | "peer_uid" | "action" | "result" | "exec_id"
                    ),
                    "detached exec audit exposed unexpected key {key:?}: {line}"
                );
            }
            assert_eq!(event.get("vm").and_then(|v| v.as_str()), Some("corp-vm"));
            assert_eq!(event.get("peer_uid").and_then(|v| v.as_u64()), Some(1000));
            assert_eq!(
                event.get("exec_id").and_then(|v| v.as_str()),
                Some("exec-opaque-1")
            );
        }

        let create = serde_json::from_str::<serde_json::Value>(&records[0])
            .expect("parse detached create record");
        assert_eq!(
            create["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_create")
        );
        assert_eq!(create["event"]["action"].as_str(), Some("create"));
        assert_eq!(create["event"]["result"].as_str(), Some("created"));

        let kill = serde_json::from_str::<serde_json::Value>(&records[1])
            .expect("parse detached kill record");
        assert_eq!(
            kill["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_kill")
        );
        assert_eq!(kill["event"]["action"].as_str(), Some("cancel"));
        assert_eq!(kill["event"]["result"].as_str(), Some("cancelling"));
    }

    #[test]
    fn no_op_does_not_write_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // Create a no-op log — but give it the temp dir to make sure the
        // file is NOT created.
        let log = DaemonAuditLog::no_op();
        // Manually set state_dir to the temp dir via a helper.
        // We can't do that here because state_dir is private; instead,
        // create a no_op and verify its captured vec is empty.
        log.write_event(&DaemonEvent::ApiReadyTimeout {
            vm: "vm-a".to_owned(),
            runner: "ch-runner".to_owned(),
            elapsed_secs: 30,
            mode: "strict".to_owned(),
        })
        .expect("no-op write should not error");

        // No file should appear in temp dir (no state_dir set).
        let count = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .count();
        assert_eq!(count, 0, "no-op log must not write any files");
    }

    #[test]
    fn write_vm_api_ready_state_roundtrip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        write_vm_api_ready_state(
            dir.path(),
            "vm-a",
            serde_json::Value::String("timeout".to_owned()),
        )
        .expect("write api-ready state");

        let path = dir.path().join("vm-a").join("api-ready.json");
        let content = std::fs::read_to_string(&path).expect("read api-ready.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("parse api-ready.json");
        assert_eq!(
            parsed.get("apiReady").and_then(|v| v.as_str()),
            Some("timeout"),
        );
    }

    #[test]
    fn concurrent_writes_produce_valid_jsonl() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let log = std::sync::Arc::new(DaemonAuditLog::new(dir.path()));

        let mut handles = Vec::new();
        for thread_idx in 0..8 {
            let log = std::sync::Arc::clone(&log);
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    log.write_event(&DaemonEvent::VmStartRunnerExited {
                        vm: format!("vm-{thread_idx}"),
                        role_id: "swtpm".to_owned(),
                        reason_kind: VmStartRunnerExitReason::RunnerExited,
                        exit_kind: Some(RunnerExitKind::Exited),
                        exit_code: Some(1),
                        exit_signal: None,
                        elapsed_ms: 12,
                    })
                    .expect("concurrent write");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("join writer thread");
        }

        let today = utc_date_string();
        let path = dir.path().join(format!("daemon-events-{today}.jsonl"));
        let content = std::fs::read_to_string(&path).expect("read jsonl file");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 8 * 25, "every concurrent append must land");
        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line is valid, line-atomic JSON");
            assert_eq!(
                parsed.get("source").and_then(|v| v.as_str()),
                Some("nixlingd"),
            );
        }
    }

    #[test]
    fn retention_prunes_stale_logs_on_open() {
        let dir = tempfile::tempdir().expect("create temp dir");

        // A file dated well beyond the retention window must be pruned.
        let stale = dir.path().join("daemon-events-2000-01-01.jsonl");
        std::fs::write(&stale, "{}\n").expect("write stale log");
        // A foreign file must be left untouched.
        let foreign = dir.path().join("unrelated.txt");
        std::fs::write(&foreign, "keep me").expect("write foreign file");
        // A current-day file must survive.
        let today = utc_date_string();
        let fresh = dir.path().join(format!("daemon-events-{today}.jsonl"));
        std::fs::write(&fresh, "{}\n").expect("write fresh log");

        // Construction prunes on open.
        let _log = DaemonAuditLog::new(dir.path());

        assert!(!stale.exists(), "stale audit log must be pruned on open");
        assert!(foreign.exists(), "foreign files must not be touched");
        assert!(fresh.exists(), "current-day log must survive retention");
    }

    #[test]
    fn parse_ymd_rejects_malformed_dates() {
        assert_eq!(parse_ymd("2024-03-09"), Some((2024, 3, 9)));
        assert_eq!(parse_ymd("2024-13-01"), None);
        assert_eq!(parse_ymd("2024-03"), None);
        assert_eq!(parse_ymd("2024-03-09-10"), None);
        assert_eq!(parse_ymd("not-a-date"), None);
    }
}
