//! Bounded, redacted Zone support-bundle projection.

use clap::Args;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    print_stdout,
    zone_doctor::{self, ZoneDoctorReport, ZonePhase},
};

/// Arguments for `d2b zone support-bundle`.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ZoneSupportBundleArgs {}

/// Maximum status snapshots per resource type.
pub const MAX_SNAPSHOTS_PER_TYPE: usize = 32;
/// Maximum total status snapshots.
pub const MAX_TOTAL_SNAPSHOTS: usize = 512;
/// Maximum structured log entries.
pub const MAX_LOG_ENTRIES: usize = 2000;

/// Resource status fields safe for support output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ResourceStatusSnapshot {
    /// Opaque resource UID.
    pub uid: String,
    /// Zone identity.
    pub zone: String,
    /// Generation.
    pub generation: u64,
    /// Store revision.
    pub revision: u64,
    /// Last observed timestamp token.
    pub observed_at: String,
    /// Closed phase.
    pub phase: String,
    /// Bounded condition codes.
    pub conditions: Vec<String>,
    /// Observed generation.
    pub observed_generation: u64,
    /// Stable outcome code.
    pub outcome: Option<String>,
}

/// A bounded controller checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ControllerSnapshot {
    /// Closed handler.
    pub handler: String,
    /// Phase.
    pub phase: String,
    /// Queue depth.
    pub queue_depth: u32,
}

/// An audit segment inventory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuditSegmentInventory {
    /// Date-derived owned filename.
    pub filename: String,
    /// Segment size.
    pub bytes: u64,
    /// Record count.
    pub records: u64,
}

/// Provider self-metric summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OtelSummary {
    /// Provider phase.
    pub phase: String,
    /// Exported record count.
    pub exported: u64,
    /// Dropped record count.
    pub dropped: u64,
}

/// One already-redacted structured log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StructuredLogEntry {
    /// Stable event class.
    pub event: String,
    /// Stable outcome code.
    pub outcome: String,
    /// Opaque timestamp token.
    pub timestamp: String,
}

/// Output envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportBundle {
    /// Complete or partial.
    pub bundle_completeness: String,
    /// Doctor section.
    pub doctor: ZoneDoctorReport,
    /// Bounded resource status snapshots.
    pub resource_status: Vec<ResourceStatusSnapshot>,
    /// Controller snapshots.
    pub controllers: Vec<ControllerSnapshot>,
    /// Schema catalog names and versions only.
    pub schema_catalog: Vec<(String, String)>,
    /// Audit segment inventory.
    pub audit_segments: Vec<AuditSegmentInventory>,
    /// Optional OTEL summary.
    pub telemetry: Option<OtelSummary>,
    /// Redacted bounded logs.
    pub logs: Vec<StructuredLogEntry>,
}

/// Run the admin-only bounded support-bundle service.
pub(crate) fn run(
    context: &ZoneContext,
    _args: &ZoneSupportBundleArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::Support,
        "SupportService/GenerateBundle",
        "support-bundle",
        json!({
            "zone": context.zone_name(),
        }),
        deadline,
        OutputMode::Json,
    )?;
    let bundle = bundle_from_value(&value, context.zone_name())
        .map_err(|message| context.failure("exec-protocol-error", message, mode, 1))?;
    let partial = bundle.bundle_completeness == "partial";
    let rendered = render_ndjson(&bundle).map_err(|_| {
        context.failure("internal-error", "failed to render support bundle", mode, 1)
    })?;
    print_stdout(&rendered);
    Ok(if partial { 1 } else { 0 })
}

