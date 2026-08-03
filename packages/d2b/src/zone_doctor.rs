//! Read-only Zone health aggregation.

use std::{fs, path::Path};

use clap::Args;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    print_stdout,
};

/// Arguments for `d2b zone doctor`.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ZoneDoctorArgs {}

/// Closed Zone phase used in the doctor envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZonePhase {
    /// Bootstrap is incomplete.
    Pending,
    /// All mandatory components are ready.
    Ready,
    /// An optional or non-fatal component is unavailable.
    Degraded,
    /// The Zone cannot serve.
    Failed,
    /// No trusted phase observation exists.
    Unknown,
}

/// Check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// Healthy.
    Ok,
    /// Degraded but usable.
    Warn,
    /// Failed or quarantined.
    Error,
}

/// One stable doctor check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Check result.
    pub status: CheckStatus,
    /// Stable detail code, never raw provider output.
    pub detail: String,
}

/// Store health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StoreHealth {
    /// Store phase.
    pub phase: String,
    /// Current revision.
    pub revision: u64,
    /// Compaction floor.
    pub compaction_floor: u64,
    /// Active watch count.
    pub watch_active: u32,
}

/// Controller health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ControllerHealth {
    /// Closed handler name.
    pub handler: String,
    /// Closed phase.
    pub phase: String,
    /// Queue depth.
    pub queue_depth: u32,
    /// Opaque monotonic observation tick.
    pub last_reconciled_at: String,
}

/// Provider health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderHealth {
    /// Closed provider class.
    pub provider: String,
    /// Provider phase.
    pub phase: String,
    /// Component phases by closed component type.
    pub component_phases: std::collections::BTreeMap<String, String>,
}

/// Audit health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuditHealth {
    /// `ok`, `rate_limited`, or `unavailable`.
    pub phase: String,
    /// Segment count.
    pub segments: u32,
    /// Privileged drops.
    pub drop_privileged: u64,
    /// Total drops.
    pub drop_total: u64,
}

/// Telemetry health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TelemetryHealth {
    /// `ok`, `buffering`, or `unavailable`.
    pub phase: String,
    /// Dropped frame count.
    pub drop_total: u64,
}

/// Summary counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DoctorSummary {
    /// Number of successful checks.
    pub ok: u32,
    /// Number of warning checks.
    pub warn: u32,
    /// Number of error checks.
    pub error: u32,
}

/// Complete doctor report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneDoctorReport {
    /// Zone name.
    pub zone: String,
    /// Zone phase.
    pub zone_phase: ZonePhase,
    /// Store projection.
    pub store_health: StoreHealth,
    /// Controller projections.
    pub controllers: Vec<ControllerHealth>,
    /// Provider projections.
    pub providers: Vec<ProviderHealth>,
    /// Active and failed process counts.
    pub process_counts: ProcessCounts,
    /// Audit projection.
    pub audit: AuditHealth,
    /// Telemetry projection.
    pub telemetry: TelemetryHealth,
    /// Named checks.
    pub checks: Vec<DoctorCheck>,
    /// Check summary.
    pub summary: DoctorSummary,
}

/// Process counts without PID or process identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProcessCounts {
    /// Active processes.
    pub active: u32,
    /// Failed processes.
    pub failed: u32,
}

/// The non-suppressible warning shown for a user-only Host resource.
pub const NO_ISOLATION_WARNING: &str = "⚠ no isolation boundary (user domain)";

/// Render the Host posture annotation without exposing resource identity.
pub fn isolation_warning(isolation_posture: Option<&str>) -> Option<&'static str> {
    (isolation_posture == Some("none")).then_some(NO_ISOLATION_WARNING)
}

/// Inputs from trusted read-only Zone adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorInput {
    /// Zone phase.
    pub zone_phase: ZonePhase,
    /// Store status.
    pub store_health: StoreHealth,
    /// Controller statuses.
    pub controllers: Vec<ControllerHealth>,
    /// Provider statuses.
    pub providers: Vec<ProviderHealth>,
    /// Process counts.
    pub process_counts: ProcessCounts,
    /// Audit status.
    pub audit: AuditHealth,
    /// Telemetry status.
    pub telemetry: TelemetryHealth,
    /// Whether the latest schema catalog is consistent.
    pub schema_catalog_consistent: bool,
    /// Whether watch quota has headroom.
    pub watch_quota_headroom: bool,
    /// Whether the hash chain is clean.
    pub audit_hash_chain_clean: bool,
    /// Number of user-only Host resources.
    pub user_only_hosts: u32,
    /// Number of user-only Host statuses carrying the required posture.
    pub user_only_hosts_declared: u32,
}

