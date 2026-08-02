#[path = "../src/zone_audit.rs"]
mod zone_audit;

#[test]
fn audit_export_grant_cannot_be_used_for_another_zone() {
    let grant = zone_audit::AuditExportGrant::admit(true, "work").unwrap();
    let request = zone_audit::AuditExportRequest {
        zone: "other".to_owned(),
        after: None,
        before: None,
    };
    assert!(matches!(
        zone_audit::export_ndjson(&grant, &request, "/nonexistent"),
        Err(zone_audit::ZoneAuditError::ZoneMismatch)
    ));
}
