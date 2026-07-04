//! Contract tests for the USB security-key proxy feature wire contracts.
//!
//! # Scope
//!
//! Type 4 contract tests: Rust assertions over the security-key DTO
//! shapes, serde round-trips, deny_unknown_fields enforcement, and the
//! broker capability set.
//!
//! Type 5 policy lints: verify that the new broker operations appear in
//! the privileges matrix, the dispositions doc, and the W3 capability
//! set.

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use d2b_contracts::{
    BrokerCapabilities,
    broker_wire::BrokerRequest,
    public_wire::{PublicRequest, PublicResponse},
    security_key::{
        SecurityKeyCancelRequest, SecurityKeyCancelResponse, SecurityKeyDeviceLabel,
        SecurityKeyDeviceStatus, SecurityKeyEvent, SecurityKeyLeaseState, SecurityKeyOpenDeviceRequest,
        SecurityKeySession, SecurityKeySessionId, SecurityKeySessionResult, SecurityKeySessionsResponse,
        SecurityKeyStatusResponse, SecurityKeyVmSessionState, SecurityKeyVmState,
        SecurityKeyApplyUdevRulesRequest,
    },
};
use d2b_core::privileges_w3::W3BrokerOperation;

// ---------------------------------------------------------------------------
// Type 4: DTO serde round-trips + deny_unknown_fields
// ---------------------------------------------------------------------------

#[test]
fn security_key_status_response_round_trips() {
    let resp = SecurityKeyStatusResponse {
        host_proxy_enabled: true,
        devices: vec![SecurityKeyDeviceStatus {
            label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            vendor_id: 0x1050,
            product_id: 0x0407,
            serial: None,
            present: true,
            broker_accessible: true,
            usbip_bound: false,
        }],
        current_lease: Some(SecurityKeyLeaseState {
            vm: "corp-vm".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            session_id: Some(SecurityKeySessionId::new("sk-corp-vm-1")),
            acquired_at: "2026-07-03T22:00:00Z".to_owned(),
        }),
        vm_states: vec![SecurityKeyVmState {
            vm: "corp-vm".to_owned(),
            enabled: true,
            virtual_device_present: true,
            session_state: SecurityKeyVmSessionState::Active,
        }],
    };
    let json = serde_json::to_string(&resp).expect("serialize");
    let decoded: SecurityKeyStatusResponse = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(resp, decoded);
}

#[test]
fn security_key_status_response_rejects_unknown_fields() {
    let json = r#"{
        "hostProxyEnabled": true,
        "devices": [],
        "currentLease": null,
        "vmStates": [],
        "unknownFutureField": "x"
    }"#;
    let err = serde_json::from_str::<SecurityKeyStatusResponse>(json)
        .expect_err("unknown field must fail");
    assert!(
        err.to_string().contains("unknown field"),
        "expected unknown-field error, got: {err}"
    );
}

#[test]
fn security_key_sessions_response_round_trips() {
    let resp = SecurityKeySessionsResponse {
        sessions: vec![SecurityKeySession {
            session_id: SecurityKeySessionId::new("sk-corp-vm-1"),
            vm: "corp-vm".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            rp_id: Some("login.example.com".to_owned()),
            result: SecurityKeySessionResult::Success,
            started_at: "2026-07-03T22:00:00Z".to_owned(),
            ended_at: Some("2026-07-03T22:00:05Z".to_owned()),
        }],
    };
    let json = serde_json::to_string(&resp).expect("serialize");
    let decoded: SecurityKeySessionsResponse = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(resp, decoded);
}

#[test]
fn security_key_sessions_rp_id_omitted_when_none() {
    let session = SecurityKeySession {
        session_id: SecurityKeySessionId::new("sk-corp-vm-1"),
        vm: "corp-vm".to_owned(),
        device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
        rp_id: None,
        result: SecurityKeySessionResult::InProgress,
        started_at: "2026-07-03T22:00:00Z".to_owned(),
        ended_at: None,
    };
    let json = serde_json::to_string(&session).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert!(
        !value.as_object().unwrap().contains_key("rpId"),
        "rpId must be omitted when None, got: {json}"
    );
    assert!(
        !value.as_object().unwrap().contains_key("endedAt"),
        "endedAt must be omitted when None, got: {json}"
    );
}

