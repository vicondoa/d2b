use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_provider_display_wayland::{
    DisplayController, DisplayEndpointVocabulary, DisplayIdentity, DisplayLabelPosition,
    DisplayProcessRole, EndpointSpec, FilterInput, Phase, PolicyWarning, PrincipalPool,
    ProcessObservation, WaylandPolicy, WaylandPolicySnapshot, WaylandSessionSpec,
    display_committed_endpoint_shape,
};
use d2b_provider_endpoint::{CommittedEndpointShape, EndpointRealization};

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
fn wayland_session_accepts_the_canonical_debug_logging_filter_field() {
    let value = serde_json::json!({
        "guestRef": "Guest/work-vm",
        "hostRef": "Host/host-system",
        "userRef": "User/alice",
        "policyRef": "display-wayland.d2bus.org.WaylandPolicy/default",
        "identity": {
            "label": "work-vm",
            "activeColor": "#7fc8ff",
            "inactiveColor": "#45475a",
            "urgentColor": "#f38ba8",
            "borderEnabled": true,
            "borderWidth": 2,
            "labelEnabled": true,
            "labelText": "work-vm",
            "labelPosition": "top-left"
        },
        "crossDomainTrusted": true,
        "reconnectGeneration": 1,
        "virglVideo": false,
        "filter": {
            "debugLogging": false,
            "allowGlobals": [],
            "denyGlobals": [],
            "maxVersions": {},
            "dmabufAllow": [],
            "dmabufDeny": []
        }
    });

    let spec = serde_json::from_value::<WaylandSessionSpec>(value).unwrap();
    let encoded = serde_json::to_value(spec).unwrap();
    assert_eq!(encoded["filter"]["debugLogging"], false);
}

fn policy_for(spec: &WaylandSessionSpec) -> WaylandPolicySnapshot {
    WaylandPolicySnapshot::from_test_core(
        spec.policy_ref().clone(),
        d2b_contracts_resource::v3::ZoneId::parse("local").unwrap(),
        1,
        FilterInput::default(),
        FilterInput::default(),
    )
    .unwrap()
}

fn reconcile(
    controller: &mut DisplayController,
    spec: &WaylandSessionSpec,
    dependencies: d2b_provider_display_wayland::DependencyState,
    observation: ProcessObservation,
) -> Result<
    d2b_provider_display_wayland::ReconcileResult,
    d2b_provider_display_wayland::WaylandSpecError,
> {
    let policy = policy_for(spec);
    controller.reconcile_with_policy(spec, dependencies, observation, None, &policy)
}

fn reconcile_with_evidence(
    controller: &mut DisplayController,
    spec: &WaylandSessionSpec,
    dependencies: d2b_provider_display_wayland::DependencyState,
    observation: ProcessObservation,
    evidence: d2b_provider_display_wayland::WorkerRestartEvidence,
) -> Result<
    d2b_provider_display_wayland::ReconcileResult,
    d2b_provider_display_wayland::WaylandSpecError,
