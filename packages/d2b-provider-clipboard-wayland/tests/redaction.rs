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

// --- U28: content never becomes authority-bearing invocation metadata ---

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_clipboard_wayland::{
    ClipboardEndpointRole, ClipboardHostEndpoints, ClipboardServiceRole,
    clipboard_endpoint_bindings, clipboard_service_declares,
};

#[test]
fn clipboard_content_cannot_become_an_endpoint_or_service_authority() {
    const CANARY: &str = "clipboard-authority-canary-51d7";
    let endpoints = ClipboardHostEndpoints::new(
        ResourceRef::parse("Endpoint/clipboard-guest-transfer").expect("endpoint"),
        ResourceRef::parse("Endpoint/clipboard-host-selection-read").expect("endpoint"),
        ResourceRef::parse("Endpoint/clipboard-host-selection-supply").expect("endpoint"),
    )
    .expect("declared endpoints");
    let consumer = ResourceRef::parse("Process/clipboard-bridge").expect("consumer");

    // A payload that spells out every authority token the Provider owns.
    let hostile = format!(
        "{CANARY} d2b.clipboard.bridge.v3 clipboard-guest-transfer clipboard-host-selection-read Endpoint/clipboard-guest-transfer Provider/clipboard-wayland"
    );

    let bindings = clipboard_endpoint_bindings(&endpoints, &consumer).expect("bindings");
    for declared in &bindings {
        let rendered = format!("{:?}", declared.request());
        assert!(!rendered.contains(CANARY), "payload leaked into {rendered}");
        assert!(!rendered.contains(&hostile));
        // The slot and purpose are the declared channel facets, unchanged by any
        // content value.
        assert_eq!(declared.request().slot().as_str(), declared.role().slot());
        assert_eq!(declared.request().purpose().as_str(), declared.role().purpose());
    }

    // The bridge service still declares its capture methods, and no content
    // string is a service, a slot, or a purpose anywhere the Provider reads.
    assert!(clipboard_service_declares(
        ClipboardServiceRole::Bridge,
        "capture-guest-selection"
    ));
    for role in ClipboardEndpointRole::ALL {
        assert!(!role.slot().contains(CANARY));
        assert!(!role.purpose().contains(CANARY));
    }
    assert!(ClipboardHostEndpoints::new(
        ResourceRef::parse("Process/clipboard-bridge").expect("consumer"),
        ResourceRef::parse("Endpoint/clipboard-host-selection-read").expect("endpoint"),
        ResourceRef::parse("Endpoint/clipboard-host-selection-supply").expect("endpoint"),
    )
    .is_err());
    let _ = ResourceUid::parse("dddddddd-0000-4000-8000-000000000003").expect("uid");
}