#[test]
fn security_key_cancel_request_round_trips() {
    // with session ID
    let req = SecurityKeyCancelRequest {
        session_id: Some(SecurityKeySessionId::new("sk-corp-vm-1")),
        cancel_current: false,
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: SecurityKeyCancelRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);

    // cancel_current mode
    let req2 = SecurityKeyCancelRequest {
        session_id: None,
        cancel_current: true,
    };
    let json2 = serde_json::to_string(&req2).expect("serialize");
    let decoded2: SecurityKeyCancelRequest = serde_json::from_str(&json2).expect("deserialize");
    assert_eq!(req2, decoded2);
}

#[test]
fn security_key_events_round_trip_all_variants() {
    let events = vec![
        SecurityKeyEvent::SessionStarted {
            session_id: SecurityKeySessionId::new("sk-1"),
            vm: "vm1".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
        },
        SecurityKeyEvent::SessionSucceeded {
            session_id: SecurityKeySessionId::new("sk-1"),
            vm: "vm1".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            ended_at: "2026-01-01T00:00:05Z".to_owned(),
        },
        SecurityKeyEvent::SessionFailed {
            session_id: SecurityKeySessionId::new("sk-2"),
            vm: "vm1".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            result: SecurityKeySessionResult::Timeout,
            ended_at: "2026-01-01T00:00:30Z".to_owned(),
        },
        SecurityKeyEvent::SessionCancelled {
            session_id: SecurityKeySessionId::new("sk-3"),
            vm: "vm2".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            ended_at: "2026-01-01T00:00:10Z".to_owned(),
        },
        SecurityKeyEvent::DeviceRemoved {
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            interrupted_session_id: Some(SecurityKeySessionId::new("sk-3")),
        },
        SecurityKeyEvent::DeviceReinserted {
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
        },
        SecurityKeyEvent::SessionQueued {
            session_id: SecurityKeySessionId::new("sk-4"),
            vm: "vm2".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yk-primary"),
            queued_at: "2026-01-01T00:00:01Z".to_owned(),
            blocking_vm: "vm1".to_owned(),
        },
    ];

    for event in events {
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: SecurityKeyEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, decoded);
    }
}

#[test]
fn security_key_broker_open_device_request_round_trips() {
    let req = SecurityKeyOpenDeviceRequest {
        device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
        session_id: SecurityKeySessionId::new("sk-corp-vm-1"),
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: SecurityKeyOpenDeviceRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);
}

#[test]
fn security_key_broker_apply_udev_rules_request_round_trips() {
    let req = SecurityKeyApplyUdevRulesRequest {
        bundle_udev_intent_ref: "sk-udev-intent-host".to_owned(),
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: SecurityKeyApplyUdevRulesRequest =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);
}

// ---------------------------------------------------------------------------
// Public wire: new variants round-trip via tag+content serde
// ---------------------------------------------------------------------------

#[test]
fn public_request_usb_security_key_status_round_trips() {
    let req = PublicRequest::UsbSecurityKeyStatus;
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: PublicRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);
}

#[test]
fn public_request_usb_security_key_sessions_round_trips() {
    let req = PublicRequest::UsbSecurityKeySessions;
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: PublicRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);
}

#[test]
fn public_request_usb_security_key_cancel_round_trips() {
    let req = PublicRequest::UsbSecurityKeyCancel(SecurityKeyCancelRequest {
        session_id: Some(SecurityKeySessionId::new("sk-1")),
        cancel_current: false,
    });
    let json = serde_json::to_string(&req).expect("serialize");
    let decoded: PublicRequest = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(req, decoded);
}

#[test]
fn public_response_usb_security_key_status_round_trips() {
    let resp = PublicResponse::UsbSecurityKeyStatus(SecurityKeyStatusResponse {
        host_proxy_enabled: false,
        devices: vec![],
        current_lease: None,
        vm_states: vec![],
    });
    let json = serde_json::to_string(&resp).expect("serialize");
    // PublicResponse is Serialize only (no Deserialize), so assert tag is present
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(
        value.get("kind").and_then(|v| v.as_str()),
        Some("usb security-key status"),
        "unexpected kind tag in: {json}"
    );
}