/// Run the read-only Zone health command.
pub(crate) fn run(
    context: &ZoneContext,
    _args: &ZoneDoctorArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = context.invoke(
        "ZoneStatus",
        json!({
            "resourceRef": context.zone_ref(),
            "doctor": true,
        }),
        deadline,
        mode,
    )?;
    let report = report_from_value(&value, context.zone_name());
    if mode.is_json() {
        let mut rendered = render_json(&report).map_err(|_| {
            context.failure(
                "internal-error",
                "failed to render Zone doctor report",
                mode,
                1,
            )
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&render_human(&report));
    }
    Ok(exit_code(&report))
}

/// Adapt a bounded Zone status response into the fixed doctor report.
pub(crate) fn report_from_value(value: &Value, fallback_zone: &str) -> ZoneDoctorReport {
    let mut input = input_from_value(value);
    apply_optional_fixture_reads(&mut input);
    build_report(fallback_zone, input)
}

fn input_from_value(value: &Value) -> DoctorInput {
    let object = status_object(value);
    let mut store_health: StoreHealth =
        value_or_default(alias(object, &["store_health", "storeHealth", "store"]));
    let mut audit: AuditHealth = value_or_default(alias(object, &["audit"]));
    let mut telemetry: TelemetryHealth = value_or_default(alias(object, &["telemetry"]));
    if store_health.phase.is_empty() {
        store_health.phase = "unknown".to_owned();
    }
    if audit.phase.is_empty() {
        audit.phase = "unavailable".to_owned();
    }
    if telemetry.phase.is_empty() {
        telemetry.phase = "unavailable".to_owned();
    }

    let audit_value = alias(object, &["audit"]);
    let audit_hash_chain_clean = alias(object, &["audit_hash_chain_clean", "auditHashChainClean"])
        .and_then(Value::as_bool)
        .or_else(|| {
            audit_value
                .and_then(|value| value.get("hash_chain_clean"))
                .and_then(Value::as_bool)
        })
        .or_else(|| {
            audit_value
                .and_then(|value| value.get("defects"))
                .and_then(Value::as_array)
                .map(Vec::is_empty)
        })
        .unwrap_or(audit.phase == "ok");

    DoctorInput {
        zone_phase: parse_zone_phase(alias(object, &["zone_phase", "zonePhase", "phase"])),
        store_health,
        controllers: value_or_default(alias(object, &["controllers", "controller_status"])),
        providers: value_or_default(alias(object, &["providers", "provider_status"])),
        process_counts: value_or_default(alias(object, &["process_counts", "processCounts"])),
        audit,
        telemetry,
        schema_catalog_consistent: alias(
            object,
            &["schema_catalog_consistent", "schemaCatalogConsistent"],
        )
        .and_then(Value::as_bool)
        .unwrap_or(false),
        watch_quota_headroom: alias(object, &["watch_quota_headroom", "watchQuotaHeadroom"])
            .and_then(Value::as_bool)
            .unwrap_or(false),
        audit_hash_chain_clean,
        user_only_hosts: alias(object, &["user_only_hosts", "userOnlyHosts"])
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32,
        user_only_hosts_declared: alias(
            object,
            &["user_only_hosts_declared", "userOnlyHostsDeclared"],
        )
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32,
    }
}

fn status_object(value: &Value) -> &serde_json::Map<String, Value> {
    for key in ["doctor", "report", "status", "health"] {
        if let Some(object) = value.get(key).and_then(Value::as_object) {
            return object;
        }
    }
    value.as_object().unwrap_or_else(|| {
        static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> =
            std::sync::OnceLock::new();
        EMPTY.get_or_init(serde_json::Map::new)
    })
}

fn alias<'a>(object: &'a serde_json::Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name))
}

