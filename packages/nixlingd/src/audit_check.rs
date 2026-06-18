//! In-daemon replacement for the retired
//! `nixling-audit-check.{service,timer}` host singleton + timer that
//! periodically validated the broker audit log shape.
//!
//! # Why this lives in the daemon
//!
//! Before the daemon-owned check, the framework shipped a dedicated
//! systemd oneshot (`nixling-audit-check.service`) and a daily timer
//! (`nixling-audit-check.timer`) whose job was to read the broker's
//! `/var/lib/nixling/audit/broker-<utc-date>.jsonl` daily files and
//! sanity-check their shape — every record parseable as
//! [`OpAuditRecord`][broker], required header fields present, no
//! orphan record whose `decision` contradicts the
//! `error_kind`/`result` invariants the broker writer guarantees.
//! That singleton folds into the unprivileged daemon: the same pure
//! check runs on demand
//! through `GET /health/audit-check` on the daemon's HTTP surface
//! (alongside `GET /metrics`) and on a 5-minute interval from the
//! supervisor event loop, so operators don't have to wait for the
//! daily timer to catch malformed records and `nixling host doctor`
//! can poll cheaply.
//!
//! [broker]: https://github.com/vicondoa/nixling/blob/main/packages/nixling-priv-broker/src/ops/audit_op.rs
//!
//! # What the check asserts
//!
//! For every JSONL line in scope (everything in `audit_dir` whose
//! name matches `broker-YYYY-MM-DD.jsonl`, optionally filtered by
//! `since` to the records emitted after a prior successful check):
//!
//! * The line parses as a JSON object.
//! * Every required header field (`ts_ms`, `broker_version`,
//!   `bundle_version`, `bundle_hash`, `operation`,
//!   `public_operation_id`, `peer_uid`, `peer_gid`, `authz_result`,
//!   `subject_id`, `scope_id`, `decision`) is present with the
//!   expected JSON type.
//! * `decision` is one of the canonical values
//!   (`allowed` / `denied-refused` / `denied-unknown` / `errored`).
//! * `authz_result` is one of the canonical values
//!   (`launcher` / `admin` / `deny`).
//! * Orphan rule: `decision = "errored"` MUST carry a non-null
//!   `error_kind`; conversely a non-null `error_kind` only appears
//!   alongside `decision = "errored"` or `decision = "denied-*"`.
//!   Records that violate either side are flagged as orphans.
//!
//! The check is read-only and hermetic. The pure function
//! [`check_audit_lines`] takes an iterator of `&str` and returns an
//! [`AuditCheckReport`]; the side-effecting wrapper
//! [`run_audit_check`] reads the configured audit directory once,
//! filters by `since`, and defers to the pure function. Tests
//! exercise the pure function with fixtures so we don't need a
//! live broker to lock in regressions.
//!
//! # HTTP surface
//!
//! [`audit_check_handler`] returns the same `AuditCheckReport`
//! rendered as JSON for `GET /health/audit-check`. The handler is
//! pure: callers pass a closure that produces the report so the
//! HTTP layer never touches the filesystem directly. This mirrors
//! the [`metrics_handler`][super::metrics::metrics_handler]
//! pattern.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default broker audit directory. Mirrors the broker's `--audit-dir`
/// default and the retire-shim layout described in
/// `docs/reference/daemon-api.md::Audit`.
pub const DEFAULT_AUDIT_DIR: &str = "/var/lib/nixling/audit";

/// Default sweep interval used by the supervisor event loop.
/// Operators can poll `GET /health/audit-check` on demand at any
/// cadence; this constant only governs the daemon's own background
/// sweep.
pub const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 300;

/// Canonical `decision` values from
/// `nixling_priv_broker::ops::audit_op::OpAuditRecord`.
pub const DECISION_VALUES: &[&str] = &["allowed", "denied-refused", "denied-unknown", "errored"];

/// Canonical `authz_result` values.
pub const AUTHZ_RESULT_VALUES: &[&str] = &["launcher", "admin", "deny"];

/// Required header fields with their expected JSON type.
const REQUIRED_FIELDS: &[(&str, FieldKind)] = &[
    ("ts_ms", FieldKind::Number),
    ("broker_version", FieldKind::String),
    ("bundle_version", FieldKind::String),
    ("bundle_hash", FieldKind::String),
    ("operation", FieldKind::String),
    ("public_operation_id", FieldKind::String),
    ("peer_uid", FieldKind::Number),
    ("peer_gid", FieldKind::Number),
    ("authz_result", FieldKind::String),
    ("subject_id", FieldKind::String),
    ("scope_id", FieldKind::String),
    ("decision", FieldKind::String),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    String,
    Number,
}

