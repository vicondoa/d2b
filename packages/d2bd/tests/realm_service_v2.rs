use d2b_contracts::v2_services::method_spec;

#[test]
fn daemon_uses_the_complete_generated_realm_service_surface() {
    for method in [
        "Bootstrap",
        "Enroll",
        "ResolveRoute",
        "AuthorizeShortcut",
        "RevokeShortcut",
        "ReportShortcutClose",
        "Inspect",
        "Cancel",
    ] {
        assert!(
            method_spec("d2b.realm.v2", "RealmService", method).is_some(),
            "missing generated realm method {method}"
        );
    }
}

#[test]
fn retired_realm_facade_cannot_restore_a_bypass() {
    let source = include_str!("../src/realm_stubs.rs");
    for retired in [
        "struct ApiFrontend",
        "struct LocalExecutor",
        "struct TargetResolver",
        "ProtocolCodec",
        "with_local_default()",
    ] {
        assert!(
            !source.contains(retired),
            "retired realm bypass remains: {retired}"
        );
    }
    assert!(source.contains("RealmServiceProcess"));
    assert!(source.contains("ComponentSession"));
}
