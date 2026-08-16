use d2b_provider_clipboard_wayland::{
    ClipboardAuditEvent, ClipboardAuditQueue, ClipboardReason, SizeBucket,
};

#[test]
fn audit_never_contains_payload_bytes() {
    let mut queue = ClipboardAuditQueue::new(1);
    queue
        .push(ClipboardAuditEvent::new(
            "zone-a",
            "zone-b",
            ClipboardReason::Allowed,
            SizeBucket::Lt1K,
        ))
        .unwrap();
    assert!(!queue.to_wire().contains("payload"));
}
