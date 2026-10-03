use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_provider_display_wayland::{
    DisplayController, DisplayEndpointVocabulary, DisplayIdentity, DisplayLabelPosition,
    EndpointSpec, FilterInput, Phase, PolicyWarning, PrincipalPool, ProcessObservation,
    SharedDisplayEndpointVocabulary, WaylandPolicy, WaylandPolicySnapshot, WaylandSessionSpec,
    session_children,
};
use d2b_provider_endpoint::{
    CommittedEndpointShape, CommittedEndpointShapeSource, EndpointRealization,
};

/// The one shape this Provider commits for `spec`, asked through the seam the
/// production composition injects this Provider's vocabulary through.
fn seam_committed_shape(
    vocabulary: &SharedDisplayEndpointVocabulary,
    spec: &EndpointSpec,
) -> Option<CommittedEndpointShape> {
    vocabulary.committed_endpoint_shape(spec)
}

/// The shape this Provider commits for `spec`, asked through the Endpoint
/// family's own provider-neutral seam.
fn committed_shape(
    vocabulary: &DisplayEndpointVocabulary,
    spec: &EndpointSpec,
) -> Option<CommittedEndpointShape> {
    d2b_provider_endpoint::provider_committed_endpoint_shape(spec, vocabulary)
}

/// The same question under the name the vocabulary tests read it by.
fn display_committed_endpoint_shape(
    vocabulary: &DisplayEndpointVocabulary,
    spec: &EndpointSpec,
) -> Option<CommittedEndpointShape> {
    committed_shape(vocabulary, spec)
}

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
            serde_json::from_value::<EndpointSpec>(value["spec"].clone()).unwrap(),
        )
    })
    .collect()
}

/// The one committed endpoint row this session derives under `reference`.
fn derived_endpoint<'a>(
    endpoints: &'a [(ResourceRef, EndpointSpec)],
    reference: &ResourceRef,
) -> &'a EndpointSpec {
    endpoints
        .iter()
        .find(|(candidate, _)| candidate == reference)
        .map(|(_, spec)| spec)
        .expect("the derived endpoint row")
}

/// The canonical `EndpointBinding` rows one endpoint publishes to one consumer.
///
/// This is the launch gate's own derivation - the source's answer to "which
/// committed relationship rows does this exact Process require" - filtered to
/// the committed rows that endpoint actually publishes, so a consumer can
/// never widen its own relationship by asking for a different slot or a
/// different endpoint.
fn required_rows(
    endpoints: &[(ResourceRef, EndpointSpec)],
    endpoint_ref: &ResourceRef,
    consumer: &ResourceRef,
) -> Vec<ResourceRef> {
    let source = derived_endpoint(endpoints, endpoint_ref);
    let expected = d2b_provider_endpoint::expected_bindings_for_process(
        &zone(),
        source,
        endpoint_ref,
        consumer,
    )
    .expect("the source derives this consumer's expectation");
    session_children::display_canonical_bindings(&zone(), endpoint_ref, source)
        .expect("the committed endpoint derives its relationships")
        .into_iter()
        .filter(|row| {
            expected
                .iter()
                .any(|want| row.name().as_str() == want.name().as_str())
        })
        .collect()
}

/// The session's two workers and its three endpoint rows, named by the display
/// Provider's own durable derivation.
fn display_graph(uid: &ResourceUid) -> (ResourceRef, ResourceRef, ResourceRef, ResourceRef, ResourceRef) {
    (
        d2b_provider_display_wayland::durable_host_proxy_process_ref(uid).unwrap(),
        d2b_provider_display_wayland::durable_guest_frontend_process_ref(uid).unwrap(),
        d2b_provider_display_wayland::durable_compositor_endpoint_ref(uid).unwrap(),
        d2b_provider_display_wayland::durable_host_proxy_endpoint_ref(uid).unwrap(),
        d2b_provider_display_wayland::durable_wayland_endpoint_ref(uid).unwrap(),
    )
}

