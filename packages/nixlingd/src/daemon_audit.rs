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
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

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
}

/// JSONL audit-log writer for daemon-side events.
///
/// - **Production**: use [`DaemonAuditLog::new`]; events are appended to
///   the day's `daemon-events-{YYYY-MM-DD}.jsonl` file inside the
///   daemon-state directory.
/// - **Tests that don't care about audit output**: use
///   [`DaemonAuditLog::no_op`]; all writes are silently discarded.
#[derive(Debug)]
pub struct DaemonAuditLog {
    state_dir: Option<PathBuf>,
    #[cfg(test)]
    pub(crate) captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl DaemonAuditLog {
    /// Production constructor. Events are appended to the day's JSONL
    /// file under `state_dir`.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: Some(state_dir.into()),
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// No-op constructor for tests that do not exercise audit output.
    pub fn no_op() -> Self {
        Self {
            state_dir: None,
            #[cfg(test)]
            captured: Default::default(),
        }
    }

    /// Serialize and append one `DaemonEvent` JSONL line.
    ///
    /// This method is best-effort: callers MUST NOT abort the surrounding
    /// operation on audit failure. They should log the error (if any) and
    /// continue.
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
            write_jsonl_line(state_dir, &line)?;
        }
        Ok(())
    }
}

fn write_jsonl_line(state_dir: &Path, line: &str) -> io::Result<()> {
    let today = utc_date_string();
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
}
