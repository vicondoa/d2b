use d2b_provider_clipboard_wayland::{ClipboardConfig, ClipboardEntry, ClipboardHistory};

#[test]
fn guest_destroy_purges_history() {
    let mut history = ClipboardHistory::new(ClipboardConfig::default()).unwrap();
    history
        .insert(ClipboardEntry::new("Guest/work", "text/plain", b"x", 1).unwrap())
        .unwrap();
    history.purge_guest("Guest/work");
    assert!(history.is_empty());
}