fn bundle_from_value(value: &Value, fallback_zone: &str) -> Result<SupportBundle, &'static str> {
    let root = ["bundle", "result"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_object))
        .or_else(|| value.as_object())
        .ok_or("support bundle response was not an object")?;
    let doctor_value = root
        .get("doctor")
        .cloned()
        .unwrap_or_else(|| Value::Object(root.clone()));
    let doctor = zone_doctor::report_from_value(&doctor_value, fallback_zone);
    let resource_status = root
        .get("resource_status")
        .or_else(|| root.get("resourceStatus"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(parse_resource_status)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let controllers = parse_vec(root, &["controllers", "controller_snapshots"]);
    let schema_catalog = parse_schema_catalog(root);
    let audit_segments = parse_vec(root, &["audit_segments", "auditSegments"]);
    let telemetry = root
        .get("telemetry")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let logs = parse_vec(root, &["logs", "structured_logs"]);
    let quarantined = root
        .get("quarantined")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || root
            .get("bundle_completeness")
            .or_else(|| root.get("bundleCompleteness"))
            .and_then(Value::as_str)
            == Some("partial")
        || doctor.zone_phase == ZonePhase::Failed
        || doctor.store_health.phase == "quarantined"
        || contains_quarantine_signal(&Value::Object(root.clone()));
    Ok(build_bundle(
        doctor,
        quarantined,
        resource_status,
        controllers,
        schema_catalog,
        audit_segments,
        telemetry,
        logs,
    ))
}

fn parse_vec<T>(root: &serde_json::Map<String, Value>, names: &[&str]) -> Vec<T>
where
    T: DeserializeOwned,
{
    names
        .iter()
        .find_map(|name| root.get(*name).and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_resource_status(value: &Value) -> Option<ResourceStatusSnapshot> {
    let object = value.as_object()?;
    if object.contains_key("uid") {
        return serde_json::from_value(value.clone()).ok();
    }
    let metadata = object.get("metadata").and_then(Value::as_object);
    let status = object.get("status").and_then(Value::as_object);
    Some(ResourceStatusSnapshot {
        uid: metadata
            .and_then(|metadata| metadata.get("uid"))
            .and_then(Value::as_str)
            .unwrap_or("opaque")
            .to_owned(),
        zone: metadata
            .and_then(|metadata| metadata.get("zone"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        generation: metadata
            .and_then(|metadata| metadata.get("generation"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        revision: metadata
            .and_then(|metadata| metadata.get("revision"))
            .and_then(Value::as_u64)
            .or_else(|| {
                status
                    .and_then(|status| status.get("revision"))
                    .and_then(Value::as_u64)
            })
            .unwrap_or(0),
        observed_at: metadata
            .and_then(|metadata| metadata.get("observed_at"))
            .or_else(|| metadata.and_then(|metadata| metadata.get("observedAt")))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        phase: status
            .and_then(|status| status.get("phase"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        conditions: status
            .and_then(|status| status.get("conditions"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        observed_generation: status
            .and_then(|status| status.get("observedGeneration"))
            .or_else(|| status.and_then(|status| status.get("observed_generation")))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        outcome: status
            .and_then(|status| status.get("outcome"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn parse_schema_catalog(root: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    let Some(items) = root
        .get("schema_catalog")
        .or_else(|| root.get("schemaCatalog"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(pair) = item.as_array() {
                return Some((
                    pair.first()?.as_str()?.to_owned(),
                    pair.get(1)?.as_str()?.to_owned(),
                ));
            }

            let object = item.as_object()?;
            Some((
                object.get("name")?.as_str()?.to_owned(),
                object.get("version")?.as_str()?.to_owned(),
            ))
        })
        .collect()
}

fn contains_quarantine_signal(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case("quarantined") && value.as_bool() == Some(true)
                || key.eq_ignore_ascii_case("phase")
                    && value
                        .as_str()
                        .is_some_and(|phase| phase.eq_ignore_ascii_case("quarantined"))
                || contains_quarantine_signal(value)
        }),
        Value::Array(values) => values.iter().any(contains_quarantine_signal),
        _ => false,
    }
}

/// Build a bounded support bundle.
#[allow(clippy::too_many_arguments)]
pub fn build_bundle(
    doctor: ZoneDoctorReport,
    quarantined: bool,
    resource_status: Vec<ResourceStatusSnapshot>,
    controllers: Vec<ControllerSnapshot>,
    schema_catalog: Vec<(String, String)>,
    audit_segments: Vec<AuditSegmentInventory>,
    telemetry: Option<OtelSummary>,
    logs: Vec<StructuredLogEntry>,
) -> SupportBundle {
    let mut by_type = std::collections::BTreeMap::<String, Vec<_>>::new();
    for snapshot in resource_status.into_iter().map(sanitize_resource_status) {
        let kind = snapshot
            .uid
            .split_once(':')
            .map(|(kind, _)| kind)
            .unwrap_or("resource")
            .to_owned();
        by_type.entry(kind).or_default().push(snapshot);
    }
    let mut resource_status = by_type
        .into_values()
        .flat_map(|mut snapshots| {
            if snapshots.len() > MAX_SNAPSHOTS_PER_TYPE {
                let keep_from = snapshots.len() - MAX_SNAPSHOTS_PER_TYPE;
                snapshots.drain(..keep_from);
            }
            snapshots
        })
        .collect::<Vec<_>>();
    if resource_status.len() > MAX_TOTAL_SNAPSHOTS {
        let keep_from = resource_status.len() - MAX_TOTAL_SNAPSHOTS;
        resource_status.drain(..keep_from);
    }
    let mut controllers = controllers
        .into_iter()
        .map(sanitize_controller)
        .collect::<Vec<_>>();
    controllers.truncate(64);
    let mut schema_catalog = schema_catalog
        .into_iter()
        .map(|(name, version)| (safe_token(&name), safe_token(&version)))
        .collect::<Vec<_>>();
    schema_catalog.truncate(128);
    let mut audit_segments = audit_segments
        .into_iter()
        .map(sanitize_audit_segment)
        .collect::<Vec<_>>();
    audit_segments.truncate(128);
    let mut logs = logs.into_iter().map(sanitize_log).collect::<Vec<_>>();
    if logs.len() > MAX_LOG_ENTRIES {
        let keep_from = logs.len() - MAX_LOG_ENTRIES;
        logs.drain(..keep_from);
    }
    let telemetry = telemetry.map(sanitize_otel);
    SupportBundle {
        bundle_completeness: if quarantined {
            "partial".to_owned()
        } else {
            "complete".to_owned()
        },
        doctor,
        resource_status,
        controllers,
        schema_catalog,
        audit_segments,
        telemetry,
        logs,
    }
}

/// Serialize the bounded bundle as one NDJSON document.
pub fn render_ndjson(bundle: &SupportBundle) -> Result<String, serde_json::Error> {
    let mut output = serde_json::to_string(bundle)?;
    output.push('\n');
    Ok(output)
}

fn sanitize_resource_status(mut snapshot: ResourceStatusSnapshot) -> ResourceStatusSnapshot {
    snapshot.uid = safe_opaque(&snapshot.uid);
    snapshot.zone = safe_zone(&snapshot.zone);
    snapshot.observed_at = safe_observation(&snapshot.observed_at);
    snapshot.phase = closed_token(
        &snapshot.phase,
        &["pending", "ready", "degraded", "failed", "unknown"],
    );
    snapshot.conditions = snapshot
        .conditions
        .into_iter()
        .filter_map(|condition| {
            let value = closed_token(
                &condition,
                &[
                    "store-quarantined",
                    "deletion-pending",
                    "pending-cleanup",
                    "telemetry-export-unavailable",
                    "cleanup-stalled",
                ],
            );
            (value != "unknown").then_some(value)
        })
        .collect();
    snapshot.outcome = snapshot.outcome.and_then(|outcome| {
        let value = closed_token(&outcome, &["ok", "error", "denied", "degraded"]);
        (value != "unknown").then_some(value)
    });
    snapshot
}

fn sanitize_controller(mut controller: ControllerSnapshot) -> ControllerSnapshot {
    controller.handler = closed_token(
        &controller.handler,
        &[
            "configuration",
            "api_catalog",
            "authz",
            "provider",
            "controller_registration",
            "ownership",
            "watch_maintenance",
            "ephemeral_cleanup",
            "zone_link",
            "budget",
            "store_lifecycle",
            "system_core_host",
            "system_core_user",
        ],
    );
    controller.phase = closed_token(
        &controller.phase,
        &["pending", "ready", "degraded", "failed", "unknown"],
    );
    controller
}

fn sanitize_audit_segment(mut segment: AuditSegmentInventory) -> AuditSegmentInventory {
    if !owned_segment_name(&segment.filename) {
        segment.filename = "audit-00000000000000000000.jsonl".to_owned();
    }
    segment
}

fn sanitize_otel(mut telemetry: OtelSummary) -> OtelSummary {
    telemetry.phase = closed_token(
        &telemetry.phase,
        &["ok", "buffering", "unavailable", "unknown"],
    );
    telemetry
}

fn sanitize_log(mut log: StructuredLogEntry) -> StructuredLogEntry {
    log.event = closed_token(
        &log.event,
        &[
            "collector-drain",
            "collector-export",
            "collector-startup",
            "forwarder-session",
            "journald-cycle",
            "unknown",
        ],
    );
    log.outcome = closed_token(&log.outcome, &["ok", "error", "dropped", "unknown"]);
    log.timestamp = safe_observation(&log.timestamp);
    log
}

fn closed_token(value: &str, allowed: &[&str]) -> String {
    if allowed.contains(&value) {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn safe_token(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn safe_opaque(value: &str) -> String {
    if value.contains(':')
        && !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| !byte.is_ascii_control() && byte != b'/')
    {
        value.to_owned()
    } else {
        "opaque".to_owned()
    }
}

fn safe_zone(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 63
        && value.bytes().enumerate().all(|(index, byte)| {
            (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && (index != 0 || byte.is_ascii_lowercase())
        })
    {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn safe_observation(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'T' | b'Z')
        })
    {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn owned_segment_name(name: &str) -> bool {
    let Some(digits) = name
        .strip_prefix("audit-")
        .and_then(|value| value.strip_suffix(".jsonl"))
    else {
        return false;
    };
    digits.len() == 20 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zone_doctor::{
        AuditHealth, ControllerHealth, DoctorInput, ProcessCounts, ProviderHealth, StoreHealth,
        TelemetryHealth, ZonePhase, build_report,
    };

    fn doctor() -> ZoneDoctorReport {
        build_report(
            "work",
            DoctorInput {
                zone_phase: ZonePhase::Ready,
                store_health: StoreHealth {
                    phase: "ready".to_owned(),
                    revision: 1,
                    compaction_floor: 0,
                    watch_active: 0,
                },
                controllers: vec![ControllerHealth {
                    handler: "provider".to_owned(),
                    phase: "ready".to_owned(),
                    queue_depth: 0,
                    last_reconciled_at: "tick".to_owned(),
                }],
                providers: vec![ProviderHealth {
                    provider: "system-core".to_owned(),
                    phase: "ready".to_owned(),
                    component_phases: Default::default(),
                }],
                process_counts: ProcessCounts {
                    active: 0,
                    failed: 0,
                },
                audit: AuditHealth {
                    phase: "ok".to_owned(),
                    segments: 1,
                    drop_privileged: 0,
                    drop_total: 0,
                },
                telemetry: TelemetryHealth {
                    phase: "ok".to_owned(),
                    drop_total: 0,
                },
                schema_catalog_consistent: true,
                watch_quota_headroom: true,
                audit_hash_chain_clean: true,
                user_only_hosts: 0,
                user_only_hosts_declared: 0,
            },
        )
    }

    #[test]
    fn quarantine_is_partial_and_status_contains_no_name_or_spec() {
        let bundle = build_bundle(
            doctor(),
            true,
            vec![ResourceStatusSnapshot {
                uid: "Provider:opaque".to_owned(),
                zone: "work".to_owned(),
                generation: 1,
                revision: 1,
                observed_at: "tick".to_owned(),
                phase: "degraded".to_owned(),
                conditions: vec!["store-quarantined".to_owned()],
                observed_generation: 1,
                outcome: None,
            }],
            Vec::new(),
            vec![("Provider".to_owned(), "v3".to_owned())],
            Vec::new(),
            None,
            Vec::new(),
        );
        let json = render_ndjson(&bundle).unwrap();
        assert!(json.contains("\"bundle_completeness\":\"partial\""));
        assert!(!json.contains("\"metadata\""));
        assert!(!json.contains("\"spec\""));
        assert!(!json.contains("\"metadata.name\""));
    }
}
