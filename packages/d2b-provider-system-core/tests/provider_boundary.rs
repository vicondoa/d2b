use d2b_provider_system_core::{ReconcileOutcome, ResourceReconciledAudit, SystemCoreManifest};

#[test]
fn manifest_is_empty_config_in_process_and_state_free() {
    let manifest = SystemCoreManifest::canonical();
    assert_eq!(manifest.artifact_id, "system-core");
    assert_eq!(manifest.components, ["host-controller", "user-controller"]);
    assert_eq!(manifest.resource_types, ["Host", "User"]);
    assert!(!manifest.declares_state_volume);
    assert!(SystemCoreManifest::validate_config(&serde_json::json!({})).is_ok());
    assert!(SystemCoreManifest::validate_config(&serde_json::json!({"extra": true})).is_err());
}

#[test]
fn host_and_user_audits_are_redacted_and_hyphenated() {
    let host = ResourceReconciledAudit::host("alice", ReconcileOutcome::Converged, "ready");
    let user = ResourceReconciledAudit::user("alice", ReconcileOutcome::Degraded, "drifted");
    let encoded = serde_json::to_string(&host).unwrap();
    assert!(encoded.contains("\"handler\":\"system-core-host\""));
    assert!(!encoded.contains("\"alice\""));
    assert_eq!(format!("{host:?}"), "ResourceReconciledAudit(<redacted>)");
    assert!(
        serde_json::to_string(&user)
            .unwrap()
            .contains("\"handler\":\"system-core-user\"")
    );
}