impl FieldKind {
    fn matches(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Number => value.is_number(),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
        }
    }
}

/// Per-line problem the check surfaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum AuditLineProblem {
    /// Line did not parse as JSON or was not a JSON object.
    ParseError { message: String },
    /// Required field missing from the record.
    MissingField { field: String },
    /// Required field present but the wrong JSON type.
    WrongFieldType {
        field: String,
        expected: String,
        actual: String,
    },
    /// `decision` value outside the canonical set.
    UnknownDecision { value: String },
    /// `authz_result` value outside the canonical set.
    UnknownAuthzResult { value: String },
    /// `decision = errored` without a populated `error_kind`, or
    /// a populated `error_kind` paired with a non-error decision.
    OrphanRecord {
        decision: String,
        error_kind: Option<String>,
    },
}

/// One bad line + the problem the check identified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLineDefect {
    /// 1-based line index in the concatenated sweep input. The
    /// supervisor sweep concatenates daily files in chronological
    /// order so this index is reproducible across runs (modulo new
    /// records appended after the cursor).
    pub line_index: usize,
    /// File the line came from when known. `None` when callers feed
    /// the pure function directly without a source filename.
    pub source_file: Option<String>,
    pub problem: AuditLineProblem,
}

/// Aggregated result of one check sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditCheckReport {
    /// Total number of lines scanned.
    pub lines_scanned: usize,
    /// Number of lines that passed every assertion.
    pub lines_ok: usize,
    /// Defects discovered, in line order.
    pub defects: Vec<AuditLineDefect>,
}

impl AuditCheckReport {
    pub fn is_clean(&self) -> bool {
        self.defects.is_empty()
    }
}

/// Pure check entry point. Iterates over `(source_file, line)`
/// pairs, validates each, and returns a report. `source_file` is
/// `None` when the caller does not have a filename to attribute
/// (e.g., unit tests).
pub fn check_audit_lines<'a, I>(lines: I) -> AuditCheckReport
where
    I: IntoIterator<Item = (Option<&'a str>, &'a str)>,
{
    let mut report = AuditCheckReport {
        lines_scanned: 0,
        lines_ok: 0,
        defects: Vec::new(),
    };
    for (idx, (source, line)) in lines.into_iter().enumerate() {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // Blank trailing newlines are not records; skip them
            // without counting toward lines_scanned.
            continue;
        }
        report.lines_scanned += 1;
        let line_index = idx + 1;
        let source_owned = source.map(str::to_owned);
        match validate_line(trimmed) {
            Ok(()) => report.lines_ok += 1,
            Err(problems) => {
                for problem in problems {
                    report.defects.push(AuditLineDefect {
                        line_index,
                        source_file: source_owned.clone(),
                        problem,
                    });
                }
            }
        }
    }
    report
}