> {
    let policy = policy_for(spec);
    controller.reconcile_with_policy_and_evidence(
        spec,
        dependencies,
        observation,
        evidence,
        None,
        &policy,
    )
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
fn dmabuf_rules_are_compiled_and_digest_bound() {
    let defaults = FilterInput::default();
    let zone = FilterInput::new(
        Vec::<String>::new(),
        Vec::<String>::new(),
        Vec::<(String, u32)>::new(),
        ["format-x"],
    )
    .unwrap()
    .with_dmabuf_deny(["format-y"])
    .unwrap();
    let compiled = WaylandPolicy::compile(&defaults, &zone, &defaults).unwrap();
    assert!(compiled.dmabuf_allowed().contains(&"format-x".to_owned()));
    assert!(compiled.dmabuf_denied().contains(&"format-y".to_owned()));
    assert!(compiled.is_dmabuf_allowed("format-x"));
    assert!(!compiled.is_dmabuf_allowed("format-y"));
}

#[test]
fn principal_pool_is_opaque_and_fails_closed_when_exhausted() {
    let mut pool = PrincipalPool::new(1).unwrap();
    let lease = pool.acquire_dynamic().unwrap();
    assert!(pool.acquire_dynamic().is_err());
    assert!(format!("{lease:?}").contains("REDACTED"));
    pool.release(lease).unwrap();
    assert!(pool.acquire_dynamic().is_ok());
}

#[test]
fn controller_status_transitions_pending_ready_and_failed() {
    let (guest, host, user, policy) = refs();
    let spec = WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(4).unwrap();
    let pending = reconcile(
        &mut controller,
        &spec,
        d2b_provider_display_wayland::DependencyState::default(),
        d2b_provider_display_wayland::ProcessObservation::default(),
    )
    .unwrap();
    assert_eq!(pending.status.phase, Phase::Pending);
    let ready = reconcile(
        &mut controller,
        &spec,
        d2b_provider_display_wayland::DependencyState::ready(),
        d2b_provider_display_wayland::ProcessObservation::ready_for_session(&spec, 1, 1),
    )
    .unwrap();
    assert_eq!(ready.status.phase, Phase::Ready);
    let failed = reconcile_with_evidence(
        &mut controller,
        &spec,
        d2b_provider_display_wayland::DependencyState::ready(),
        d2b_provider_display_wayland::ProcessObservation::proxy_failed(5),
        d2b_provider_display_wayland::WorkerRestartEvidence::for_test(1_000, Some(0), None, 1),
    )
    .unwrap();
    assert_eq!(failed.status.phase, Phase::Failed);
}

#[test]
fn failed_reconcile_retains_the_session_principal_until_cleanup() {
    let (guest, host, user, policy) = refs();
    let first = WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap();
    let second = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/second").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default").unwrap(),
        DisplayIdentity::new("second", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(1).unwrap();
    let first_status = reconcile(
        &mut controller,
        &first,
        d2b_provider_display_wayland::DependencyState::ready(),
        ProcessObservation::ready_for_session(&first, 1, 1),
    )
    .unwrap()
    .status;
    assert!(first_status.principal.is_some());
    assert_eq!(
        reconcile_with_evidence(
            &mut controller,
            &first,
            d2b_provider_display_wayland::DependencyState::ready(),
            ProcessObservation::proxy_failed(5),
            d2b_provider_display_wayland::WorkerRestartEvidence::for_test(1_000, Some(0), None, 1,),
        )
        .unwrap()
        .status
        .phase,
        Phase::Failed
    );
    assert_eq!(
        reconcile(
            &mut controller,
            &second,
            d2b_provider_display_wayland::DependencyState::ready(),
            ProcessObservation::ready_for_session(&second, 1, 1),
        )
        .unwrap()
        .status
        .phase,
        Phase::Failed
    );
}

#[test]
fn mutable_session_fields_reuse_the_same_principal() {
    let (guest, host, user, policy) = refs();
    let first = WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap();
    let changed = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/work-vm").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/changed").unwrap(),
        DisplayIdentity::new("work-vm", "#a6e3a1", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(1).unwrap();
    let first_principal = reconcile(
        &mut controller,
        &first,
        d2b_provider_display_wayland::DependencyState::ready(),
        ProcessObservation::ready_for_session(&first, 1, 1),
    )
    .unwrap()
    .status
    .principal;
    let changed_principal = reconcile(
        &mut controller,
        &changed,
        d2b_provider_display_wayland::DependencyState::ready(),
        ProcessObservation::ready_for_session(&changed, 1, 1),
    )
    .unwrap()
    .status
    .principal;
    assert_eq!(first_principal, changed_principal);
}

#[test]
fn readiness_cannot_be_reused_for_a_different_host_or_user_binding() {
    let (guest, host, user, policy) = refs();
    let spec = WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap();
    let retargeted = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/work-vm").unwrap(),
        ResourceRef::parse("Host/other-host").unwrap(),
        ResourceRef::parse("User/bob").unwrap(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default").unwrap(),
        identity(),
        true,
    )
    .unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(2).unwrap();
    assert_eq!(
        reconcile(
            &mut controller,
            &retargeted,
            d2b_provider_display_wayland::DependencyState::ready(),
            ProcessObservation::ready_for_session(&spec, 1, 1),
        )
        .unwrap()
        .status
        .phase,
        Phase::Pending
    );
}

#[test]
fn wire_deserialization_reuses_display_validation() {
    let value = serde_json::to_value(identity()).unwrap();
    let mut invalid_identity = value;
    invalid_identity["label"] = serde_json::json!("Work VM");
    assert!(serde_json::from_value::<DisplayIdentity>(invalid_identity).is_err());
}

#[test]
fn distinct_authenticated_sessions_do_not_share_display_principals() {
    let (_, host, user, policy) = refs();
    let first = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/first").unwrap(),
        host.clone(),
        user.clone(),
        policy.clone(),
        identity(),
        true,
    )
    .unwrap();
    let second = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/second").unwrap(),
        host,
        user,
        policy,
        identity(),
        true,
    )
    .unwrap();
    let mut controller = d2b_provider_display_wayland::DisplayController::new(2).unwrap();
    let first_status = reconcile(
        &mut controller,
        &first,
        d2b_provider_display_wayland::DependencyState::ready(),
        ProcessObservation::ready_for_session(&first, 1, 1),
    )
    .unwrap()
    .status;
    let second_status = reconcile(
        &mut controller,
        &second,
        d2b_provider_display_wayland::DependencyState::ready(),
        ProcessObservation::ready_for_session(&second, 1, 1),
    )
    .unwrap()
    .status;
    assert_ne!(first_status.principal, second_status.principal);
}

