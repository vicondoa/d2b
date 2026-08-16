use d2b_provider_runtime_qemu_media::{
    AuditEventKind, AuditOutcome, AuditRecord, MetricOutcome, QmpOperation, SpanKind,
    TelemetryFrame,
};

#[test]
fn audit_events_are_bounded_and_redacted() {
    let record = AuditRecord::new(
        AuditEventKind::QmpReady,
        AuditOutcome::Success,
        "corp",
        "Guest/media-vm",
        "operation-1",
    );
    let json = record.to_json().unwrap();
    assert!(json.contains("qmp-ready"));
    assert!(!json.contains("/run"));
    assert!(!json.contains("argv"));
    assert!(!json.contains("socket"));
    assert!(!json.contains("pid"));
}

#[test]
fn every_audit_kind_has_a_stable_wire_name() {
    for kind in AuditEventKind::ALL {
        assert!(!kind.as_str().is_empty());
        assert!(kind.as_str().starts_with("guest/"));
    }
}

#[test]
fn telemetry_labels_are_closed_and_zone_is_a_resource_attribute() {
    let frame = TelemetryFrame::new("corp", MetricOutcome::Success);
    assert!(frame.validate().is_ok());
    assert!(
        frame
            .resource_attributes()
            .iter()
            .any(|field| field.key == "d2b.zone")
    );
    assert!(frame.metric_labels().iter().all(|field| {
        ["provider", "outcome", "operation", "phase", "dep_type"].contains(&field.key)
    }));
    assert!(TelemetryFrame::validate_field("vm", "media-vm").is_err());
    assert!(TelemetryFrame::validate_field("outcome", "caller-controlled").is_err());
}

#[test]
fn span_attributes_use_fixed_semantic_values() {
    let span = TelemetryFrame::span(
        SpanKind::QmpCommand,
        QmpOperation::QueryStatus,
        MetricOutcome::Success,
    );
    assert!(span.validate().is_ok());
    assert!(TelemetryFrame::validate_span_attribute("guest", "Guest/media-vm").is_err());
}
