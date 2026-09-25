use d2b_provider_clipboard_wayland::{
    ClipboardAuditEvent, ClipboardConfig, ClipboardEntry, ClipboardHistory, ClipboardReason,
    SizeBucket,
};

#[test]
fn payload_canary_stays_out_of_clipboard_debug_and_audit() {
    const CANARY: &str = "clipboard-payload-canary-7f4a";
    let entry =
        ClipboardEntry::new("Guest/work", "text/plain", CANARY.as_bytes(), 100).expect("entry");
    let mut history = ClipboardHistory::new(ClipboardConfig::default());
    history.insert(entry).expect("insert");

    let debug = format!("{history:?}");
    assert!(!debug.contains(CANARY));

    let event = ClipboardAuditEvent::new(
        "zone-a",
        "zone-b",
        ClipboardReason::Allowed,
        SizeBucket::from_len(CANARY.len()),
    );
    assert!(!event.to_wire().contains(CANARY));
    assert!(!format!("{event:?}").contains(CANARY));
}