#[test]
fn finalizer_is_fail_closed() {
    assert_eq!(
        d2b_provider_display_wayland::DisplayController::finalizer(),
        "display-wayland.d2bus.org/proxy-stopped"
    );
}

#[test]
fn display_runner_contract_disables_legacy_scheduling() {
    let contract = d2b_provider_display_wayland::display_runner_contract();
    assert_eq!(
        contract.session_resource_type(),
        "display-wayland.d2bus.org.WaylandSession"
    );
    assert_eq!(
        contract.policy_resource_type(),
        "display-wayland.d2bus.org.WaylandPolicy"
    );
    assert_eq!(
        contract.finalizer(),
        "display-wayland.d2bus.org/proxy-stopped"
    );
    assert_eq!(contract.repair_interval_secs(), 30);
    assert_eq!(contract.max_repair_interval_secs(), 60);
    assert!(contract.watched_configuration_is_dependency());
}

// -- endpoint authority (U26) ---------------------------------------------

/// A committed session uid: the durable identity the session's child rows and
/// endpoint relationships are derived from.
fn session_uid(seed: &str) -> d2b_contracts_resource::v3::ResourceUid {
    d2b_contracts_resource::v3::ResourceUid::parse(seed).unwrap()
}

fn zone() -> d2b_contracts_resource::v3::ZoneId {
    d2b_contracts_resource::v3::ZoneId::parse("local").unwrap()
}

fn session_spec() -> WaylandSessionSpec {
    let (guest, host, user, policy) = refs();
    WaylandSessionSpec::new(guest, host, user, policy, identity(), true).unwrap()
}

/// The endpoint rows one session derives, keyed by their resource reference,
/// decoded back into the endpoint contract the relationships are evaluated
/// against.
fn derived_endpoints(
    spec: &WaylandSessionSpec,
    uid: &d2b_contracts_resource::v3::ResourceUid,
    generation: u64,
) -> Vec<(ResourceRef, d2b_provider_display_wayland::EndpointSpec)> {
    d2b_provider_display_wayland::session_children::display_owned_child_intents(
        &zone(),
        &ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/demo").unwrap(),
        uid,
        spec,
        generation,
    )
    .unwrap()
    .into_iter()
    .filter(|intent| intent.target().resource_type().as_str() == "Endpoint")
    .map(|intent| {
        let value: serde_json::Value = serde_json::from_slice(intent.canonical_resource()).unwrap();
        (
            intent.target().clone(),
            d2b_provider_display_wayland::decode_endpoint_spec(&value["spec"]).unwrap(),
        )
    })
    .collect()
}

