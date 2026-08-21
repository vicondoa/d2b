//! Integration: `OpenHidrawSecurityKey` broker op - audit field shape
//! and wire round-trip.
//!
//! These tests exercise the public API only (no physical YubiKey and
//! no root privilege required): the audit field JSON shape (scrubbed
//! of raw device paths) and the `BrokerRequest`/`BrokerResponse` wire
//! contract round-trip via `serde_json`.

use d2b_contracts::types::VmId;
use d2b_contracts_broker::broker_wire::{
    BrokerRequest, BrokerResponse, OpenHidrawSecurityKeyRequest, OpenHidrawSecurityKeyResponse,
};
use d2b_contracts_resource::v3::ResourceRef;
use d2b_priv_broker::ops::audit_op::OperationFields;
use d2b_priv_broker::{fd_passing::recv_fds, protocol::send_json_frame_with_fds};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
use nix::unistd::close;
use std::os::fd::{AsRawFd, OwnedFd};

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
    assert!(
        json.contains("personal-dev"),
        "vm_id must appear in audit fields"
    );
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
/// opaque `selector_id`, admitted Device reference, and authority key).
#[test]
fn open_hidraw_security_key_request_wire_round_trips() {
    let device_ref = ResourceRef::parse("Device/yk5c-selector").expect("device ref");
    let expected_authority_key = d2b_contracts_broker::broker_wire::security_key_authority_binding(
        &device_ref,
        "yk5c-selector",
    );
    let request = BrokerRequest::OpenHidrawSecurityKey(OpenHidrawSecurityKeyRequest {
        vm_id: VmId::new("personal-dev"),
        selector_id: "yk5c-selector".to_owned(),
        device_ref: device_ref.clone(),
        authority_key: expected_authority_key.clone(),
        tracing_span_id: None,
    });
    let json = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(json["kind"], "OpenHidrawSecurityKey");
    assert_eq!(json["payload"]["vmId"], "personal-dev");
    assert_eq!(json["payload"]["selectorId"], "yk5c-selector");
    assert_eq!(
        json["payload"]["deviceRef"],
        device_ref.to_canonical_string()
    );
    assert_eq!(json["payload"]["authorityKey"], expected_authority_key);
    assert!(json["payload"].get("hidrawPath").is_none());

    let round_tripped: BrokerRequest =
        serde_json::from_value(json).expect("deserialize request round-trip");
    assert_eq!(round_tripped, request);
}

/// The `BrokerResponse::OpenHidrawSecurityKey` response body carries
/// only the resolved selector label and device class - never a raw
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

/// The production response transport carries the hidraw handle out-of-band
/// while keeping the JSON body path-free.
#[test]
fn open_hidraw_security_key_response_passes_one_fd_with_scm_rights() {
    let (sender, receiver): (OwnedFd, OwnedFd) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");
    let source = std::fs::File::open("/dev/null").expect("open fd fixture");
    let response = BrokerResponse::OpenHidrawSecurityKey(OpenHidrawSecurityKeyResponse {
        selector_resolved: "yk5c-selector".to_owned(),
        device_class: "hidraw-fido".to_owned(),
    });

    send_json_frame_with_fds(sender.as_raw_fd(), &response, &[source.as_raw_fd()])
        .expect("send SCM_RIGHTS response");
    let (frame, fds) = recv_fds(receiver.as_raw_fd()).expect("receive SCM_RIGHTS response");
    let decoded: BrokerResponse = serde_json::from_slice(&frame[4..]).expect("decode response");

    assert_eq!(decoded, response);
    assert_eq!(fds.len(), 1);
    for fd in fds {
        close(fd).expect("close received fd");
    }
}
