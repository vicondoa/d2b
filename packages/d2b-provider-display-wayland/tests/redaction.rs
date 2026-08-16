use d2b_provider_display_wayland::{DisplayAuditKind, DisplayAuditOutcome, DisplayAuditRecord};

#[test]
fn audit_uses_digests_for_session_identity() {
    let record = DisplayAuditRecord::new(
        DisplayAuditKind::SessionCreated,
        DisplayAuditOutcome::Success,
        "dev",
        "window-title-canary",
        "alice",
        "operation",
    );
    assert!(!record.to_wire_record().contains("window-title-canary"));
    assert!(!record.to_wire_record().contains("alice"));
}