/// One admitted relationship proved against the session's own derived rows.
fn admit(
    spec: &WaylandSessionSpec,
    uid: &d2b_contracts_resource::v3::ResourceUid,
    binding: &d2b_provider_display_wayland::DisplayEndpointBinding,
    generation: u64,
) -> Result<(), d2b_provider_display_wayland::WorkerEffectError> {
    let endpoints = derived_endpoints(spec, uid, generation);
    let source = endpoints
        .iter()
        .find(|(reference, _)| reference == binding.source_ref())
        .map(|(_, endpoint_spec)| endpoint_spec)
        .cloned()
        .expect("the derived source row");
    let observed = d2b_provider_display_wayland::DisplayEndpointObservation {
        spec: &source,
        source_generation: generation,
        consumer_generation: generation,
        consumer_user: Some(spec.user_ref().clone()),
    };
    d2b_provider_display_wayland::admit_display_endpoint(
        &zone(),
        spec,
        uid,
        binding,
        &observed,
        uid,
        uid,
    )
    .map(|_| ())
}

#[test]
fn every_worker_reaches_only_the_endpoint_its_relationship_names() {
    let spec = session_spec();
    let uid = session_uid("11111111-1111-4111-8111-111111111111");
    let bindings =
        d2b_provider_display_wayland::display_endpoint_bindings(&uid, &spec).unwrap();

    let compositor = bindings
        .iter()
        .find(|binding| binding.role() == DisplayProcessRole::HostProxy)
        .expect("the proxy consumes the host compositor");
    assert_eq!(
        compositor.source_ref().resource_type().as_str(),
        "Endpoint"
    );
    assert_eq!(
        compositor.source_ref(),
        &d2b_provider_display_wayland::durable_compositor_endpoint_ref(&uid).unwrap()
    );
    assert_eq!(
        compositor.request().slot().as_str(),
        d2b_provider_display_wayland::COMPOSITOR_BINDING_SLOT
    );
    assert_eq!(
        compositor.request().attachment(),
        d2b_contracts_resource::v3::EndpointAttachmentKind::Connect
    );
    assert_eq!(
        compositor.request().purpose().as_str(),
        d2b_provider_display_wayland::COMPOSITOR_BINDING_PURPOSE
    );

    let frontend = bindings
        .iter()
        .find(|binding| binding.role() == DisplayProcessRole::GuestFrontend)
        .expect("the frontend consumes the proxy's own endpoint");
    assert_eq!(
        frontend.request().slot().as_str(),
        d2b_provider_display_wayland::PROXY_BINDING_SLOT
    );
    assert_eq!(
        frontend.request().attachment(),
        d2b_contracts_resource::v3::EndpointAttachmentKind::Attach
    );
    assert_ne!(frontend.source_ref(), compositor.source_ref());

    // Every derived relationship is admitted against the session's own rows.
    for binding in &bindings {
        admit(&spec, &uid, binding, 3).expect("the derived relationship is admitted");
    }

    // The compositor row admits exactly the proxy row and no other subject.
    let (_, compositor_endpoint) = derived_endpoints(&spec, &uid, 3)
        .into_iter()
        .find(|(reference, _)| reference == compositor.source_ref())
        .unwrap();
    assert_eq!(
        compositor_endpoint
            .consumer_policy()
            .allowed_subjects(),
        &[compositor.consumer_ref().clone()]
    );
}

/// One observation of a committed endpoint row at one generation.
fn observed(
    generation: u64,
    user: Option<ResourceRef>,
    endpoint_spec: &d2b_provider_display_wayland::EndpointSpec,
) -> d2b_provider_display_wayland::DisplayEndpointObservation<'_> {
    d2b_provider_display_wayland::DisplayEndpointObservation {
        spec: endpoint_spec,
        source_generation: generation,
        consumer_generation: generation,
        consumer_user: user,
    }
}

