//! Bounded, redacted Zone support-bundle projection.

use serde::Serialize;

use crate::zone_doctor::ZoneDoctorReport;

/// Maximum status snapshots per resource type.
pub const MAX_SNAPSHOTS_PER_TYPE: usize = 32;
/// Maximum total status snapshots.
pub const MAX_TOTAL_SNAPSHOTS: usize = 512;
/// Maximum structured log entries.
pub const MAX_LOG_ENTRIES: usize = 2000;

/// Resource status fields safe for support output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControllerSnapshot {
    /// Closed handler.
    pub handler: String,
    /// Phase.
    pub phase: String,
    /// Queue depth.
    pub queue_depth: u32,
}

/// An audit segment inventory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditSegmentInventory {
    /// Date-derived owned filename.
    pub filename: String,
    /// Segment size.
    pub bytes: u64,
    /// Record count.
    pub records: u64,
}

/// Provider self-metric summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OtelSummary {
    /// Provider phase.
    pub phase: String,
    /// Exported record count.
    pub exported: u64,
    /// Dropped record count.
    pub dropped: u64,
}

/// One already-redacted structured log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StructuredLogEntry {
    /// Stable event class.
    pub event: String,
    /// Stable outcome code.
    pub outcome: String,
    /// Opaque timestamp token.
    pub timestamp: String,
}

/// Output envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupportBundle {
    /// Complete or partial.
    pub bundle_completeness: &'static str,
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

/// Build a bounded support bundle.
#[allow(clippy::too_many_arguments)]
pub fn build_bundle(
    doctor: ZoneDoctorReport,
    quarantined: bool,
    mut resource_status: Vec<ResourceStatusSnapshot>,
    mut controllers: Vec<ControllerSnapshot>,
    mut schema_catalog: Vec<(String, String)>,
    mut audit_segments: Vec<AuditSegmentInventory>,
    telemetry: Option<OtelSummary>,
    mut logs: Vec<StructuredLogEntry>,
) -> SupportBundle {
    resource_status.truncate(MAX_TOTAL_SNAPSHOTS);
    let mut by_type = std::collections::BTreeMap::<String, usize>::new();
    resource_status.retain(|snapshot| {
        let kind = snapshot
            .uid
            .split_once(':')
            .map(|(kind, _)| kind)
            .unwrap_or("resource");
        let count = by_type.entry(kind.to_owned()).or_default();
        if *count >= MAX_SNAPSHOTS_PER_TYPE {
            false
        } else {
            *count += 1;
            true
        }
    });
    controllers.truncate(64);
    schema_catalog.truncate(128);
    audit_segments.truncate(128);
    logs.truncate(MAX_LOG_ENTRIES);
    SupportBundle {
        bundle_completeness: if quarantined { "partial" } else { "complete" },
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
