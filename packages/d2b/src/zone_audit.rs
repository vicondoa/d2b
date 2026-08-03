//! The admin-only Zone audit export command.

use std::{fs, io::BufRead, path::Path};

use clap::Args;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    print_stdout,
};

/// Maximum bytes accepted for one exported audit line.
pub(crate) const MAX_AUDIT_LINE_BYTES: usize = 64 * 1024;
/// Maximum number of lines accepted in one bounded service response.
pub(crate) const MAX_AUDIT_LINES: usize = 4096;

/// Arguments for `d2b zone audit export`.
#[derive(Debug, Args, Clone)]
pub(crate) struct AuditExportArgs {
    /// Export segments after this owned segment basename.
    #[arg(long)]
    pub(crate) after: Option<String>,
    /// Export segments before this owned segment basename.
    #[arg(long)]
    pub(crate) before: Option<String>,
}

/// Run the audit export service and stream its bounded NDJSON response.
pub(crate) fn run(
    context: &ZoneContext,
    args: &AuditExportArgs,
    _mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    validate_boundary(context, args.after.as_deref(), "after")?;
    validate_boundary(context, args.before.as_deref(), "before")?;

    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::Audit,
        "AuditService/Export",
        "audit-export",
        json!({
            "zone": context.zone_name(),
            "after": args.after,
            "before": args.before,
        }),
        deadline,
        OutputMode::Json,
    )?;
    let lines = response_lines(context, &value)?;
    let mut validator = AuditStreamValidator::new(args.after.is_some());
    for line in lines {
        let rendered = validator.accept(&line);
        print_stdout(&rendered);
        print_stdout("\n");
    }
    Ok(if validator.had_break() { 1 } else { 0 })
}

fn validate_boundary(
    context: &ZoneContext,
    value: Option<&str>,
    label: &str,
) -> Result<(), CliFailure> {
    if value.is_some_and(|value| !is_segment_name(value)) {
        return Err(context.failure(
            "ref-invalid",
            &format!("{label} must be an owned audit segment basename"),
            OutputMode::Json,
            2,
        ));
    }
    Ok(())
}

fn response_lines(context: &ZoneContext, value: &Value) -> Result<Vec<String>, CliFailure> {
    let lines = if let Some(lines) = value.get("lines").and_then(Value::as_array) {
        lines
    } else if let Some(lines) = value.get("events").and_then(Value::as_array) {
        lines
    } else if let Some(lines) = value.get("records").and_then(Value::as_array) {
        lines
    } else if let Some(ndjson) = value.get("ndjson").and_then(Value::as_str) {
        return split_ndjson(context, ndjson);
    } else if value.is_array() {
        value.as_array().expect("array checked above")
    } else {
        return Err(context.failure(
            "exec-protocol-error",
            "audit export returned no NDJSON stream",
            OutputMode::Json,
            1,
        ));
    };

    if lines.len() > MAX_AUDIT_LINES {
        return Err(context.failure(
            "exec-protocol-error",
            "audit export exceeded the bounded line limit",
            OutputMode::Json,
            1,
        ));
    }
    lines
        .iter()
        .try_fold(
            (Vec::with_capacity(lines.len()), 0_usize),
            |(mut output, total), line| {
                let line = match line {
                    Value::String(line) => line.clone(),
                    value => serde_json::to_string(value).map_err(|_| {
                        context.failure(
                            "exec-protocol-error",
                            "audit export returned an invalid NDJSON line",
                            OutputMode::Json,
                            1,
                        )
                    })?,
                };
                if line.len() > MAX_AUDIT_LINE_BYTES {
                    return Err(context.failure(
                        "exec-protocol-error",
                        "audit export returned an oversized NDJSON line",
                        OutputMode::Json,
                        1,
                    ));
                }
                let total = total.saturating_add(line.len());
                if total > crate::MAX_FRAME_BYTES {
                    return Err(context.failure(
                        "exec-protocol-error",
                        "audit export exceeded the bounded response limit",
                        OutputMode::Json,
                        1,
                    ));
                }
                output.push(line);
                Ok((output, total))
            },
        )
        .map(|(output, _)| output)
}

fn split_ndjson(context: &ZoneContext, ndjson: &str) -> Result<Vec<String>, CliFailure> {
    if ndjson.len() > crate::MAX_FRAME_BYTES {
        return Err(context.failure(
            "exec-protocol-error",
            "audit export exceeded the bounded response limit",
            OutputMode::Json,
            1,
        ));
    }
    let lines = ndjson
        .lines()
        .map(str::to_owned)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.len() > MAX_AUDIT_LINES || lines.iter().any(|line| line.len() > MAX_AUDIT_LINE_BYTES) {
        return Err(context.failure(
            "exec-protocol-error",
            "audit export exceeded the bounded line limit",
            OutputMode::Json,
            1,
        ));
    }
    Ok(lines)
}