fn validate_line(line: &str) -> Result<(), Vec<AuditLineProblem>> {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return Err(vec![AuditLineProblem::ParseError {
                message: err.to_string(),
            }]);
        }
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            return Err(vec![AuditLineProblem::ParseError {
                message: format!("expected JSON object, got {}", json_kind(&value)),
            }]);
        }
    };

    let mut problems = Vec::new();
    for (field, kind) in REQUIRED_FIELDS {
        match obj.get(*field) {
            None => problems.push(AuditLineProblem::MissingField {
                field: (*field).to_owned(),
            }),
            Some(v) if !kind.matches(v) => problems.push(AuditLineProblem::WrongFieldType {
                field: (*field).to_owned(),
                expected: kind.as_str().to_owned(),
                actual: json_kind(v).to_owned(),
            }),
            _ => {}
        }
    }

    // Enum range checks only when the value is the right shape.
    if let Some(decision) = obj.get("decision").and_then(Value::as_str)
        && !DECISION_VALUES.contains(&decision)
    {
        problems.push(AuditLineProblem::UnknownDecision {
            value: decision.to_owned(),
        });
    }
    if let Some(authz) = obj.get("authz_result").and_then(Value::as_str)
        && !AUTHZ_RESULT_VALUES.contains(&authz)
    {
        problems.push(AuditLineProblem::UnknownAuthzResult {
            value: authz.to_owned(),
        });
    }

    // Orphan rule: errored ↔ error_kind non-null. error_kind may
    // also appear on denied-* decisions but never on allowed.
    let decision = obj.get("decision").and_then(Value::as_str);
    let error_kind = obj
        .get("error_kind")
        .filter(|v| !v.is_null())
        .and_then(Value::as_str);
    match (decision, error_kind) {
        (Some("errored"), None) => problems.push(AuditLineProblem::OrphanRecord {
            decision: "errored".to_owned(),
            error_kind: None,
        }),
        (Some("allowed"), Some(kind)) => problems.push(AuditLineProblem::OrphanRecord {
            decision: "allowed".to_owned(),
            error_kind: Some(kind.to_owned()),
        }),
        _ => {}
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Returns the sorted list of `broker-YYYY-MM-DD.jsonl` files in
/// `audit_dir`. Non-conforming filenames (operator notes, tarballs,
/// the legacy `broker-audit.log` single-file shape) are skipped so
/// out-of-band artifacts don't pollute the sweep.
pub fn discover_daily_files(audit_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(audit_dir) {
        Ok(it) => it,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name_str = name.to_str()?.to_owned();
            let stem = name_str
                .strip_prefix("broker-")
                .and_then(|s| s.strip_suffix(".jsonl"))?;
            let parts: Vec<&str> = stem.split('-').collect();
            if parts.len() != 3 {
                return None;
            }
            parts[0].parse::<i32>().ok()?;
            parts[1].parse::<u32>().ok()?;
            parts[2].parse::<u32>().ok()?;
            Some(entry.path())
        })
        .collect();
    paths.sort();
    Ok(paths)
}

/// Side-effecting wrapper: reads every `broker-*.jsonl` daily file
/// under `audit_dir`, optionally filters to lines whose `ts_ms` is
/// at or after `since_ts_ms`, and runs [`check_audit_lines`] on the
/// concatenation. A missing `audit_dir` is treated as zero records
/// (the broker has not yet emitted any audit lines on this host),
/// not an error: the report comes back clean with
/// `lines_scanned = 0`.
pub fn run_audit_check(
    audit_dir: &Path,
    since_ts_ms: Option<u128>,
) -> std::io::Result<AuditCheckReport> {
    let files = discover_daily_files(audit_dir)?;
    let mut bag: Vec<(Option<String>, String)> = Vec::new();
    for path in &files {
        let content = std::fs::read_to_string(path)?;
        let file_label = path.file_name().and_then(|s| s.to_str()).map(str::to_owned);
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(cutoff) = since_ts_ms
                && !line_ts_at_least(line, cutoff)
            {
                continue;
            }
            bag.push((file_label.clone(), line.to_owned()));
        }
    }
    let report = check_audit_lines(
        bag.iter()
            .map(|(source, line)| (source.as_deref(), line.as_str())),
    );
    Ok(report)
}

/// True if the line parses as an object and its `ts_ms` field is
/// \>= cutoff. Unparseable lines pass through (so the parse-error
/// defect lands in the report instead of being silently dropped).
fn line_ts_at_least(line: &str, cutoff: u128) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return true;
    };
    let Some(ts) = value.get("ts_ms").and_then(Value::as_u64) else {
        return true;
    };
    u128::from(ts) >= cutoff
}

/// HTTP handler for `GET /health/audit-check`. Other methods get
/// `405`, other paths get `404`. The caller passes a closure that
/// produces the report so the handler stays pure and the sweep
/// surface (filesystem, cursor) is testable independently.
///
/// Response shape:
/// * `200 OK` with `AuditCheckReport` JSON body when the sweep ran
///   to completion, regardless of whether defects were found. The
///   `defects` array tells operators (and `nixling host doctor`)
///   what action to take.
/// * `500 Internal Server Error` when the closure returned an IO
///   error reading the audit directory. Body is a JSON object
///   `{"error": "<message>"}`.
pub fn audit_check_handler<F>(request: &[u8], run: F) -> Vec<u8>
where
    F: FnOnce() -> std::io::Result<AuditCheckReport>,
{
    let head = request.split(|b| *b == b'\n').next().unwrap_or(&[]);
    let head = std::str::from_utf8(head).unwrap_or("");
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    if method != "GET" {
        return http_response(
            405,
            "application/json",
            "{\"error\":\"method not allowed\"}\n",
        );
    }
    if path != "/health/audit-check" {
        return http_response(404, "application/json", "{\"error\":\"not found\"}\n");
    }

    match run() {
        Ok(report) => {
            let body = serde_json::to_string(&report)
                .unwrap_or_else(|err| format!("{{\"error\":\"serialize: {}\"}}", err));
            http_response(200, "application/json", &format!("{body}\n"))
        }
        Err(err) => {
            let body = format!("{{\"error\":\"{}\"}}\n", err);
            http_response(500, "application/json", &body)
        }
    }
}