#[test]
fn a_wrong_compositor_endpoint_user_or_generation_is_refused() {
    let spec = session_spec();
    let uid = session_uid("22222222-2222-4222-8222-222222222222");
    let bindings =
        d2b_provider_display_wayland::display_endpoint_bindings(&uid, &spec).unwrap();
    let compositor = bindings
        .iter()
        .find(|binding| binding.role() == DisplayProcessRole::HostProxy)
        .unwrap();
    let (_, endpoint) = derived_endpoints(&spec, &uid, 3)
        .into_iter()
        .find(|(reference, _)| reference == compositor.source_ref())
        .unwrap();

    let admit_with =
        |observation: d2b_provider_display_wayland::DisplayEndpointObservation<'_>| {
            d2b_provider_display_wayland::admit_display_endpoint(
                &zone(),
                &spec,
                &uid,
                compositor,
                &observation,
                &uid,
                &uid,
            )
        };

    assert!(
        admit_with(observed(3, Some(spec.user_ref().clone()), &endpoint)).is_ok(),
        "the session's own compositor row is admitted"
    );

    // A consumer row admitted for another User cannot reach this session's
    // compositor endpoint.
    assert!(
        admit_with(observed(
            3,
            Some(ResourceRef::parse("User/mallory").unwrap()),
            &endpoint
        ))
        .is_err()
    );
    // A consumer row with no admitted User at all is refused too.
    assert!(admit_with(observed(3, None, &endpoint)).is_err());

    // An endpoint row admitted for another reconnect generation is refused:
    // the derived row's fingerprint binds it to this session's generation.
    let other_generation = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        spec.host_ref().clone(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap()
    .with_reconnect_generation(2)
    .unwrap();
    let (_, other_endpoint) = derived_endpoints(&other_generation, &uid, 3)
        .into_iter()
        .find(|(reference, _)| reference == compositor.source_ref())
        .unwrap();
    assert!(
        admit_with(observed(3, Some(spec.user_ref().clone()), &other_endpoint)).is_err(),
        "an endpoint admitted for another reconnect generation cannot be reused"
    );

    // A compositor endpoint produced by another Host is not this session's
    // endpoint, so it is refused even when everything else matches.
    let other = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        ResourceRef::parse("Host/other-host").unwrap(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap();
    let (_, foreign) = derived_endpoints(&other, &uid, 3)
        .into_iter()
        .find(|(reference, _)| reference == compositor.source_ref())
        .unwrap();
    assert!(admit_with(observed(3, Some(spec.user_ref().clone()), &foreign)).is_err());
}

#[test]
fn an_absolute_display_string_cannot_expand_admitted_access() {
    let (guest, host, user, policy) = refs();
    let wire = |display: &str| {
        serde_json::json!({
            "guestRef": guest.to_canonical_string(),
            "hostRef": host.to_canonical_string(),
            "userRef": user.to_canonical_string(),
            "policyRef": policy.to_canonical_string(),
            "identity": {
                "label": "work-vm",
                "activeColor": "#7fc8ff",
                "inactiveColor": "#45475a",
                "urgentColor": "#f38ba8",
                "borderEnabled": true,
                "borderWidth": 2,
                "labelEnabled": true,
                "labelText": "work-vm",
                "labelPosition": "top-left"
            },
            "crossDomainTrusted": true,
            "virglVideo": false,
            "filter": {
                "debugLogging": false,
                "allowGlobals": [],
                "denyGlobals": [],
                "maxVersions": {},
                "dmabufAllow": [],
                "dmabufDeny": []
            },
            "compositorDisplay": display,
        })
    };

    for absolute in [
        "/run/user/1000/wayland-0",
        "/tmp/attacker.sock",
        "wayland-0/../wayland-1",
        "../wayland-1",
        "",
    ] {
        assert!(
            serde_json::from_value::<WaylandSessionSpec>(wire(absolute)).is_err(),
            "an absolute or nested display string must not decode: {absolute}"
        );
    }

    let spec: WaylandSessionSpec = serde_json::from_value(wire("wayland-1")).unwrap();
    assert_eq!(
        spec.compositor_display().map(|token| token.as_str()),
        Some("wayland-1")
    );
    let uid = session_uid("33333333-3333-4333-8333-333333333333");
    let bindings =
        d2b_provider_display_wayland::display_endpoint_bindings(&uid, &spec).unwrap();
    let compositor = bindings
        .iter()
        .find(|binding| binding.role() == DisplayProcessRole::HostProxy)
        .unwrap();
    assert_eq!(
        compositor.request().purpose().as_str(),
        "wayland-1",
        "the display name labels the admitted relationship"
    );
    assert!(admit(&spec, &uid, compositor, 1).is_ok());

    // Naming another display does not redirect the relationship: the endpoint
    // row this session derives for it is the only row that matches.
    let (_, endpoint) = derived_endpoints(&spec, &uid, 1)
        .into_iter()
        .find(|(reference, _)| reference == compositor.source_ref())
        .unwrap();
    let sibling = d2b_provider_display_wayland::EndpointSpec::new(
        endpoint.provider_ref().clone(),
        endpoint.producer_ref().clone(),
        endpoint.endpoint_class(),
        endpoint.transport(),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("wayland-9").unwrap(),
        endpoint.service_fingerprint().cloned(),
        endpoint.locality(),
        endpoint.visibility(),
        *endpoint.attachment_policy(),
        endpoint.consumer_policy().clone(),
        endpoint.lifecycle_policy(),
    )
    .unwrap();
    let refused = d2b_provider_display_wayland::DisplayEndpointObservation {
        spec: &sibling,
        source_generation: 1,
        consumer_generation: 1,
        consumer_user: Some(spec.user_ref().clone()),
    };
    assert!(
        d2b_provider_display_wayland::admit_display_endpoint(
            &zone(),
            &spec,
            &uid,
            compositor,
            &refused,
            &uid,
            &uid,
        )
        .is_err(),
        "a socket name that differs from the derived endpoint row is refused"
    );
}

#[test]
fn worker_rows_ask_for_no_privilege_beyond_their_declared_sandbox() {
    let spec = session_spec();
    let uid = session_uid("44444444-4444-4444-8444-444444444444");
    let intents = d2b_provider_display_wayland::session_children::display_owned_child_intents(
        &zone(),
        &ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/demo").unwrap(),
        &uid,
        &spec,
        2,
    )
    .unwrap();
    let workers = intents
        .iter()
        .filter(|intent| intent.target().resource_type().as_str() == "Process")
        .count();
    assert_eq!(workers, 2);
    for intent in &intents {
        let value: serde_json::Value = serde_json::from_slice(intent.canonical_resource()).unwrap();
        if value["type"] != serde_json::json!("Process") {
            continue;
        }
        assert_eq!(value["spec"]["userRef"], serde_json::json!(spec.user_ref().to_canonical_string()));
        assert_eq!(value["spec"]["sandbox"]["capabilityClasses"], serde_json::json!([]));
        assert_eq!(value["spec"]["sandbox"]["namespaceClasses"], serde_json::json!([]));
        assert_eq!(value["spec"]["sandbox"]["environmentClass"], serde_json::json!("minimal"));
        assert_eq!(value["spec"]["sandbox"]["noNewPrivileges"], serde_json::json!(true));
        assert_eq!(value["spec"]["sandbox"]["readOnlyRoot"], serde_json::json!(true));
        assert_eq!(value["spec"]["mounts"], serde_json::json!([]));
        assert_eq!(value["spec"]["deviceUsage"], serde_json::json!([]));
    }

    // A session that names an authorized policy records it on both worker rows.
    let policied = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        spec.host_ref().clone(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap()
    .with_execution_policy(Some(ResourceRef::parse("ExecutionPolicy/display-worker").unwrap()))
    .unwrap();
    assert!(
        WaylandSessionSpec::new(
            spec.guest_ref().clone(),
            spec.host_ref().clone(),
            spec.user_ref().clone(),
            spec.policy_ref().clone(),
            DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
            true,
        )
        .unwrap()
        .with_execution_policy(Some(ResourceRef::parse("SeccompProfile/strict").unwrap()))
        .is_err(),
        "a policy selection must name an ExecutionPolicy"
    );
    let annotated = d2b_provider_display_wayland::session_children::display_owned_child_intents(
        &zone(),
        &ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/demo").unwrap(),
        &uid,
        &policied,
        2,
    )
    .unwrap();
    for intent in &annotated {
        let value: serde_json::Value = serde_json::from_slice(intent.canonical_resource()).unwrap();
        if value["type"] != serde_json::json!("Process") {
            continue;
        }
        assert_eq!(
            value["metadata"]["annotations"]
                [d2b_provider_display_wayland::DISPLAY_EXECUTION_POLICY_ANNOTATION],
            serde_json::json!("ExecutionPolicy/display-worker")
        );
    }
    assert_ne!(
        policied.session_digest(1),
        spec.session_digest(1),
        "an authorized policy change is bound into the session digest"
    );
}

#[test]
fn filtering_still_applies_to_a_session_with_admitted_endpoint_access() {
    let spec = session_spec().with_filter(
        FilterInput::new(
            ["wl_compositor"],
            ["zwp_linux_dmabuf_v1", "wl_data_device_manager"],
            Vec::<(String, u32)>::new(),
            Vec::<String>::new(),
        )
        .unwrap(),
    );
    let uid = session_uid("55555555-5555-4555-8555-555555555555");
    let bindings =
        d2b_provider_display_wayland::display_endpoint_bindings(&uid, &spec).unwrap();
    for binding in &bindings {
        assert!(
            admit(&spec, &uid, binding, 1).is_ok(),
            "endpoint admission is independent of the protocol filter"
        );
    }
    let compiled = WaylandPolicy::compile(
        &FilterInput::default(),
        &FilterInput::default(),
        spec.filter(),
    )
    .unwrap();
    assert!(!compiled.is_allowed("zwp_linux_dmabuf_v1"));
    assert!(!compiled.is_allowed("wl_data_device_manager"));
    assert!(compiled.is_allowed("wl_compositor"));
}

// -- the endpoint shapes this Provider commits (U5, KTD5) ----------------------

/// The session uid one committed set of shapes is derived from.
fn committed_session_uid() -> ResourceUid {
    ResourceUid::parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap()
}

/// One admitted display session: its committed spec and its durable row
/// identity.
fn committed_session() -> (ResourceUid, ResourceRef, WaylandSessionSpec) {
    let spec = WaylandSessionSpec::new(
        ResourceRef::parse("Guest/work").unwrap(),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default").unwrap(),
        DisplayIdentity::new("work", "#112233", "#223344", "#334455").unwrap(),
        true,
    )
    .unwrap();
    (
        committed_session_uid(),
        ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-wayland").unwrap(),
        spec,
    )
}

/// The endpoint contracts the session's durable child rows carry, decoded back
/// out of the envelopes it commits.
fn committed_endpoint_specs(
    uid: &ResourceUid,
    session_ref: &ResourceRef,
    spec: &WaylandSessionSpec,
) -> Vec<EndpointSpec> {
    d2b_provider_display_wayland::session_children::display_owned_child_intents(
        &ZoneId::parse("work").unwrap(),
        session_ref,
        uid,
        spec,
        4,
    )
    .unwrap()
    .into_iter()
    .filter(|intent| intent.target().resource_type().as_str() == "Endpoint")
    .map(|intent| {
        let value: serde_json::Value =
            serde_json::from_slice(intent.canonical_resource()).unwrap();
        serde_json::from_value(value["spec"].clone()).unwrap()
    })
    .collect()
}

/// One committed shape with a single field edited, built through the contract's
/// own wire form so the result still decodes as a valid Endpoint spec: a
/// look-alike, not a malformed row.
fn mutated(spec: &EndpointSpec, field: &str, value: serde_json::Value) -> EndpointSpec {
    let mut wire = serde_json::to_value(spec).unwrap();
    wire.as_object_mut().unwrap().insert(field.to_owned(), value);
    serde_json::from_value(wire).unwrap()
}

/// The vocabulary admits exactly the three shapes the session's durable rows
/// carry, each with the realization it is served behind and the reconnect
/// generation its fingerprint is bound to (R14, R15).
///
/// The rows are read back out of the envelopes the session commits, so this
/// case proves the two derivations are ONE: what this Provider admits into the
/// Endpoint plane is exactly what it commits as a child row.
#[test]
fn the_vocabulary_admits_exactly_the_three_durable_shapes() {
    let (uid, session_ref, spec) = committed_session();
    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    let shapes = committed_endpoint_specs(&uid, &session_ref, &spec);
    assert_eq!(shapes.len(), 3, "the session commits three endpoint rows");

    let expected = [
        (0, EndpointRealization::HostSocketTransport),
        (1, EndpointRealization::WorkerDataAttachment),
        (2, EndpointRealization::WorkerCrossDomainTransport),
    ];
    for (index, realization) in expected {
        let committed = display_committed_endpoint_shape(&vocabulary, &shapes[index]);
        assert_eq!(
            committed.map(CommittedEndpointShape::realization),
            Some(realization),
            "the committed row is the shape the Endpoint plane serves"
        );
        assert_eq!(
            committed.map(CommittedEndpointShape::reconnect_generation),
            Some(spec.reconnect_generation()),
            "and the shape is bound to this session's reconnect generation"
        );
    }
}

/// A look-alike on ANY committed axis is a shape this Provider does not commit,
/// and the Endpoint driver refuses it terminally (R14).
#[test]
fn a_look_alike_on_any_committed_axis_is_not_committed() {
    let (uid, session_ref, spec) = committed_session();
    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    let shapes = committed_endpoint_specs(&uid, &session_ref, &spec);
    let proxy = shapes
        .iter()
        .find(|shape| shape.producer_ref().resource_type().as_str() == "Process")
        .expect("the host proxy's own endpoint row");
    assert!(
        display_committed_endpoint_shape(&vocabulary, proxy).is_some(),
        "the committed proxy shape is admitted"
    );

    let look_alikes = [
        (
            "provider",
            mutated(
                proxy,
                "providerRef",
                serde_json::json!("Provider/device-tpm"),
            ),
        ),
        (
            "producer",
            mutated(proxy, "producerRef", serde_json::json!("Process/impostor")),
        ),
        ("class", mutated(proxy, "endpointClass", serde_json::json!("service"))),
        ("transport", mutated(proxy, "transport", serde_json::json!("tcp"))),
        (
            "purpose",
            mutated(proxy, "purpose", serde_json::json!("wayland-other")),
        ),
        (
            "locality",
            mutated(proxy, "locality", serde_json::json!("host-local")),
        ),
        (
            "visibility",
            mutated(proxy, "visibility", serde_json::json!("provider")),
        ),
        (
            "lifecycle",
            mutated(proxy, "lifecyclePolicy", serde_json::json!("pinned")),
        ),
        (
            "attachment",
            mutated(
                proxy,
                "attachmentPolicy",
                serde_json::json!({"supported": true, "maxAttachments": 2}),
            ),
        ),
        (
            "publication",
            mutated(proxy, "bindingPublication", serde_json::json!("none")),
        ),
    ];
    assert_eq!(look_alikes.len(), 10, "every committed axis is covered");
    for (axis, look_alike) in look_alikes {
        assert_ne!(&look_alike, proxy, "the {axis} fixture is edited");
        assert!(
            display_committed_endpoint_shape(&vocabulary, &look_alike).is_none(),
            "this Provider commits no {axis} look-alike"
        );
    }
}

/// A shape minted for another reconnect generation is not a shape this
/// Provider commits now (R15).
///
/// The fingerprint is bound to the generation the session currently
/// authenticates, so a row still carrying an earlier generation's fingerprint
/// is refused rather than realized under the current one.
#[test]
fn a_shape_from_another_reconnect_generation_is_not_committed() {
    let (uid, session_ref, spec) = committed_session();
    let later = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        spec.host_ref().clone(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work", "#112233", "#223344", "#334455").unwrap(),
        true,
    )
    .unwrap()
    .with_reconnect_generation(spec.reconnect_generation() + 1)
    .unwrap();
    let now = committed_endpoint_specs(&uid, &session_ref, &spec);
    let next = committed_endpoint_specs(&uid, &session_ref, &later);
    assert_ne!(now, next, "the fingerprint is bound to the generation");

    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    for shape in &now {
        assert!(
            display_committed_endpoint_shape(&vocabulary, shape).is_some(),
            "the current generation's shapes are committed"
        );
    }
    for shape in &next {
        assert!(
            display_committed_endpoint_shape(&vocabulary, shape).is_none(),
            "the next generation's fingerprint is not a shape this session commits yet"
        );
    }
    let advanced = DisplayEndpointVocabulary::for_session(&uid, &later).unwrap();
    for shape in &next {
        assert_eq!(
            display_committed_endpoint_shape(&advanced, shape)
                .map(CommittedEndpointShape::reconnect_generation),
            Some(spec.reconnect_generation() + 1),
            "and once it does commit them, each shape carries that generation"
        );
    }
}

/// One session's vocabulary commits nothing of another session's.
#[test]
fn a_vocabulary_commits_only_its_own_sessions_shapes() {
    let (uid, session_ref, spec) = committed_session();
    let other_uid = ResourceUid::parse("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    for shape in committed_endpoint_specs(&other_uid, &session_ref, &spec) {
        assert!(
            display_committed_endpoint_shape(&vocabulary, &shape).is_none(),
            "another session's endpoint row is not a shape this vocabulary commits"
        );
    }
}