/// Each worker requires exactly the canonical `EndpointBinding` row its
/// endpoint's publication intent names for it (AE7-AE8, R18, R20).
///
/// The rows are the Endpoint family's own derivation of the committed
/// endpoint's publication intent - the same derivation the `Endpoint` actor
/// commits them from - so a worker can only ever be gated on a relationship
/// the graph actually owns. The host proxy's expectation and the guest
/// frontend's come from DIFFERENT endpoints, which is what makes the ordering
/// an evidence ordering: the frontend's requirement names the proxy's own
/// carriage, so it can only be satisfied once the proxy is standing, and no
/// direct parent launch can stand in for that.
#[test]
fn each_worker_requires_exactly_the_canonical_binding_row_publication_names() {
    let spec = session_spec();
    let uid = session_uid("11111111-1111-4111-8111-111111111111");
    let endpoints = derived_endpoints(&spec, &uid, 3);
    let (proxy, frontend, compositor, carriage, wayland) = display_graph(&uid);

    assert_eq!(
        required_rows(&endpoints, &compositor, &proxy).len(),
        1,
        "the host proxy consumes the session's host compositor socket, once"
    );
    assert_eq!(
        required_rows(&endpoints, &carriage, &frontend).len(),
        1,
        "the guest frontend consumes the proxy's own cross-domain carriage, once"
    );
    assert_ne!(
        required_rows(&endpoints, &compositor, &proxy)[0],
        required_rows(&endpoints, &carriage, &frontend)[0],
        "the two relationships are two distinct committed rows"
    );

    // Neither worker consumes anything else: the proxy never reaches its own
    // carriage, and the frontend never reaches the compositor socket.
    assert!(required_rows(&endpoints, &carriage, &proxy).is_empty());
    assert!(required_rows(&endpoints, &compositor, &frontend).is_empty());

    // The compositor row publishes to the proxy row and to nobody else.
    assert_eq!(
        derived_endpoint(&endpoints, &compositor)
            .consumer_policy()
            .allowed_subjects(),
        &[proxy.clone()]
    );

    // The guest frontend's own Endpoint publishes nothing, so it derives no
    // row at all: it gates the session's aggregate readiness instead of
    // delivering anything in-Zone (R20).
    assert!(
        session_children::display_canonical_bindings(
            &zone(),
            &wayland,
            derived_endpoint(&endpoints, &wayland),
        )
        .expect("the frontend endpoint derives its relationships")
        .is_empty(),
        "the guest frontend's endpoint publishes nothing and therefore derives no row"
    );
    assert!(required_rows(&endpoints, &wayland, &proxy).is_empty());
    assert!(required_rows(&endpoints, &wayland, &frontend).is_empty());

    // The session's projected Wayland endpoint is that frontend-produced row,
    // named by the Provider's own derivation rather than picked out of a list
    // of children (R23).
    assert_eq!(
        wayland,
        d2b_provider_display_wayland::durable_wayland_endpoint_ref(&uid).unwrap(),
    );
    assert_ne!(wayland, carriage);
    assert_ne!(wayland, compositor);
    assert_eq!(
        derived_endpoint(&endpoints, &wayland).producer_ref(),
        &frontend,
        "the projected wayland endpoint is produced by the guest frontend itself"
    );
}

/// Only a delivered projection at the binding row's OWN current generation
/// proves the relationship is standing (R20).
///
/// Every other closed state - a replaced endpoint, an undelivered row, a
/// draining row, an unreadable layer, and a delivery published for an earlier
/// row generation - is the same answer: not standing. A session that cannot
/// prove its delivery is not usable, however Ready its worker rows look.
#[test]
fn only_a_delivery_at_the_rows_own_generation_proves_the_relationship() {
    let (uid, _, spec) = committed_session();
    let (_, _, compositor, _, _) = display_graph(&uid);
    let delivered = d2b_provider_endpoint::BindingDeliveryProjection::Delivered {
        incarnation: d2b_provider_endpoint::endpoint::RealizationIncarnation::derive(
            zone().as_str(),
            &compositor,
            4,
            uid.as_str(),
            1,
            spec.reconnect_generation(),
            None,
        )
        .expect("a committed row derives an incarnation"),
        generation: 4,
    };
    let at = |generation: u64| {
        session_children::display_binding_delivered(Some(&delivered.projection()), generation)
    };
    assert!(at(4), "the row's own current generation is delivered");
    assert!(
        !at(5),
        "a delivery published for an earlier row generation proves nothing"
    );
    assert!(
        !session_children::display_binding_delivered(None, 4),
        "an absent published layer proves nothing"
    );
    for state in ["undelivered", "endpoint-replaced", "draining"] {
        assert!(
            !session_children::display_binding_delivered(
                Some(&serde_json::json!({ "state": state })),
                4
            ),
            "{state} is not a delivery"
        );
    }
    assert!(
        !session_children::display_binding_delivered(
            Some(&serde_json::json!({"state": "delivered"})),
            4
        ),
        "a delivery layer that carries no incarnation cannot be read back"
    );
}

