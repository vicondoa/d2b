#![cfg(feature = "layer1-bootstrap")]

// The layer1-bootstrap profile's mirror of the contracts-crate audit
// labels: a label that drifts here shows up in audit records written by
// this profile only. Mirrors
// `d2b_contracts_broker::broker_wire::broker_caller_role_display_uses_stable_audit_labels`
// (which pins the full five-label set on the contracts side).
use d2b_broker::bootstrap::wire::CallerRole;

#[test]
fn caller_role_display_mirrors_the_contract_audit_labels() {
    assert_eq!(CallerRole::RootUid { uid: 0 }.for_display(), "d2b-root");
    assert_eq!(CallerRole::AdminUid { uid: 0 }.for_display(), "d2b-admin");
    assert_eq!(
        CallerRole::NotAuthorized.for_display(),
        "d2b-not-authorized"
    );
}