/// Stateful inline validator for the redacted audit stream.
#[derive(Debug)]
struct AuditStreamValidator {
    previous: String,
    sequence: u64,
    allow_non_genesis_first: bool,
    chain_valid: bool,
}

impl AuditStreamValidator {
    fn new(allow_non_genesis_first: bool) -> Self {
        Self {
            previous: genesis_hash(),
            sequence: 0,
            allow_non_genesis_first,
            chain_valid: true,
        }
    }

    fn had_break(&self) -> bool {
        !self.chain_valid
    }

    fn accept(&mut self, line: &str) -> String {
        let sequence = self.sequence;
        self.sequence = self.sequence.saturating_add(1);
        if line.len() > MAX_AUDIT_LINE_BYTES {
            self.chain_valid = false;
            return error_line(sequence, "record-oversize");
        }

        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) => {
                self.chain_valid = false;
                return error_line(sequence, "record-invalid");
            }
        };
        if contains_forbidden_field(&value) {
            self.chain_valid = false;
            return error_line(sequence, "record-invalid");
        }
        if let Some(error_code) = value.get("export_error").and_then(Value::as_str) {
            self.chain_valid = false;
            return error_line(sequence, allowed_error_code(error_code));
        }

        if !self.chain_valid {
            return error_line(sequence, "hash-break");
        }
        let expected_previous = if self.sequence == 1 && self.allow_non_genesis_first {
            None
        } else {
            Some(self.previous.as_str())
        };
        let record_hash = match validate_record(&value, expected_previous) {
            Ok(record_hash) => record_hash,
            Err(RecordValidationError::Invalid) => {
                self.chain_valid = false;
                return error_line(sequence, "record-invalid");
            }
            Err(RecordValidationError::ChainBreak) => {
                self.chain_valid = false;
                return error_line(sequence, "hash-break");
            }
        };
        self.previous = record_hash;
        serde_json::to_string(&value).unwrap_or_else(|_| error_line(sequence, "record-invalid"))
    }
}

fn error_line(sequence: u64, error_code: &str) -> String {
    json!({
        "export_error": error_code,
        "sequence": sequence,
    })
    .to_string()
}

fn allowed_error_code(value: &str) -> &'static str {
    match value {
        "hash-break" => "hash-break",
        "read-failed" => "read-failed",
        "record-oversize" => "record-oversize",
        _ => "record-invalid",
    }
}

