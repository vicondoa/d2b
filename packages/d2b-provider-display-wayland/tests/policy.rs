use d2b_provider_display_wayland::{
    DisplayIdentity, FilterInput, PolicyWarning, WaylandPolicy, WaylandSessionSpec,
};

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

/// The compiled protocol filter is a function of the session's filter alone:
/// deriving a session's endpoint authority does not add, remove, or relax a
/// single interface decision.
#[test]
fn the_compiled_filter_is_independent_of_the_endpoint_derivation() {
    let session = |display: &str| {
        WaylandSessionSpec::new(
            d2b_contracts_resource::v3::ResourceRef::parse("Guest/work-vm").unwrap(),
            d2b_contracts_resource::v3::ResourceRef::parse("Host/host-system").unwrap(),
            d2b_contracts_resource::v3::ResourceRef::parse("User/alice").unwrap(),
            d2b_contracts_resource::v3::ResourceRef::parse(
                "display-wayland.d2bus.org.WaylandPolicy/default",
            )
            .unwrap(),
            DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
            true,
        )
        .unwrap()
        .with_compositor_display(Some(
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(display).unwrap(),
        ))
        .unwrap()
        .with_filter(
            FilterInput::new(
                ["wl_compositor"],
                ["zwp_linux_dmabuf_v1"],
                Vec::<(String, u32)>::new(),
                Vec::<String>::new(),
            )
            .unwrap(),
        )
    };

    let first = WaylandPolicy::compile(
        &FilterInput::default(),
        &FilterInput::default(),
        session("wayland-0").filter(),
    )
    .unwrap();
    let second = WaylandPolicy::compile(
        &FilterInput::default(),
        &FilterInput::default(),
        session("wayland-7").filter(),
    )
    .unwrap();

    assert_eq!(first.digest(), second.digest());
    assert!(!first.is_allowed("zwp_linux_dmabuf_v1"));
    assert!(first.is_allowed("wl_compositor"));
}
