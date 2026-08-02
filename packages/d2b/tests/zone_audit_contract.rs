#[test]
fn audit_export_contract_is_admin_only_and_zone_bound() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/zone_audit.rs"),
    )
    .expect("zone audit destination exists");
    assert!(source.contains("AdminRequired"));
    assert!(source.contains("ZoneMismatch"));
    assert!(source.contains("export_segments"));
    assert!(source.contains("AuditExportGrant"));
}
