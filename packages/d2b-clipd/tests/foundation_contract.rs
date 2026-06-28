use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use d2b_clipd::audit::{AuditDecision, AuditEvent, AuditQueue, AuditQueueConfig};
use d2b_clipd::framing::{PICKER_TO_DAEMON_MAX_FRAME_BYTES, decode_frame, encode_frame};
use d2b_clipd::picker::sanitize_picker_env;
use d2b_clipd::policy::{AttributionQuality, ReasonCode, has_secret_hint, is_mime_allowed};
use d2b_clipd::protocol::{
    ClientHello, PickerToDaemonMessage, ProtocolVersionRange, negotiate_version,
};

#[test]
fn picker_handshake_is_small_tokenless_and_versioned() {
    let hello = PickerToDaemonMessage::ClientHello(ClientHello {
        protocol_version_range: ProtocolVersionRange { min: 1, max: 1 },
        picker_version: "d2b-clip-picker-test".to_owned(),
    });

    let frame = encode_frame(&hello, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect("encode");
    assert!(frame.len() < 256);
    assert!(!String::from_utf8_lossy(&frame).contains("token"));
    let decoded: PickerToDaemonMessage =
        decode_frame(&frame, PICKER_TO_DAEMON_MAX_FRAME_BYTES).expect("decode");
    let PickerToDaemonMessage::ClientHello(decoded_hello) = decoded else {
        panic!("decoded wrong message type");
    };
    assert_eq!(
        negotiate_version(&decoded_hello.protocol_version_range, 1),
        Some(1)
    );
}

#[test]
fn picker_environment_never_forwards_niri_socket() {
    let ambient = BTreeMap::from([
        (
            OsString::from("WAYLAND_DISPLAY"),
            OsString::from("wayland-1"),
        ),
        (
            OsString::from("NIRI_SOCKET"),
            OsString::from("/run/user/1000/niri.sock"),
        ),
    ]);

    let sanitized = sanitize_picker_env(&ambient);
    assert!(sanitized.contains_key(OsStr::new("WAYLAND_DISPLAY")));
    assert!(!sanitized.contains_key(OsStr::new("NIRI_SOCKET")));
}

#[test]
fn policy_and_audit_fail_closed_foundation_match_clipboard_contract() {
    assert!(is_mime_allowed("text/plain;charset=utf-8"));
    assert!(!is_mime_allowed("application/octet-stream"));
    assert!(has_secret_hint(["x-kde-passwordManagerHint"]));

    let mut audit = AuditQueue::new(AuditQueueConfig { per_realm_quota: 0 });
    let failure = audit.enqueue_fail_closed(AuditEvent {
        request_id: "r".to_owned(),
        source_realm: "vm-a".to_owned(),
        destination_realm: "host".to_owned(),
        mime_type: "text/plain".to_owned(),
        byte_count: 5,
        decision: AuditDecision::Deny,
        attribution: AttributionQuality::ExactClient,
        reason: ReasonCode::PolicyDenied,
        timestamp_unix_ms: 1,
    });
    assert_eq!(failure, Err(ReasonCode::AuditFailure));
}