fn http_response(status: u16, content_type: &str, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut out = Vec::with_capacity(body.len() + 128);
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {status} {reason}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(body.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_record() -> String {
        serde_json::json!({
            "ts_ms": 1_700_000_000_000u64,
            "broker_version": "0.4.0",
            "bundle_version": "v2",
            "bundle_hash": "sha256:dead",
            "operation": "SpawnRunner",
            "public_operation_id": "op-1",
            "peer_uid": 1000,
            "peer_gid": 1000,
            "authz_result": "launcher",
            "subject_id": "vm:work",
            "scope_id": "env:default",
            "decision": "allowed",
            "error_kind": Value::Null,
        })
        .to_string()
    }

    #[test]
    fn clean_lines_produce_clean_report() {
        let l1 = good_record();
        let l2 = good_record();
        let report = check_audit_lines([
            (Some("broker-2024-01-01.jsonl"), l1.as_str()),
            (Some("broker-2024-01-01.jsonl"), l2.as_str()),
        ]);
        assert_eq!(report.lines_scanned, 2);
        assert_eq!(report.lines_ok, 2);
        assert!(report.is_clean(), "report not clean: {:?}", report.defects);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let l = good_record();
        let report = check_audit_lines([(None, "\n"), (None, l.as_str()), (None, "")]);
        assert_eq!(report.lines_scanned, 1);
        assert_eq!(report.lines_ok, 1);
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        let report = check_audit_lines([(None, "not-json")]);
        assert_eq!(report.lines_scanned, 1);
        assert_eq!(report.lines_ok, 0);
        assert!(matches!(
            report.defects[0].problem,
            AuditLineProblem::ParseError { .. }
        ));
    }

    #[test]
    fn non_object_json_is_a_parse_error() {
        let report = check_audit_lines([(None, "[1,2,3]")]);
        assert_eq!(report.defects.len(), 1);
        match &report.defects[0].problem {
            AuditLineProblem::ParseError { message } => {
                assert!(message.contains("array"), "{message}");
            }
            other => panic!("wrong problem: {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_is_flagged() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut().unwrap().remove("bundle_hash");
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::MissingField { field } if field == "bundle_hash"
        )));
    }

    #[test]
    fn wrong_field_type_is_flagged() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("ts_ms".to_owned(), Value::String("not-a-number".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::WrongFieldType { field, .. } if field == "ts_ms"
        )));
    }

    #[test]
    fn unknown_decision_is_flagged() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("decision".to_owned(), Value::String("maybe".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::UnknownDecision { value } if value == "maybe"
        )));
    }

    #[test]
    fn unknown_authz_result_is_flagged() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("authz_result".to_owned(), Value::String("guest".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::UnknownAuthzResult { value } if value == "guest"
        )));
    }

    #[test]
    fn errored_without_error_kind_is_orphan() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("decision".to_owned(), Value::String("errored".into()));
        // error_kind is null in good_record() — fine for allowed,
        // orphan for errored.
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::OrphanRecord { decision, error_kind } if decision == "errored" && error_kind.is_none()
        )));
    }

    #[test]
    fn allowed_with_error_kind_is_orphan() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("error_kind".to_owned(), Value::String("boom".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.defects.iter().any(|d| matches!(
            &d.problem,
            AuditLineProblem::OrphanRecord { decision, error_kind } if decision == "allowed" && error_kind.as_deref() == Some("boom")
        )));
    }

    #[test]
    fn errored_with_error_kind_is_clean() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert("decision".to_owned(), Value::String("errored".into()));
        obj.insert("error_kind".to_owned(), Value::String("eperm".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.is_clean(), "{:?}", report.defects);
    }

    #[test]
    fn denied_refused_with_error_kind_is_clean() {
        let mut v: Value = serde_json::from_str(&good_record()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert(
            "decision".to_owned(),
            Value::String("denied-refused".into()),
        );
        obj.insert("error_kind".to_owned(), Value::String("authz".into()));
        let line = v.to_string();
        let report = check_audit_lines([(None, line.as_str())]);
        assert!(report.is_clean(), "{:?}", report.defects);
    }

    #[test]
    fn defects_record_source_filename() {
        let report = check_audit_lines([(Some("broker-2024-02-02.jsonl"), "not-json")]);
        assert_eq!(
            report.defects[0].source_file.as_deref(),
            Some("broker-2024-02-02.jsonl")
        );
        assert_eq!(report.defects[0].line_index, 1);
    }

    #[test]
    fn run_audit_check_on_missing_dir_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        let report = run_audit_check(&nonexistent, None).unwrap();
        assert_eq!(report.lines_scanned, 0);
        assert!(report.is_clean());
    }

    #[test]
    fn run_audit_check_reads_dated_files_and_skips_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("broker-2024-01-01.jsonl"),
            format!("{}\n{}\n", good_record(), good_record()),
        )
        .unwrap();
        // Out-of-band file: must be skipped.
        std::fs::write(dir.path().join("broker-audit.log"), "garbage\n").unwrap();
        std::fs::write(dir.path().join("operator-notes.txt"), "garbage\n").unwrap();

        let report = run_audit_check(dir.path(), None).unwrap();
        assert_eq!(report.lines_scanned, 2);
        assert_eq!(report.lines_ok, 2);
    }

    #[test]
    fn run_audit_check_respects_since_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let old = serde_json::json!({
            "ts_ms": 100u64,
            "broker_version": "0.4.0",
            "bundle_version": "v2",
            "bundle_hash": "sha256:dead",
            "operation": "SpawnRunner",
            "public_operation_id": "op-1",
            "peer_uid": 1000,
            "peer_gid": 1000,
            "authz_result": "launcher",
            "subject_id": "vm:work",
            "scope_id": "env:default",
            "decision": "allowed",
            "error_kind": Value::Null,
        })
        .to_string();
        let new = good_record();
        std::fs::write(
            dir.path().join("broker-2024-01-01.jsonl"),
            format!("{old}\n{new}\n"),
        )
        .unwrap();
        let report = run_audit_check(dir.path(), Some(1_000_000_000_000)).unwrap();
        assert_eq!(report.lines_scanned, 1);
    }

    #[test]
    fn run_audit_check_surfaces_defects_with_filename() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broker-2024-01-01.jsonl"), "not-json\n").unwrap();
        let report = run_audit_check(dir.path(), None).unwrap();
        assert_eq!(report.defects.len(), 1);
        assert_eq!(
            report.defects[0].source_file.as_deref(),
            Some("broker-2024-01-01.jsonl")
        );
    }

    #[test]
    fn handler_returns_200_with_report_body() {
        let resp = audit_check_handler(b"GET /health/audit-check HTTP/1.1\r\n\r\n", || {
            Ok(AuditCheckReport {
                lines_scanned: 3,
                lines_ok: 3,
                defects: Vec::new(),
            })
        });
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("application/json"));
        assert!(text.contains("\"lines_scanned\":3"));
        assert!(text.contains("\"defects\":[]"));
    }

    #[test]
    fn handler_returns_200_when_defects_present() {
        let resp = audit_check_handler(b"GET /health/audit-check HTTP/1.1\r\n\r\n", || {
            Ok(AuditCheckReport {
                lines_scanned: 1,
                lines_ok: 0,
                defects: vec![AuditLineDefect {
                    line_index: 1,
                    source_file: Some("broker-2024-01-01.jsonl".to_owned()),
                    problem: AuditLineProblem::ParseError {
                        message: "boom".to_owned(),
                    },
                }],
            })
        });
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("parse-error"));
    }

    #[test]
    fn handler_rejects_non_get() {
        let resp = audit_check_handler(b"POST /health/audit-check HTTP/1.1\r\n\r\n", || {
            Ok(AuditCheckReport {
                lines_scanned: 0,
                lines_ok: 0,
                defects: Vec::new(),
            })
        });
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 405 "));
    }

    #[test]
    fn handler_rejects_other_paths() {
        let resp = audit_check_handler(b"GET /metrics HTTP/1.1\r\n\r\n", || {
            Ok(AuditCheckReport {
                lines_scanned: 0,
                lines_ok: 0,
                defects: Vec::new(),
            })
        });
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 404 "));
    }

    #[test]
    fn handler_returns_500_on_io_error() {
        let resp = audit_check_handler(b"GET /health/audit-check HTTP/1.1\r\n\r\n", || {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            ))
        });
        let text = std::str::from_utf8(&resp).unwrap();
        assert!(text.starts_with("HTTP/1.1 500 "));
        assert!(text.contains("denied"));
    }
}