fn value_or_default<T>(value: Option<&Value>) -> T
where
    T: DeserializeOwned + Default,
{
    value
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn parse_zone_phase(value: Option<&Value>) -> ZonePhase {
    match value.and_then(Value::as_str).unwrap_or("unknown") {
        value if value.eq_ignore_ascii_case("pending") => ZonePhase::Pending,
        value if value.eq_ignore_ascii_case("ready") => ZonePhase::Ready,
        value if value.eq_ignore_ascii_case("degraded") => ZonePhase::Degraded,
        value
            if value.eq_ignore_ascii_case("failed")
                || value.eq_ignore_ascii_case("quarantined") =>
        {
            ZonePhase::Failed
        }
        _ => ZonePhase::Unknown,
    }
}

fn apply_optional_fixture_reads(input: &mut DoctorInput) {
    if let Some(value) = read_fixture(&["D2B_OTEL_SELF_METRICS_PATH", "D2B_OTEL_METRICS_PATH"]) {
        if let Some(phase) = value.get("phase").and_then(Value::as_str) {
            input.telemetry.phase = phase.to_owned();
        }
        if let Some(drop_total) = value.get("drop_total").and_then(Value::as_u64) {
            input.telemetry.drop_total = drop_total;
        }
    } else if manifest_observability_disabled() {
        input.telemetry.phase = "unavailable".to_owned();
    }

    if let Some(value) = read_fixture(&["D2B_AUDIT_STATUS_PATH", "D2B_ZONE_AUDIT_STATUS_PATH"]) {
        if let Some(phase) = value.get("phase").and_then(Value::as_str) {
            input.audit.phase = phase.to_owned();
        }
        input.audit.segments = value
            .get("segments")
            .and_then(Value::as_u64)
            .unwrap_or(u64::from(input.audit.segments))
            .min(u64::from(u32::MAX)) as u32;
        input.audit.drop_privileged = value
            .get("drop_privileged")
            .and_then(Value::as_u64)
            .unwrap_or(input.audit.drop_privileged);
        input.audit.drop_total = value
            .get("drop_total")
            .and_then(Value::as_u64)
            .unwrap_or(input.audit.drop_total);
        if let Some(defects) = value.get("defects").and_then(Value::as_array) {
            input.audit_hash_chain_clean = defects.is_empty();
        }
    } else if let Some(directory) =
        std::env::var_os("D2B_ZONE_AUDIT_DIR").or_else(|| std::env::var_os("D2B_AUDIT_DIR"))
    {
        let directory = Path::new(&directory);
        if let Some((segments, clean)) = audit_directory_health(directory) {
            input.audit.segments = segments;
            input.audit.phase = "ok".to_owned();
            input.audit_hash_chain_clean = clean;
        } else {
            input.audit.phase = "unavailable".to_owned();
            input.audit_hash_chain_clean = false;
        }
    }
}

fn read_fixture(names: &[&str]) -> Option<Value> {
    let path = names.iter().find_map(std::env::var_os)?;
    let bytes = fs::read(path).ok()?;
    if bytes.len() > crate::MAX_FRAME_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn manifest_observability_disabled() -> bool {
    let Some(path) = std::env::var_os("D2B_MANIFEST_PATH") else {
        return false;
    };
    read_fixture_value(Path::new(&path)).and_then(|value| {
        value
            .pointer("/_observability/enabled")
            .and_then(Value::as_bool)
    }) == Some(false)
}

fn read_fixture_value(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    (bytes.len() <= crate::MAX_FRAME_BYTES)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}

fn audit_directory_health(path: &Path) -> Option<(u32, bool)> {
    crate::zone_audit::audit_directory_health(path)
}

/// Build the fixed doctor check set.
pub fn build_report(zone: impl Into<String>, input: DoctorInput) -> ZoneDoctorReport {
    let zone = safe_zone(&zone.into());
    let input = sanitize_input(input);
    let mut checks = Vec::new();
    let store_revision_monotonic =
        input.store_health.revision >= input.store_health.compaction_floor;
    push_check(
        &mut checks,
        "store-revision-monotonic",
        store_revision_monotonic,
        false,
        if store_revision_monotonic {
            "ok"
        } else {
            "revision-before-compaction-floor"
        },
    );
    let controllers_ready = input
        .controllers
        .iter()
        .all(|controller| controller.phase == "ready");
    push_check(
        &mut checks,
        "controller-all-ready",
        controllers_ready,
        true,
        if controllers_ready {
            "ok"
        } else {
            "controller-degraded"
        },
    );
    let mandatory_providers_ready = input
        .providers
        .iter()
        .filter(|provider| provider.provider != "observability-otel")
        .all(|provider| provider.phase == "ready");
    push_check(
        &mut checks,
        "mandatory-providers-ready",
        mandatory_providers_ready,
        true,
        if mandatory_providers_ready {
            "ok"
        } else {
            "provider-degraded"
        },
    );
    let audit_healthy = input.audit.phase == "ok";
    let audit_rate_limited = input.audit.phase == "rate_limited";
    push_check(
        &mut checks,
        "audit-sink-healthy",
        audit_healthy,
        !audit_rate_limited,
        if audit_healthy {
            "ok"
        } else if audit_rate_limited {
            "audit-rate-limited"
        } else {
            "audit-unavailable"
        },
    );
    push_check(
        &mut checks,
        "otel-sink-reachable",
        matches!(input.telemetry.phase.as_str(), "ok" | "buffering"),
        false,
        match input.telemetry.phase.as_str() {
            "ok" => "ok",
            "buffering" => "telemetry-buffering",
            _ => "telemetry-unavailable",
        },
    );
    push_check(
        &mut checks,
        "schema-catalog-consistent",
        input.schema_catalog_consistent,
        true,
        if input.schema_catalog_consistent {
            "ok"
        } else {
            "schema-catalog-inconsistent"
        },
    );
    push_check(
        &mut checks,
        "watch-quota-headroom",
        input.watch_quota_headroom,
        false,
        if input.watch_quota_headroom {
            "ok"
        } else {
            "watch-quota-low"
        },
    );
    push_check(
        &mut checks,
        "audit-hash-chain-clean",
        input.audit_hash_chain_clean,
        true,
        if input.audit_hash_chain_clean {
            "ok"
        } else {
            "audit-hash-break"
        },
    );
    if input.user_only_hosts > 0 {
        push_check(
            &mut checks,
            "isolation-posture-declared",
            input.user_only_hosts == input.user_only_hosts_declared,
            true,
            if input.user_only_hosts == input.user_only_hosts_declared {
                "ok"
            } else {
                "isolation-posture-missing"
            },
        );
    }
    let summary = summarize(&checks);
    ZoneDoctorReport {
        zone,
        zone_phase: input.zone_phase,
        store_health: input.store_health,
        controllers: input.controllers,
        providers: input.providers,
        process_counts: input.process_counts,
        audit: input.audit,
        telemetry: input.telemetry,
        checks,
        summary,
    }
}

/// Return the command exit code.
pub fn exit_code(report: &ZoneDoctorReport) -> i32 {
    if report.summary.warn == 0
        && report.summary.error == 0
        && report.zone_phase == ZonePhase::Ready
    {
        0
    } else {
        1
    }
}

/// Render the report as a stable JSON envelope.
pub fn render_json(report: &ZoneDoctorReport) -> Result<String, serde_json::Error> {
    serde_json::to_string(report)
}

/// Render the bounded human doctor summary.
pub(crate) fn render_human(report: &ZoneDoctorReport) -> String {
    let mut output = format!(
        "zone doctor {}: phase={:?} ok={} warn={} error={}\n",
        report.zone,
        report.zone_phase,
        report.summary.ok,
        report.summary.warn,
        report.summary.error
    );
    for check in &report.checks {
        output.push_str(&format!(
            "{}\t{:?}\t{}\n",
            check.name, check.status, check.detail
        ));
    }
    output
}

fn push_check(checks: &mut Vec<DoctorCheck>, name: &str, passed: bool, error: bool, detail: &str) {
    checks.push(DoctorCheck {
        name: name.to_owned(),
        status: if passed {
            CheckStatus::Ok
        } else if error {
            CheckStatus::Error
        } else {
            CheckStatus::Warn
        },
        detail: detail.to_owned(),
    });
}

fn summarize(checks: &[DoctorCheck]) -> DoctorSummary {
    let mut summary = DoctorSummary {
        ok: 0,
        warn: 0,
        error: 0,
    };
    for check in checks {
        match check.status {
            CheckStatus::Ok => summary.ok += 1,
            CheckStatus::Warn => summary.warn += 1,
            CheckStatus::Error => summary.error += 1,
        }
    }
    summary
}

fn sanitize_input(mut input: DoctorInput) -> DoctorInput {
    input.store_health.phase = closed_token(
        &input.store_health.phase,
        &[
            "pending",
            "ready",
            "degraded",
            "failed",
            "quarantined",
            "unknown",
        ],
    );
    input.controllers = input
        .controllers
        .into_iter()
        .map(|mut controller| {
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
            controller.last_reconciled_at = safe_observation(&controller.last_reconciled_at);
            controller
        })
        .collect();
    input.providers = input
        .providers
        .into_iter()
        .map(|mut provider| {
            provider.provider = closed_token(
                &provider.provider,
                &[
                    "system-core",
                    "system-minijail",
                    "system-systemd",
                    "observability-otel",
                ],
            );
            provider.phase = closed_token(
                &provider.phase,
                &["pending", "ready", "degraded", "failed", "unknown"],
            );
            provider.component_phases = provider
                .component_phases
                .into_iter()
                .filter_map(|(component, phase)| {
                    if !["controller", "service", "worker"].contains(&component.as_str()) {
                        return None;
                    }
                    Some((
                        component,
                        closed_token(
                            &phase,
                            &["pending", "ready", "degraded", "failed", "unknown"],
                        ),
                    ))
                })
                .collect();
            provider
        })
        .collect();
    input.audit.phase = closed_token(&input.audit.phase, &["ok", "rate_limited", "unavailable"]);
    input.telemetry.phase =
        closed_token(&input.telemetry.phase, &["ok", "buffering", "unavailable"]);
    input
}

fn closed_token(value: &str, allowed: &[&str]) -> String {
    if allowed.contains(&value) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(user_only_hosts: u32, declared: u32) -> DoctorInput {
        DoctorInput {
            zone_phase: ZonePhase::Ready,
            store_health: StoreHealth {
                phase: "ready".to_owned(),
                revision: 10,
                compaction_floor: 1,
                watch_active: 1,
            },
            controllers: vec![ControllerHealth {
                handler: "provider".to_owned(),
                phase: "ready".to_owned(),
                queue_depth: 0,
                last_reconciled_at: "tick-1".to_owned(),
            }],
            providers: vec![ProviderHealth {
                provider: "system-core".to_owned(),
                phase: "ready".to_owned(),
                component_phases: std::collections::BTreeMap::new(),
            }],
            process_counts: ProcessCounts {
                active: 1,
                failed: 0,
            },
            audit: AuditHealth {
                phase: "ok".to_owned(),
                segments: 1,
                drop_privileged: 0,
                drop_total: 0,
            },
            telemetry: TelemetryHealth {
                phase: "unavailable".to_owned(),
                drop_total: 2,
            },
            schema_catalog_consistent: true,
            watch_quota_headroom: true,
            audit_hash_chain_clean: true,
            user_only_hosts,
            user_only_hosts_declared: declared,
        }
    }

    #[test]
    fn all_ready_is_zero_and_otel_absence_is_a_warning() {
        let report = build_report("work", input(0, 0));
        assert_eq!(exit_code(&report), 1);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "otel-sink-reachable")
        );
        let json = render_json(&report).unwrap();
        assert!(json.contains("\"zone_phase\""));
        assert!(!json.contains("\"broker_ready\""));
    }

    #[test]
    fn all_zone_phases_serialize_with_closed_vocabulary() {
        let phases = [
            (ZonePhase::Pending, "\"Pending\""),
            (ZonePhase::Ready, "\"Ready\""),
            (ZonePhase::Degraded, "\"Degraded\""),
            (ZonePhase::Failed, "\"Failed\""),
            (ZonePhase::Unknown, "\"Unknown\""),
        ];
        for (phase, expected) in phases {
            assert_eq!(serde_json::to_string(&phase).unwrap(), expected);
        }
    }

    #[test]
    fn isolation_check_is_omitted_without_user_hosts() {
        let report = build_report("work", input(0, 0));
        assert!(
            !report
                .checks
                .iter()
                .any(|check| check.name == "isolation-posture-declared")
        );
        let report = build_report("work", input(1, 0));
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "isolation-posture-declared")
        );
    }

    #[test]
    fn no_isolation_warning_is_not_suppressible_or_identity_bearing() {
        assert_eq!(isolation_warning(Some("none")), Some(NO_ISOLATION_WARNING));
        assert_eq!(isolation_warning(None), None);
        assert!(!NO_ISOLATION_WARNING.contains("Host/"));
        assert!(!NO_ISOLATION_WARNING.contains("User/"));
        assert!(!NO_ISOLATION_WARNING.contains('/'));
    }
}