/// An endpoint row admitted for another reconnect generation or another Host
/// is not a shape this Provider commits, so the Endpoint plane admits no
/// relationship over it at all.
///
/// This is where the display-local admission that used to sit beside the
/// committed rows went: the committed row IS the authority, and the only
/// question left is whether the row a reader sees is one this Provider
/// committed for this session.
#[test]
fn a_foreign_or_stale_endpoint_row_is_no_shape_this_provider_commits() {
    let spec = session_spec();
    let uid = session_uid("22222222-2222-4222-8222-222222222222");
    let (_, _, compositor, _, _) = display_graph(&uid);
    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    let committed = derived_endpoints(&spec, &uid, 3);
    assert!(
        committed_shape(&vocabulary, derived_endpoint(&committed, &compositor)).is_some(),
        "the session's own compositor row is a shape this Provider commits"
    );

    let stale = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        spec.host_ref().clone(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap()
    .with_reconnect_generation(spec.reconnect_generation() + 1)
    .unwrap();
    let other = WaylandSessionSpec::new(
        spec.guest_ref().clone(),
        ResourceRef::parse("Host/other-host").unwrap(),
        spec.user_ref().clone(),
        spec.policy_ref().clone(),
        DisplayIdentity::new("work-vm", "#7fc8ff", "#45475a", "#f38ba8").unwrap(),
        true,
    )
    .unwrap();
    for foreign in [stale, other] {
        let rows = derived_endpoints(&foreign, &uid, 3);
        assert!(
            committed_shape(&vocabulary, derived_endpoint(&rows, &compositor)).is_none(),
            "an endpoint admitted for another generation or another Host is not a \
             shape this session commits"
        );
    }
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
    let vocabulary = DisplayEndpointVocabulary::for_session(&uid, &spec).unwrap();
    let (_, _, compositor, _, _) = display_graph(&uid);
    let committed = derived_endpoints(&spec, &uid, 1);
    let endpoint = derived_endpoint(&committed, &compositor);
    assert_eq!(
        endpoint.purpose().as_str(),
        "wayland-1",
        "the display name labels the endpoint this session commits"
    );
    assert!(
        committed_shape(&vocabulary, endpoint).is_some(),
        "the endpoint the display name selects is the one this Provider commits"
    );

    // Naming another display does not redirect the relationship: the endpoint
    // row this session commits is the only row that matches, so a socket name
    // that differs from it is not a shape this Provider commits at all.
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
    assert!(
        committed_shape(&vocabulary, &sibling).is_none(),
        "a socket name that differs from the committed endpoint row is not a \
         shape this Provider commits"
    );
    // A row rebuilt without the publication intent this session committed
    // publishes nothing, so there is no relationship to serve over it at all -
    // the committed row is the only one carrying that intent, and therefore the
    // only one that derives a row.
    assert!(
        session_children::display_canonical_bindings(&zone(), &compositor, &sibling)
            .expect("a valid endpoint row is derivable")
            .is_empty(),
        "a socket name that drops the committed publication intent derives no row"
    );
    assert_eq!(
        session_children::display_canonical_bindings(&zone(), &compositor, endpoint)
            .expect("the committed row publishes its one relationship")
            .len(),
        1,
        "the committed row itself still publishes exactly its one relationship"
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
    let endpoints = derived_endpoints(&spec, &uid, 1);
    let (proxy, frontend, compositor, carriage, _) = display_graph(&uid);
    assert_eq!(
        required_rows(&endpoints, &compositor, &proxy).len()
            + required_rows(&endpoints, &carriage, &frontend).len(),
        2,
        "the session still derives exactly its two relationships; the protocol \
         filter is independent of them"
    );
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

// -- the Zone-wide vocabulary the production composition injects (KTD5) --------

/// The registry the production composition installs admits exactly the shapes
/// the sessions committed here commit, and nothing else (KTD5, R14).
///
/// This is the seam's own question, asked the way the Endpoint family asks it:
/// the object answers with the one shape it commits, matched in full, and a
/// row it does not commit is `None` - which the Endpoint driver turns into a
/// terminal refusal rather than a near miss.
#[test]
fn the_shared_vocabulary_admits_exactly_the_committed_sessions_shapes() {
    let (uid, session_ref, spec) = committed_session();
    let vocabulary = SharedDisplayEndpointVocabulary::new();
    let shapes = committed_endpoint_specs(&uid, &session_ref, &spec);
    assert_eq!(shapes.len(), 3, "the session commits three endpoint rows");

    for shape in &shapes {
        assert_eq!(
            seam_committed_shape(&vocabulary, shape),
            None,
            "an empty registry has committed no session, so it admits nothing"
        );
    }

    vocabulary.commit_session(&uid, &spec).unwrap();
    let expected = [
        (0, EndpointRealization::HostSocketTransport),
        (1, EndpointRealization::WorkerDataAttachment),
        (2, EndpointRealization::WorkerCrossDomainTransport),
    ];
    for (index, realization) in expected {
        let committed = seam_committed_shape(&vocabulary, &shapes[index]);
        assert_eq!(
            committed.map(CommittedEndpointShape::realization),
            Some(realization),
            "the committed row is served behind the realization this Provider names for it"
        );
        assert_eq!(
            committed.map(CommittedEndpointShape::reconnect_generation),
            Some(spec.reconnect_generation()),
            "and each shape is bound to the generation the session currently authenticates"
        );
    }

    // Another session's rows are not a shape this registry commits, even
    // though they came out of the very same derivation.
    let other_uid = session_uid("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    for shape in committed_endpoint_specs(&other_uid, &session_ref, &spec) {
        assert_eq!(
            seam_committed_shape(&vocabulary, &shape),
            None,
            "a session that was never admitted contributes no shape"
        );
    }
}

/// A look-alike is refused through the injected registry on every committed
/// axis, exactly as it is through one session's own vocabulary (R14).
#[test]
fn a_look_alike_is_not_admitted_through_the_shared_vocabulary() {
    let (uid, session_ref, spec) = committed_session();
    let vocabulary = SharedDisplayEndpointVocabulary::new();
    vocabulary.commit_session(&uid, &spec).unwrap();
    let shapes = committed_endpoint_specs(&uid, &session_ref, &spec);
    let proxy = shapes
        .iter()
        .find(|shape| shape.producer_ref().resource_type().as_str() == "Process")
        .expect("the host proxy's own endpoint row");
    assert!(
        seam_committed_shape(&vocabulary, proxy).is_some(),
        "the committed proxy shape is admitted"
    );

    for (axis, look_alike) in [
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
        (
            "class",
            mutated(proxy, "endpointClass", serde_json::json!("service")),
        ),
        (
            "transport",
            mutated(proxy, "transport", serde_json::json!("tcp")),
        ),
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
            "fingerprint",
            mutated(
                proxy,
                "serviceFingerprint",
                serde_json::json!("display-wayland-data-v3-r9"),
            ),
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
    ] {
        assert_ne!(&look_alike, proxy, "the {axis} fixture is edited");
        assert_eq!(
            seam_committed_shape(&vocabulary, &look_alike),
            None,
            "the shared vocabulary commits no {axis} look-alike either"
        );
    }
}

/// Re-committing a session REPLACES the shapes it committed before, so a
/// replaced session's shape is never admitted from a stale entry (R15).
///
/// The session is the same row with the same uid; only its authenticated
/// reconnect generation moved. Its earlier endpoint rows are gone from the
/// plane, and nothing the registry still holds may answer for them.
#[test]
fn recommitting_a_session_supersedes_the_shapes_it_committed_before() {
    let (uid, session_ref, spec) = committed_session();
    let next = WaylandSessionSpec::new(
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
    let superseded = committed_endpoint_specs(&uid, &session_ref, &spec);
    let current = committed_endpoint_specs(&uid, &session_ref, &next);

    let vocabulary = SharedDisplayEndpointVocabulary::new();
    vocabulary.commit_session(&uid, &spec).unwrap();
    for shape in &superseded {
        assert!(
            seam_committed_shape(&vocabulary, shape).is_some(),
            "the shapes this session committed are admitted while it commits them"
        );
    }

    vocabulary.commit_session(&uid, &next).unwrap();
    for shape in &superseded {
        assert_eq!(
            seam_committed_shape(&vocabulary, shape),
            None,
            "a superseded generation's shape is not admitted once the session moved on"
        );
    }
    for shape in &current {
        assert_eq!(
            seam_committed_shape(&vocabulary, shape)
                .map(CommittedEndpointShape::reconnect_generation),
            Some(next.reconnect_generation()),
            "and the shapes it commits now are admitted at the generation it now authenticates"
        );
    }
}
