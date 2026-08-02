#[path = "../src/host_reconciler.rs"]
mod host_reconciler;
#[path = "../src/host_status.rs"]
mod host_status;

use d2b_contracts::v3::{ResourceRef, host::HostSpec};

#[test]
fn status_projection_declares_user_only_posture() {
    let user = ResourceRef::parse("User/alice").unwrap();
    let status = host_reconciler::reconcile_status(&HostSpec::user_only(user).unwrap());
    assert!(status.is_no_isolation());
    assert_eq!(
        host_reconciler::isolation_posture(
            &HostSpec::user_only(ResourceRef::parse("User/alice").unwrap()).unwrap()
        ),
        Some(d2b_contracts::v3::host::IsolationPosture::NoIsolation)
    );
    assert_eq!(
        status.isolation_posture,
        Some(d2b_contracts::v3::host::IsolationPosture::NoIsolation)
    );
    assert_eq!(
        status.isolation_posture_message,
        Some(host_status::ISOLATION_POSTURE_MESSAGE)
    );
}

#[test]
fn operator_cannot_supply_or_clear_posture() {
    for value in [
        serde_json::json!({"isolationPosture": "none"}),
        serde_json::json!({"isolationPosture": null}),
        serde_json::json!({"isolationPostureMessage": "suppressed"}),
    ] {
        assert_eq!(
            host_reconciler::reject_operator_status_fields(&value),
            Err(host_reconciler::HostStatusInputError::OperatorSuppliedField)
        );
    }
    assert_eq!(
        host_reconciler::reject_operator_status_fields(&serde_json::json!({"phase": "Ready"})),
        Ok(())
    );
}