// ---------------------------------------------------------------------------
// Broker wire: new variants appear in BrokerRequest and capabilities
// ---------------------------------------------------------------------------

#[test]
fn broker_request_security_key_open_device_op_name() {
    let req = BrokerRequest::SecurityKeyOpenDevice(SecurityKeyOpenDeviceRequest {
        device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
        session_id: SecurityKeySessionId::new("sk-1"),
    });
    assert_eq!(req.op_name(), "SecurityKeyOpenDevice");
}

#[test]
fn broker_request_security_key_apply_udev_rules_op_name() {
    let req = BrokerRequest::SecurityKeyApplyUdevRules(SecurityKeyApplyUdevRulesRequest {
        bundle_udev_intent_ref: "sk-udev-ref".to_owned(),
    });
    assert_eq!(req.op_name(), "SecurityKeyApplyUdevRules");
}

#[test]
fn w3_capabilities_include_security_key_operations() {
    let caps = BrokerCapabilities::w3();
    assert!(
        caps.broker_operations
            .iter()
            .any(|op| op == "SecurityKeyOpenDevice"),
        "capabilities missing SecurityKeyOpenDevice; got: {:?}",
        caps.broker_operations
    );
    assert!(
        caps.broker_operations
            .iter()
            .any(|op| op == "SecurityKeyApplyUdevRules"),
        "capabilities missing SecurityKeyApplyUdevRules; got: {:?}",
        caps.broker_operations
    );
}

#[test]
fn w3_operation_security_key_open_device_has_audit() {
    let flags = W3BrokerOperation::SecurityKeyOpenDevice.flags();
    assert!(flags.audit, "SecurityKeyOpenDevice must be audited");
    assert!(
        !flags.destructive,
        "SecurityKeyOpenDevice must not be destructive"
    );
    assert!(
        !flags.secret_access,
        "SecurityKeyOpenDevice must not have secret access"
    );
}

#[test]
fn w3_operation_security_key_apply_udev_rules_is_destructive() {
    let flags = W3BrokerOperation::SecurityKeyApplyUdevRules.flags();
    assert!(flags.audit, "SecurityKeyApplyUdevRules must be audited");
    assert!(
        flags.destructive,
        "SecurityKeyApplyUdevRules must be destructive (writes udev rules)"
    );
}

// ---------------------------------------------------------------------------
// Type 5: policy lints — new operations appear in docs and privileges matrix
// ---------------------------------------------------------------------------

const DISPOSITIONS_DOC: &str = "docs/reference/broker-w2-dispositions.md";
const PRIVILEGES_JSON: &str = "docs/reference/schemas/v2/privileges.json";

#[test]
fn security_key_operations_in_dispositions_doc() {
    assert!(
        repo_path_exists(DISPOSITIONS_DOC),
        "broker-w2-dispositions.md must exist"
    );
    let doc = read_repo_file(DISPOSITIONS_DOC);

    for op in ["SecurityKeyOpenDevice", "SecurityKeyApplyUdevRules"] {
        assert!(
            doc.contains(op),
            "{DISPOSITIONS_DOC} missing row for {op}"
        );
    }
}

#[test]
fn security_key_operations_are_stubs_in_dispositions_doc() {
    let doc = read_repo_file(DISPOSITIONS_DOC);
    // Phase 1: these must be stubs, not promoted-live.
    for op in ["SecurityKeyOpenDevice", "SecurityKeyApplyUdevRules"] {
        let line = doc
            .lines()
            .find(|l| l.contains(op))
            .unwrap_or_else(|| panic!("{DISPOSITIONS_DOC} missing row for {op}"));
        assert!(
            line.contains("stubbed-unimplemented") || line.contains("future work"),
            "{op} should be a stub in phase 1, got: {line}"
        );
    }
}

#[test]
fn public_operation_usb_security_key_in_privileges_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(&read_repo_file(PRIVILEGES_JSON)).expect("privileges.json parses");
    let ops = schema
        .pointer("/definitions/OperationAuthz/properties/operation/enum")
        .and_then(|v| v.as_array())
        .expect("OperationAuthz.operation.enum present");
    let found: Vec<&str> = ops.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        found.contains(&"usb security-key"),
        "privileges schema missing 'usb security-key' public op; found: {found:?}"
    );
}
