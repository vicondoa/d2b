use d2b_contracts_control::public_wire::{AuditResponse, PublicRequest};

#[test]
fn public_wire_round_trips_strict_request_and_audit_page() {
    let request = PublicRequest::Capabilities;
    let encoded = serde_json::to_string(&request).expect("request serializes");
    assert_eq!(encoded, "{\"kind\":\"capabilities\"}");

    let page = serde_json::json!({
        "entries": [],
        "complete": true
    });
    let decoded: AuditResponse = serde_json::from_value(page).expect("page decodes");
    assert!(decoded.complete);
}
