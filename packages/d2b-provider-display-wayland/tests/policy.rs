use d2b_provider_display_wayland::{FilterInput, PolicyWarning, WaylandPolicy};

#[test]
fn clipboard_boundary_allow_entries_are_advisory_only() {
    let compiled = WaylandPolicy::compile(
        &FilterInput::default(),
        &FilterInput::default(),
        &FilterInput::new(
            ["wl_data_device_manager"],
            Vec::<String>::new(),
            Vec::<(String, u32)>::new(),
            Vec::<String>::new(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!compiled.is_allowed("wl_data_device_manager"));
    assert!(
        compiled
            .warnings()
            .contains(&PolicyWarning::ClipboardBoundaryIgnored)
    );
}
