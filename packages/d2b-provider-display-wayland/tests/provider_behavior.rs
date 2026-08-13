use d2b_contracts::v3::ResourceRef;
use d2b_provider_display_wayland::{
    AttachmentGrantHandle, DisplayAuditKind, DisplayAuditOutcome, DisplayIdentity,
    DisplayLabelPosition, DisplayProviderDescriptor, DisplayTelemetryField, DisplayTelemetryFrame,
    DisplayUserPortal, FilterInput, FinalizationInput, Phase, PolicyWarning, PrincipalPool,
    ProxyReadinessFailure, ProxyReadinessStage, ProxyReadinessState, WaylandPolicy,
    WaylandSessionSpec,
};

fn refs() -> (ResourceRef, ResourceRef, ResourceRef, ResourceRef) {
    (
        ResourceRef::parse("Guest/work-vm").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default").unwrap(),
    )
}

fn identity() -> DisplayIdentity {
    DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8")
        .unwrap()
        .with_label_position(DisplayLabelPosition::TopLeft)
}

#[test]
fn session_rejects_untrusted_cross_domain_and_invalid_identity() {
    let (guest, host, user, policy) = refs();
    assert!(
        WaylandSessionSpec::new(
            guest.clone(),
            host.clone(),
            user.clone(),
            policy.clone(),
            identity(),
            false,
        )
        .is_err()
    );
    assert!(DisplayIdentity::new("Work VM", "#7fc8ff", "#45475a", "#f38ba8").is_err());
    assert!(DisplayIdentity::new("work-vm", "red", "#45475a", "#f38ba8").is_err());
}

#[test]
fn policy_layering_is_closed_and_clipboard_globals_are_virtualized() {
    let defaults = FilterInput::default();
    let zone = FilterInput::new(
        ["zwp_linux_dmabuf_v1"],
        ["zwp_pointer_constraints_v1", "zwp_linux_dmabuf_v1"],
        Vec::<(String, u32)>::new(),
        Vec::<String>::new(),
    )
    .unwrap();
    let session = FilterInput::new(
        ["zwp_pointer_constraints_v1", "wl_data_device_manager"],
        Vec::<String>::new(),
        Vec::<(String, u32)>::new(),
        Vec::<String>::new(),
    )
    .unwrap();
    let compiled = WaylandPolicy::compile(&defaults, &zone, &session).unwrap();
    assert!(compiled.is_allowed("wl_compositor"));
    assert!(!compiled.is_allowed("zwp_linux_dmabuf_v1"));
    assert!(
        compiled
            .warnings()
            .contains(&PolicyWarning::ClipboardBoundaryIgnored)
    );
    assert!(
        WaylandPolicy::compile(
            &defaults,
            &FilterInput::new(
                ["unknown_global"],
                Vec::<String>::new(),
                Vec::<(String, u32)>::new(),
                Vec::<String>::new(),
            )
            .unwrap(),
            &FilterInput::default(),
        )
        .is_err()
    );
}

#[test]
fn principal_pool_is_opaque_and_fails_closed_when_exhausted() {
    let mut pool = PrincipalPool::new(["corp-vm"], 1).unwrap();
    assert_eq!(
        PrincipalPool::principal_for("dev", "corp-vm"),
        "d2b-wlp-e57e8feb6155"
    );
    let lease = pool.acquire_dynamic().unwrap();
    assert!(pool.acquire_dynamic().is_err());
    assert!(format!("{lease:?}").contains("REDACTED"));
    pool.release(lease).unwrap();
    assert!(pool.acquire_dynamic().is_ok());
}

#[test]
fn readiness_event_is_bounded_and_path_free() {
    let event = d2b_provider_display_wayland::ProxyReadinessEvent::failed(
        ProxyReadinessStage::Upstream,
        ProxyReadinessFailure::UpstreamUnavailable,
    );
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(event.state, ProxyReadinessState::Failed);
    assert!(!json.contains("socket"));
    assert!(!json.contains("path"));
    assert!(json.contains("upstream-unavailable"));
}

#[test]
fn display_descriptor_is_status_first_and_publishes_typed_services() {
    let descriptor = DisplayProviderDescriptor::default();
    assert!(descriptor.validate().is_ok());
    assert!(!descriptor.provider_state_volume);
    assert!(
        descriptor
            .service_packages()
            .contains(&"d2b.display.host-clipboard.v3")
    );
}

#[test]
fn controller_status_transitions_pending_ready_and_failed() {
    let (guest, host, user, policy) = refs();
    let spec = WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(4);
    let pending = controller
        .reconcile(
            &spec,
            d2b_provider_display_wayland::DependencyState::default(),
            d2b_provider_display_wayland::ProcessObservation::default(),
        )
        .unwrap();
    assert_eq!(pending.status.phase, Phase::Pending);
    let ready = controller
        .reconcile(
            &spec,
            d2b_provider_display_wayland::DependencyState::ready(),
            d2b_provider_display_wayland::ProcessObservation::ready(),
        )
        .unwrap();
    assert_eq!(ready.status.phase, Phase::Ready);
    let failed = controller
        .reconcile(
            &spec,
            d2b_provider_display_wayland::DependencyState::ready(),
            d2b_provider_display_wayland::ProcessObservation::proxy_failed(5),
        )
        .unwrap();
    assert_eq!(failed.status.phase, Phase::Failed);
}

#[test]
fn portal_is_same_uid_and_finalizer_is_fail_closed() {
    let user = ResourceRef::parse("User/alice").unwrap();
    let mut portal = DisplayUserPortal::new(user.clone(), 1000, 1).unwrap();
    assert!(
        portal
            .issue_grant(
                "session-digest",
                &user,
                1001,
                AttachmentGrantHandle::from_core([3; 32]),
            )
            .is_err()
    );
    assert!(
        portal
            .issue_grant(
                "session-digest",
                &user,
                1000,
                AttachmentGrantHandle::from_core([3; 32]),
            )
            .is_ok()
    );
    let ambiguous = d2b_provider_display_wayland::DisplayController::finalize(FinalizationInput {
        stop_requested: true,
        proxy_terminal: false,
        proxy_deleted: false,
        volume_deleted: false,
        grace_expired: true,
    });
    assert!(ambiguous.ambiguous);
    assert!(!ambiguous.remove_finalizer);
}

#[test]
fn audit_and_telemetry_reject_identity_bearing_surfaces() {
    let marker = "window-title-canary";
    let record = d2b_provider_display_wayland::DisplayAuditRecord::new(
        DisplayAuditKind::ProxyStarted,
        DisplayAuditOutcome::Success,
        "dev",
        marker,
        "alice",
        "operation-1",
    );
    assert!(!record.to_wire_record().contains(marker));
    let frame =
        DisplayTelemetryFrame::new("dev", d2b_provider_display_wayland::MetricOutcome::Success);
    assert!(
        DisplayTelemetryFrame::validate_collector_fields(frame.metric_labels().to_vec()).is_ok()
    );
    assert!(
        DisplayTelemetryFrame::validate_collector_fields([DisplayTelemetryField {
            key: "window_title",
            value: marker.to_owned(),
        }])
        .is_err()
    );
}
