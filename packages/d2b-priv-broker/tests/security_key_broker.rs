//! Integration: `OpenHidrawSecurityKey` broker op — audit field shape
//! and wire round-trip.
//!
//! These tests exercise the public API only (no physical YubiKey and
//! no root privilege required): the audit field JSON shape (scrubbed
//! of raw device paths) and the `BrokerRequest`/`BrokerResponse` wire
//! contract round-trip via `serde_json`.

use d2b_contracts::broker_wire::{
    BrokerRequest, BrokerResponse, OpenHidrawSecurityKeyRequest, OpenHidrawSecurityKeyResponse,
};
use d2b_contracts::types::VmId;
use d2b_priv_broker::ops::audit_op::OperationFields;

/// Audit fields for `OpenHidrawSecurityKey` carry scrubbed metadata
/// only (no raw device path).
#[test]
fn open_hidraw_security_key_audit_fields_scrubbed() {
    let fields = OperationFields::OpenHidrawSecurityKey {
        vm_id: "personal-dev".to_owned(),
        selector_id: "yk5c-selector".to_owned(),
        device_class: "hidraw-fido".to_owned(),
        resolved: true,
    };
    let json = serde_json::to_string(&fields).expect("serialize OpenHidrawSecurityKey fields");
    assert!(json.contains("personal-dev"), "vm_id must appear in audit fields");
    assert!(json.contains("hidraw-fido"), "device_class must appear");
    // Raw hidraw paths must never appear in the audit JSON.
    assert!(
        !json.contains("/dev/hidraw"),
        "raw device path must not appear in audit fields"
    );
}

/// `OperationFields::from_operation_value` round-trips
/// `OpenHidrawSecurityKey` fields from a raw JSON value (the shape the
/// runtime dispatcher hands it).
#[test]
fn open_hidraw_security_key_audit_round_trips_from_value() {
    let value = serde_json::json!({
        "vm_id": "work-aad",
        "selector_id": "yk5c",
        "device_class": "hidraw-fido",
        "resolved": true
    });
    let fields = OperationFields::from_operation_value("OpenHidrawSecurityKey", value)
        .expect("from_operation_value should parse OpenHidrawSecurityKey fields");
    match fields {
        OperationFields::OpenHidrawSecurityKey {
            vm_id,
            selector_id,
            device_class,
            resolved,
        } => {
            assert_eq!(vm_id, "work-aad");
            assert_eq!(selector_id, "yk5c");
            assert_eq!(device_class, "hidraw-fido");
            assert!(resolved);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

/// The `BrokerRequest::OpenHidrawSecurityKey` variant round-trips
/// through the tagged wire envelope (`kind`/`payload`) and never
/// serializes a raw device path (the daemon supplies only `vm_id` +
/// opaque `selector_id`).
#[test]
fn open_hidraw_security_key_request_wire_round_trips() {
    let request = BrokerRequest::OpenHidrawSecurityKey(OpenHidrawSecurityKeyRequest {
        vm_id: VmId::new("personal-dev"),
        selector_id: "yk5c-selector".to_owned(),
        tracing_span_id: None,
    });
    let json = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(json["kind"], "OpenHidrawSecurityKey");
    assert_eq!(json["payload"]["vmId"], "personal-dev");
    assert_eq!(json["payload"]["selectorId"], "yk5c-selector");
    assert!(json["payload"].get("hidrawPath").is_none());

    let round_tripped: BrokerRequest =
        serde_json::from_value(json).expect("deserialize request round-trip");
    assert_eq!(round_tripped, request);
}

/// The `BrokerResponse::OpenHidrawSecurityKey` response body carries
/// only the resolved selector label and device class — never a raw
/// path (the fd itself travels out-of-band via `SCM_RIGHTS`).
#[test]
fn open_hidraw_security_key_response_wire_round_trips() {
    let response = BrokerResponse::OpenHidrawSecurityKey(OpenHidrawSecurityKeyResponse {
        selector_resolved: "yk5c-selector:hidraw3".to_owned(),
        device_class: "hidraw-fido".to_owned(),
    });
    let json = serde_json::to_value(&response).expect("serialize response");
    assert_eq!(json["kind"], "OpenHidrawSecurityKey");
    assert_eq!(json["payload"]["selectorResolved"], "yk5c-selector:hidraw3");
    assert_eq!(json["payload"]["deviceClass"], "hidraw-fido");

    let round_tripped: BrokerResponse =
        serde_json::from_value(json).expect("deserialize response round-trip");
    assert_eq!(round_tripped, response);
}