fn contains_forbidden_field(value: &Value) -> bool {
    const FORBIDDEN: &[&str] = &[
        "argv",
        "credential",
        "env",
        "metadata",
        "name",
        "node",
        "path",
        "realm",
        "socket",
        "workload_id",
    ];
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            FORBIDDEN.contains(&key.to_ascii_lowercase().as_str())
                || contains_forbidden_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_forbidden_field),
        Value::String(value) => {
            value.len() > MAX_AUDIT_LINE_BYTES
                || value.contains('/')
                || value.chars().any(char::is_control)
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordValidationError {
    Invalid,
    ChainBreak,
}

fn validate_record(
    value: &Value,
    expected_previous: Option<&str>,
) -> Result<String, RecordValidationError> {
    let object = value.as_object().ok_or(RecordValidationError::Invalid)?;
    const ENVELOPE_KEYS: &[&str] = &[
        "ts_ms",
        "schema_version",
        "zone",
        "record_class",
        "operation_id",
        "correlation_id",
        "trace_id",
        "source",
        "prev_hash",
        "record_hash",
        "resource_mutation_fields",
        "resource_upgrade_fields",
        "rbac_change_fields",
        "session_connect_fields",
        "route_admission_fields",
        "resource_share_fields",
        "broker_effect_fields",
        "process_effect_fields",
        "state_reset_fields",
    ];
    if object
        .keys()
        .any(|key| !ENVELOPE_KEYS.contains(&key.as_str()))
        || contains_forbidden_field(value)
    {
        return Err(RecordValidationError::Invalid);
    }
    let class = object
        .get("record_class")
        .and_then(Value::as_str)
        .ok_or(RecordValidationError::Invalid)?;
    let fields_key = match class {
        "resource-mutation" => "resource_mutation_fields",
        "resource-upgrade" => "resource_upgrade_fields",
        "rbac-change" => "rbac_change_fields",
        "session-connect" => "session_connect_fields",
        "route-admission" => "route_admission_fields",
        "resource-share" => "resource_share_fields",
        "broker-effect" => "broker_effect_fields",
        "process-effect" => "process_effect_fields",
        "state-reset" => "state_reset_fields",
        _ => return Err(RecordValidationError::Invalid),
    };
    if object
        .keys()
        .any(|key| key.ends_with("_fields") && key != fields_key)
        || object.get(fields_key).and_then(Value::as_object).is_none()
        || object.get("schema_version").and_then(Value::as_u64) != Some(1)
    {
        return Err(RecordValidationError::Invalid);
    }
    let previous = object
        .get("prev_hash")
        .and_then(Value::as_str)
        .ok_or(RecordValidationError::Invalid)?;
    let record_hash = object
        .get("record_hash")
        .and_then(Value::as_str)
        .ok_or(RecordValidationError::Invalid)?;
    if !valid_hash(previous) || !valid_hash(record_hash) {
        return Err(RecordValidationError::Invalid);
    }
    if expected_previous.is_some_and(|expected| expected != previous) {
        return Err(RecordValidationError::ChainBreak);
    }
    let canonical = json!({
        "ts_ms": object.get("ts_ms").ok_or(RecordValidationError::Invalid)?,
        "schema_version": object.get("schema_version").ok_or(RecordValidationError::Invalid)?,
        "zone": object.get("zone").ok_or(RecordValidationError::Invalid)?,
        "record_class": object.get("record_class").ok_or(RecordValidationError::Invalid)?,
        "operation_id": object.get("operation_id").ok_or(RecordValidationError::Invalid)?,
        "correlation_id": object.get("correlation_id").ok_or(RecordValidationError::Invalid)?,
        "trace_id": object.get("trace_id").ok_or(RecordValidationError::Invalid)?,
        "source": object.get("source").ok_or(RecordValidationError::Invalid)?,
        "prev_hash": object.get("prev_hash").ok_or(RecordValidationError::Invalid)?,
        fields_key: object.get(fields_key).ok_or(RecordValidationError::Invalid)?,
    });
    let canonical = serde_json::to_vec(&canonical).map_err(|_| RecordValidationError::Invalid)?;
    if record_hash != record_hash_for(previous, &canonical) {
        return Err(RecordValidationError::ChainBreak);
    }
    Ok(record_hash.to_owned())
}

fn valid_hash(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn genesis_hash() -> String {
    hash_bytes(b"d2b-audit-v3-genesis")
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn record_hash_for(previous: &str, canonical: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous.as_bytes());
    hasher.update(canonical);
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn is_segment_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix("audit-")
        .and_then(|value| value.strip_suffix(".jsonl"))
    else {
        return false;
    };
    digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Inspect audit segment inventory without exposing record contents.
pub(crate) fn audit_directory_health(path: &Path) -> Option<(u32, bool)> {
    let mut paths = fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(is_segment_name))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let count = paths.len().min(u32::MAX as usize) as u32;
    let mut validator = AuditStreamValidator::new(false);
    let mut clean = true;
    for path in paths {
        let Ok(file) = fs::File::open(path) else {
            clean = false;
            continue;
        };
        for line in std::io::BufReader::new(file).lines() {
            let Ok(line) = line else {
                clean = false;
                continue;
            };
            let output = validator.accept(&line);
            if output.contains("\"export_error\"") {
                clean = false;
            }
        }
    }
    Some((count, clean && !validator.had_break()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> String {
        let fields = json!({
            "event": "launch",
            "provider": "systemd",
            "domain": "system",
            "no_isolation": false,
            "execution_ref_digest": "sha256:exec",
            "process_uid": "uid",
            "outcome": "ok",
            "exit_class": null,
        });
        let previous = genesis_hash();
        let canonical = json!({
            "ts_ms": 1,
            "schema_version": 1,
            "zone": "work",
            "record_class": "process-effect",
            "operation_id": "op",
            "correlation_id": "corr",
            "trace_id": null,
            "source": "test",
            "prev_hash": previous,
            "process_effect_fields": fields,
        });
        let canonical_bytes = serde_json::to_vec(&canonical).unwrap();
        serde_json::json!({
            "ts_ms": 1,
            "schema_version": 1,
            "zone": "work",
            "record_class": "process-effect",
            "operation_id": "op",
            "correlation_id": "corr",
            "trace_id": null,
            "source": "test",
            "prev_hash": previous,
            "record_hash": record_hash_for(
                canonical["prev_hash"].as_str().unwrap(),
                &canonical_bytes,
            ),
            "process_effect_fields": canonical["process_effect_fields"],
        })
        .to_string()
    }

    #[test]
    fn validator_emits_inline_break_without_echoing_bad_record() {
        let mut validator = AuditStreamValidator::new(false);
        let valid = record();
        assert!(validator.accept(&valid).contains("\"record_class\""));
        let bad = r#"{"realm":"secret","path":"/private"}"#;
        let output = validator.accept(bad);
        assert!(output.contains("\"export_error\":\"record-invalid\""));
        assert!(!output.contains("secret"));
        assert!(!output.contains("/private"));
        assert!(validator.had_break());
    }

    #[test]
    fn validator_allows_a_selected_range_to_start_at_a_chain_boundary() {
        let mut validator = AuditStreamValidator::new(true);
        assert!(validator.accept(&record()).contains("\"record_class\""));
        assert!(!validator.had_break());
    }

    #[test]
    fn segment_boundaries_are_closed() {
        assert!(is_segment_name("audit-20240101000000000000.jsonl"));
        assert!(!is_segment_name("../audit-20240101000000000000.jsonl"));
    }
}
