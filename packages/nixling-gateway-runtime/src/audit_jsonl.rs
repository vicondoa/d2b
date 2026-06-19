//! Durable gateway audit JSONL sink.
//!
//! Records one redacted [`GatewayAuditEvent`](nixling_gateway::GatewayAuditEvent)
//! per line. Each line carries `prev_hash` + `record_hash` (SHA-256 over the
//! canonical JSON fields excluding `record_hash`) so truncation/reordering can
//! be detected by the gateway daemon's reconciliation/audit tooling. The sink
//! prunes old daily files at append boundaries according to the configured
//! retention floor.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nixling_gateway::{GatewayAudit, GatewayAuditEvent, GatewayError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Default gateway audit retention floor in days.
pub const DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS: u64 = 14;

/// Durable JSONL audit sink rooted at `dir`.
pub struct JsonlGatewayAudit {
    dir: PathBuf,
    retention_days: u64,
    clock_day: Box<dyn Fn() -> u64 + Send + Sync>,
    state: Mutex<AuditState>,
}

#[derive(Debug, Default)]
struct AuditState {
    day: Option<u64>,
    prev_hash: Option<String>,
}

impl JsonlGatewayAudit {
    /// Build a sink using a caller-supplied day clock (days since Unix epoch).
    pub fn new(
        dir: impl Into<PathBuf>,
        retention_days: u64,
        clock_day: Box<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            dir: dir.into(),
            retention_days,
            clock_day,
            state: Mutex::new(AuditState::default()),
        }
    }

    fn file_for_day(&self, day: u64) -> PathBuf {
        self.dir.join(format!("gateway-day-{day}.jsonl"))
    }

    fn prune_old(&self) -> std::io::Result<()> {
        if self.retention_days == 0 {
            return Ok(());
        }
        let min_day = (self.clock_day)().saturating_sub(self.retention_days);
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let Some(day) = audit_day(entry.file_name().to_string_lossy().as_ref()) else {
                continue;
            };
            if day < min_day {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
}

impl GatewayAudit for JsonlGatewayAudit {
    fn record(&self, event: GatewayAuditEvent) -> Result<(), GatewayError> {
        fs::create_dir_all(&self.dir).map_err(|_| GatewayError::AuditUnavailable)?;
        self.prune_old()
            .map_err(|_| GatewayError::AuditUnavailable)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| GatewayError::AuditUnavailable)?;
        let day = (self.clock_day)();
        let path = self.file_for_day(day);
        if state.day != Some(day) {
            state.day = Some(day);
            state.prev_hash =
                last_record_hash(&path).map_err(|_| GatewayError::AuditUnavailable)?;
        }
        let prev_hash = state.prev_hash.clone();
        let mut body = event_json(event, prev_hash);
        let hash = hash_json(&body).map_err(|_| GatewayError::AuditUnavailable)?;
        body["record_hash"] = Value::String(hash);
        let line = serde_json::to_string(&body).map_err(|_| GatewayError::AuditUnavailable)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| GatewayError::AuditUnavailable)?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|_| GatewayError::AuditUnavailable)?;
        state.prev_hash = body
            .get("record_hash")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(())
    }
}

fn event_json(event: GatewayAuditEvent, prev_hash: Option<String>) -> Value {
    json!({
        "kind": format!("{:?}", event.kind),
        "envelope": event.envelope,
        "session_id": event.session_id.map(|s| s.to_string()),
        "state": event.state.map(|s| format!("{:?}", s)),
        "error_slug": event.error_slug,
        "prev_hash": prev_hash,
    })
}

fn hash_json(value: &Value) -> serde_json::Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let digest = Sha256::digest(bytes);
    Ok(hex(&digest))
}

fn last_record_hash(path: &Path) -> std::io::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut s = String::new();
    fs::File::open(path)?.read_to_string(&mut s)?;
    Ok(s.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .and_then(|v| {
            v.get("record_hash")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }))
}

fn audit_day(file_name: &str) -> Option<u64> {
    file_name
        .strip_prefix("gateway-day-")?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{
        AuthzDecision, NodeId, OperationId, PrincipalId, RealmPath, WorkloadId,
    };
    use nixling_gateway::{GatewayAuditKind, SessionState, display_envelope};

    fn event(kind: GatewayAuditKind, op: &str) -> GatewayAuditEvent {
        GatewayAuditEvent {
            kind,
            envelope: display_envelope(
                OperationId::parse(op).unwrap(),
                RealmPath::local(),
                PrincipalId::parse("alice").unwrap(),
                NodeId::parse("gateway").unwrap(),
                WorkloadId::parse("demo").unwrap(),
                AuthzDecision::Allow,
            ),
            session_id: Some(nixling_gateway::DisplaySessionId::new("s1")),
            state: Some(SessionState::Running),
            error_slug: None,
        }
    }

    #[test]
    fn jsonl_records_hash_chain_and_redacts_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let sink = JsonlGatewayAudit::new(
            dir.path(),
            DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS,
            Box::new(|| 10),
        );
        sink.record(event(GatewayAuditKind::DisplaySessionOpenAdmitted, "op-1"))
            .unwrap();
        sink.record(event(GatewayAuditKind::DisplaySessionRunning, "op-2"))
            .unwrap();
        let body = fs::read_to_string(dir.path().join("gateway-day-10.jsonl")).unwrap();
        let lines: Vec<Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0]["prev_hash"].is_null());
        assert_eq!(lines[1]["prev_hash"], lines[0]["record_hash"]);
        assert_eq!(lines[0]["error_slug"], Value::Null);
        assert_eq!(lines[1]["error_slug"], Value::Null);
    }

    #[test]
    fn prunes_files_older_than_retention_floor() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("gateway-day-1.jsonl"), "{}\n").unwrap();
        fs::write(dir.path().join("gateway-day-9.jsonl"), "{}\n").unwrap();
        let sink = JsonlGatewayAudit::new(dir.path(), 2, Box::new(|| 10));
        sink.record(event(GatewayAuditKind::DisplaySessionRunning, "op-1"))
            .unwrap();
        assert!(!dir.path().join("gateway-day-1.jsonl").exists());
        assert!(dir.path().join("gateway-day-9.jsonl").exists());
        assert!(dir.path().join("gateway-day-10.jsonl").exists());
    }
}
