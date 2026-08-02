//! Read-only Zone health aggregation.

use serde::Serialize;

/// Closed Zone phase used in the doctor envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Check result.
    pub status: CheckStatus,
    /// Stable detail code, never raw provider output.
    pub detail: String,
}

/// Store health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderHealth {
    /// Closed provider class.
    pub provider: String,
    /// Provider phase.
    pub phase: String,
    /// Component phases by closed component type.
    pub component_phases: std::collections::BTreeMap<String, String>,
}

/// Audit health projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelemetryHealth {
    /// `ok`, `buffering`, or `unavailable`.
    pub phase: String,
    /// Dropped frame count.
    pub drop_total: u64,
}

/// Summary counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DoctorSummary {
    /// Number of successful checks.
    pub ok: u32,
    /// Number of warning checks.
    pub warn: u32,
    /// Number of error checks.
    pub error: u32,
}

/// Complete doctor report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// Build the fixed doctor check set.
pub fn build_report(zone: impl Into<String>, input: DoctorInput) -> ZoneDoctorReport {
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
    push_check(
        &mut checks,
        "audit-sink-healthy",
        input.audit.phase == "ok",
        true,
        if input.audit.phase == "ok" {
            "ok"
        } else {
            "audit-unavailable"
        },
    );
    push_check(
        &mut checks,
        "otel-sink-reachable",
        matches!(input.telemetry.phase.as_str(), "ok" | "buffering"),
        false,
        if input.telemetry.phase == "ok" {
            "ok"
        } else {
            "telemetry-buffering"
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
        zone: zone.into(),
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
