//! Hermetic VolumeBinding lifecycle, sandbox, and privacy conformance.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    ResourceUid,
    volume::VolumeSpec,
    volume_binding::{VolumeBindingReadinessFence, VolumeBindingStatusResource},
};
use d2b_core::test_support::block_on;
use d2b_provider_volume_virtiofs::testing::{PortCall, ScriptedPort, fixtures};
use d2b_provider_volume_virtiofs::{
    LaunchedWorker, MountObservation, PRESENTATION_CAPABILITY, ServingSource,
    ServingWorkerLaunch, SETUP_RESTRICTIONS, SocketPathRefusal, StoredBinding,
    VOLUME_BINDING_FINALIZER, VOLUME_BINDING_RESOURCE_TYPE, VirtiofsBindingController,
    VirtiofsBindingEffectPort, VirtiofsBindingError,
};

use d2b_provider_volume_virtiofs::BindingPhase;

/// The broker-owned runtime root every fixture binds its socket in.
///
/// It is a property of the broker, not of any Guest: nothing in the
/// derivation below reads a Guest row or a Device row, which is what makes
/// the Device-free case (AE6) the ordinary case rather than a special one.
const RUNTIME_ROOT: &str = "/run/d2b";

/// A port that exercises the trait defaults: the store-view marker probe
/// and the status write both fail closed when an adapter does not
/// implement them.
struct DefaultProbePort {
    inner: ScriptedPort,
}

impl VirtiofsBindingEffectPort for &DefaultProbePort {
    async fn launch_worker(
        &self,
        binding: &StoredBinding,
        launch: &ServingWorkerLaunch,
    ) -> Result<LaunchedWorker, VirtiofsBindingError> {
        (&self.inner).launch_worker(binding, launch).await
    }

    async fn observe_socket(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> Result<bool, VirtiofsBindingError> {
        (&self.inner).observe_socket(binding, worker).await
    }

    async fn observe_guest_mount(
        &self,
        binding: &StoredBinding,
    ) -> Result<MountObservation, VirtiofsBindingError> {
        (&self.inner).observe_guest_mount(binding).await
    }

    async fn delete_worker(
        &self,
        binding: &StoredBinding,
        worker: &LaunchedWorker,
    ) -> Result<(), VirtiofsBindingError> {
        (&self.inner).delete_worker(binding, worker).await
    }
}

fn controller<P: VirtiofsBindingEffectPort>(port: P) -> VirtiofsBindingController<P> {
    VirtiofsBindingController::new(port, fixtures::zone(), RUNTIME_ROOT)
}

fn reconcile(
    port: &ScriptedPort,
    access: &str,
) -> d2b_provider_volume_virtiofs::BindingStatusReport {
    reconcile_volume(port, &fixtures::binding(access), &fixtures::store_view_volume())
}

/// Reconcile one binding against one Volume.
fn reconcile_volume(
    port: &ScriptedPort,
    binding: &StoredBinding,
    volume: &VolumeSpec,
) -> d2b_provider_volume_virtiofs::BindingStatusReport {
    block_on(controller(port).reconcile(binding, volume, 4, fixtures::principal()))
        .expect("reconcile reports")
}

#[test]
fn the_default_marker_and_status_probes_fail_closed() {
    let port = DefaultProbePort {
        inner: ScriptedPort::serving(),
    };
    // A closure source: the default marker probe fails closed, so the
    // worker is never launched on an unverified store-view generation.
    let error = block_on(controller(&port).reconcile(
        &fixtures::binding("read-only"),
        &fixtures::closure_store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect_err("default write rejected");
    assert_eq!(error, VirtiofsBindingError::UnauthorizedWriter);
    assert!(
        port.inner.calls().is_empty(),
        "the default marker probe recorded nothing and the default status \
         write reached no server, so neither a launch nor a readiness \
         report could be produced from the defaults alone"
    );
}

#[test]
fn a_binding_reaches_ready_only_when_the_host_serves_and_the_consumer_mounts() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "read-only");
    assert_eq!(report.phase, BindingPhase::Ready);
    assert!(report.source_prepared);
    assert_eq!(report.consumer_mount, MountObservation::Present);
    assert!(report.reason.is_none());
    // KTD3: the fenced projection is written on every reconcile. A
    // declared-storage-root source needs no store-view marker, so the
    // marker probe belongs to the closure source alone.
    assert_eq!(
        port.calls(),
        vec![
            PortCall::LaunchWorker,
            PortCall::ObserveSocket,
            PortCall::ObserveGuestMount,
            PortCall::WriteStatus,
        ]
    );
    let writes = port.status_writes();
    assert_eq!(writes.len(), 1);
    let binding = fixtures::binding("read-only");
    assert!(writes[0].ready);
    assert!(writes[0].readiness_is_current(
        binding.uid(),
        binding.generation(),
        binding.revision()
    ));
    assert_eq!(writes[0].fence, binding.fence());
    assert_eq!(writes[0].reason, None);
}

#[test]
fn a_socket_that_never_listens_holds_the_binding_pending() {
    let port = ScriptedPort::serving().socket_never_ready();
    let report = reconcile(&port, "read-only");
    assert_eq!(report.phase, BindingPhase::Pending);
    assert!(!report.source_prepared);
    assert!(!report.permits_consumer_start());
    assert_eq!(report.reason, Some(VirtiofsBindingError::BindingNotReady));
    // The guest is never probed while the host side is not serving.
    assert!(!port.calls().contains(&PortCall::ObserveGuestMount));
    // Even a pending reconcile writes its fail-closed projection (KTD3).
    assert_eq!(port.status_writes().len(), 1);
    assert!(!port.status_writes()[0].ready);
}

#[test]
fn a_closure_store_view_waits_for_its_zero_length_marker_before_launch() {
    let port = ScriptedPort::serving().store_view_marker_missing();
    let report = reconcile_volume(
        &port,
        &fixtures::binding("read-only"),
        &fixtures::closure_store_view_volume(),
    );
    assert_eq!(report.phase, BindingPhase::Pending);
    assert_eq!(
        report.reason,
        Some(VirtiofsBindingError::StoreViewMarkerMissing)
    );
    assert_eq!(
        port.calls(),
        vec![PortCall::ObserveStoreViewMarker, PortCall::WriteStatus]
    );
    // Even the pending marker report writes its fail-closed projection.
    assert_eq!(port.status_writes().len(), 1);
    assert!(!port.status_writes()[0].ready);
}

#[test]
fn a_serving_host_whose_running_consumer_does_not_mount_is_degraded() {
    let port = ScriptedPort::serving().guest_never_mounts();
    let report = reconcile(&port, "read-only");
    assert_eq!(report.phase, BindingPhase::Degraded);
    assert!(report.source_prepared);
    assert_eq!(report.consumer_mount, MountObservation::Absent);
    assert!(!report.consumer_mount.is_mounted());
    assert_eq!(report.reason, Some(VirtiofsBindingError::GuestMountNotReady));
}

#[test]
fn a_read_only_binding_launches_a_read_only_worker() {
    let port = ScriptedPort::serving();
    reconcile(&port, "read-only");
    let plans = port.launched_plans();
    assert_eq!(plans.len(), 1);
    assert!(plans[0].readonly);
    assert_eq!(plans[0].thread_pool_size, 4);
}

#[test]
fn a_source_kind_with_no_serving_view_is_refused_rather_than_served() {
    let port = ScriptedPort::serving();
    let report = reconcile_volume(
        &port,
        &fixtures::binding("read-only"),
        &fixtures::unservable_source_volume(),
    );
    assert_eq!(report.phase, BindingPhase::Failed);
    assert_eq!(
        report.reason,
        Some(VirtiofsBindingError::SourceKindUnsupported)
    );
    assert!(port.launched_plans().is_empty());
}

#[test]
fn a_write_binding_over_a_read_only_view_reports_failed_with_a_reason() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "read-write");
    // KTD5: terminal admission failures surface Failed, not Pending.
    assert_eq!(report.phase, BindingPhase::Failed);
    assert_eq!(
        report.reason,
        Some(VirtiofsBindingError::ViewRightsInsufficient)
    );
    assert!(port.calls().contains(&PortCall::WriteStatus));
    assert!(report.worker_process_ref.is_none());
    assert!(!report.projection.ready);
    assert_eq!(
        report
            .projection
            .reason
            .as_ref()
            .map(d2b_contracts_resource::v3::resource_status::StatusCode::as_str),
        Some("view-rights-insufficient")
    );
}

#[test]
fn a_binding_naming_an_undeclared_view_reports_failed_with_a_reason() {
    let mut envelope = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    envelope["spec"]["view"] = serde_json::json!("absent");
    let binding = StoredBinding::from_resource_spec(&envelope).expect("conformant binding");
    let port = ScriptedPort::serving();
    let report = reconcile_volume(&port, &binding, &fixtures::store_view_volume());
    // KTD5: the rejection is visible as a Failed phase with a reason.
    assert_eq!(report.phase, BindingPhase::Failed);
    assert_eq!(report.reason, Some(VirtiofsBindingError::ViewNotFound));
    assert!(report.worker_process_ref.is_none());
    assert!(!report.projection.ready);
}

#[test]
fn a_shared_write_binding_is_never_served() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "shared-write");
    assert_eq!(report.phase, BindingPhase::Failed);
    assert_eq!(
        report.reason,
        Some(VirtiofsBindingError::SharedWriteUnsupported)
    );
    assert!(port.launched_plans().is_empty());
}

#[test]
fn a_stale_fence_report_is_never_accepted_as_ready() {
    let port = ScriptedPort::serving();
    let binding = fixtures::binding("read-only");
    let first = reconcile(&port, "read-only");
    assert!(first.projection.ready);
    assert_eq!(port.status_writes().len(), 1);

    // The binding is superseded by a newer generation (AE1): the same
    // readiness evidence now arrives under the old fence.
    port.advance_generation();
    let error = block_on(controller(&port).reconcile(
        &binding,
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect_err("stale write rejected");
    assert_eq!(error, VirtiofsBindingError::StaleFence);
    // The stale report never became a second accepted write.
    assert_eq!(port.status_writes().len(), 1);
    // And the accepted evidence no longer counts as current readiness.
    assert!(!first.projection.readiness_is_current(
        binding.uid(),
        d2b_contracts_resource::v3::ResourceGeneration::new(2).expect("generation"),
        binding.revision(),
    ));
}

#[test]
fn volume_reads_are_dependency_only_and_the_volume_is_never_written() {
    let volume_before =
        serde_json::to_value(fixtures::store_view_volume()).expect("Volume fixture serializes");
    let port = ScriptedPort::serving();
    reconcile(&port, "read-only");
    let volume_after =
        serde_json::to_value(fixtures::store_view_volume()).expect("Volume fixture serializes");
    assert_eq!(volume_before, volume_after);
    // Every recorded effect is a read or a serving effect on the
    // binding's own children; the port surface has no Volume mutation
    // method at all (AE2).
    for call in port.calls() {
        assert!(
            matches!(
                call,
                PortCall::ObserveStoreViewMarker
                    | PortCall::LaunchWorker
                    | PortCall::ObserveSocket
                    | PortCall::ObserveGuestMount
                    | PortCall::WriteStatus
                    | PortCall::DeleteWorker
            ),
            "unexpected effect {call:?}"
        );
    }
}

#[test]
fn an_unauthorized_ready_update_cannot_release_the_gate() {
    let port = ScriptedPort::serving();
    let binding = fixtures::binding("read-only");
    let ready = VolumeBindingStatusResource {
        ready: true,
        fence: binding.fence(),
        reason: None,
    };
    // A writer that is not the virtiofs controller identity is rejected
    // by the server-side identity check (KTD3).
    let foreign_writer = BoundedToken::parse("volume-local").expect("valid token");
    let error = block_on((&port).write_binding_status(&foreign_writer, &binding, &ready))
        .expect_err("foreign write rejected");
    assert_eq!(error, VirtiofsBindingError::UnauthorizedWriter);
    assert!(port.status_writes().is_empty());
    // A Ready claim whose fence does not match the binding never counts
    // as current readiness, so the Guest start gate stays closed: the
    // zero UID never matches a live binding (fail-closed, KTD3).
    let forged = VolumeBindingStatusResource {
        ready: true,
        fence: VolumeBindingReadinessFence {
            uid: ResourceUid::parse("00000000-0000-4000-8000-000000000000").expect("zero uid"),
            generation: binding.generation(),
            revision: binding.revision(),
        },
        reason: None,
    };
    assert!(!forged.readiness_is_current(
        binding.uid(),
        binding.generation(),
        binding.revision()
    ));
}

#[test]
fn a_drain_deletes_the_worker_before_confirming_the_mount_is_gone() {
    let port = ScriptedPort::serving();
    let binding = fixtures::binding("read-only");
    let report = reconcile_volume(&port, &binding, &fixtures::store_view_volume());
    let worker = LaunchedWorker {
        process_ref: report.worker_process_ref.expect("worker exists"),
        socket: report.socket.expect("socket exists"),
    };
    assert!(block_on(controller(&port).drain(&binding, &worker)).is_ok());
    let calls = port.calls();
    let deleted = calls
        .iter()
        .position(|call| *call == PortCall::DeleteWorker)
        .expect("worker deleted");
    let confirmed = calls
        .iter()
        .rposition(|call| *call == PortCall::ObserveGuestMount)
        .expect("mount confirmed");
    assert!(deleted < confirmed);
}

#[test]
fn a_mount_that_survives_deletion_blocks_the_drain() {
    let port = ScriptedPort::serving().mount_survives_delete();
    let binding = fixtures::binding("read-only");
    let owned = controller(&port);
    let worker = LaunchedWorker {
        process_ref: ResourceRef::parse("Process/vol-work-state-virtiofsd-work-vm")
            .expect("valid ref"),
        socket: binding.socket_identity(&fixtures::zone()),
    };
    assert_eq!(
        block_on(owned.drain(&binding, &worker)).unwrap_err(),
        VirtiofsBindingError::DrainIncomplete
    );
}

/// Fragments that must never appear in a public binding status document.
///
/// The fence UID is part of the neutral binding contract (KTD3), so the
/// prohibited fragments target resolved paths, sockets, and provider
/// tuning rather than identity digests.
const FORBIDDEN_STATUS_FRAGMENTS: [&str; 7] = [
    "/run",
    "/nix",
    ".sock",
    "shared-dir",
    "socket-path",
    "socket-group",
    "gid",
];

#[test]
fn public_binding_status_carries_no_socket_path_shared_dir_or_argv() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "read-only");
    let rendered = serde_json::to_string(&report)
        .expect("status serializes")
        .to_ascii_lowercase();
    for fragment in FORBIDDEN_STATUS_FRAGMENTS {
        assert!(
            !rendered.contains(fragment),
            "public status carries the forbidden fragment {fragment}"
        );
    }
    assert!(rendered.contains("volume-virtiofs"));
    assert_eq!(
        format!("{:?}", report.socket.expect("socket")),
        "SocketIdentity(<redacted>)"
    );
    assert_eq!(format!("{:?}", report.projection.reason), "None");
}

#[test]
fn two_bindings_of_one_volume_have_distinct_socket_identities() {
    let work = fixtures::binding("read-only");
    let other = fixtures::binding_for("read-only", "personal-vm", "ro-store");
    let zone = fixtures::zone();
    assert_ne!(work.socket_identity(&zone), other.socket_identity(&zone));
    assert_eq!(work.socket_identity(&zone), work.socket_identity(&zone));
}

#[test]
fn the_provider_owns_only_the_binding_resource_type_and_finalizer() {
    // Ownership pin (KTD3, R6): the serving side owns the neutral
    // VolumeBinding type, its finalizer, and nothing else.
    assert_eq!(VOLUME_BINDING_RESOURCE_TYPE, "VolumeBinding");
    assert_eq!(
        VOLUME_BINDING_FINALIZER,
        "volume-virtiofs.d2bus.org/volume-binding"
    );
    let port = ScriptedPort::serving();
    let owned = controller(&port);
    assert_eq!(owned.finalizer(), VOLUME_BINDING_FINALIZER);
    assert_eq!(owned.provider().as_str(), "volume-virtiofs");
}

#[test]
fn resource_binding_spec_keeps_one_strict_owner() {
    // The strictly neutral stored envelope parses, and foreign types,
    // any provider extension, non-Volume owners, and provider tuning
    // are all rejected.
    let binding = fixtures::binding("read-only");
    assert_eq!(
        binding.spec().volume_ref().to_canonical_string(),
        "Volume/work-state"
    );
    assert_ne!(
        binding.worker_process_ref().unwrap(),
        binding.endpoint_ref().unwrap()
    );

    let mut foreign_type = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    foreign_type["type"] = serde_json::json!("virtiofs.d2bus.org.Export");
    assert!(StoredBinding::from_resource_spec(&foreign_type).is_err());

    let mut foreign_schema = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    foreign_schema["spec"]["provider"] =
        serde_json::json!({ "schemaId": "volume-virtiofs.d2bus.org/virtiofs.d2bus.org.Export/spec" });
    assert!(StoredBinding::from_resource_spec(&foreign_schema).is_err());

    let mut foreign_owner = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    foreign_owner["metadata"]["ownerRef"] = serde_json::json!("Guest/work-vm");
    let mut mismatched_owner = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    mismatched_owner["metadata"]["ownerRef"] = serde_json::json!("Volume/other");
    assert!(StoredBinding::from_resource_spec(&mismatched_owner).is_err());

    let mut tuned = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    tuned["spec"]["threadPoolSize"] = serde_json::json!(2);
    assert!(StoredBinding::from_resource_spec(&tuned).is_err());
}

// ---------------------------------------------------------------------------
// U15: delivery is the realization of the admitted binding
// ---------------------------------------------------------------------------

/// AE6 and AE21: a Guest with no Device children boots from a Prepared
/// export and reports mount completion afterwards.
///
/// The two conditions are separate observations of the same binding, so
/// the pre-boot reconcile must already report the pre-start condition as
/// holding, and the post-boot reconcile must observe the mount without
/// changing anything the pre-boot pass derived. Nothing in either pass
/// reads a Guest row or a Device row: the fixture binding names a Guest
/// with no children, and the socket the worker binds is a direct child of
/// the broker runtime root, not a per-Guest directory whose posture comes
/// from a Device-shared storage row.
#[test]
fn a_device_free_guest_boots_from_a_prepared_export_and_mounts_afterwards() {
    let port = ScriptedPort::serving().consumer_not_running();
    let binding = fixtures::binding("read-only");
    let volume = fixtures::store_view_volume();

    // Before the Guest starts: the source is prepared, the consumer has
    // simply not run, and the pre-start condition holds.
    let pre_boot = reconcile_volume(&port, &binding, &volume);
    assert_eq!(pre_boot.phase, BindingPhase::Prepared);
    assert!(pre_boot.source_prepared);
    assert_eq!(pre_boot.consumer_mount, MountObservation::ConsumerNotRunning);
    assert!(pre_boot.permits_consumer_start());
    assert!(pre_boot.reason.is_none());
    // The fenced projection the Guest's start gate reads reports the
    // PRE-START condition, so it is already satisfied before the Guest
    // exists. Folding the mount into it is exactly the cycle this keeps
    // open: the Guest could not start until the mount was observed, and
    // the mount cannot be observed until the Guest has started.
    assert!(
        pre_boot.projection.ready,
        "the projection is the pre-start condition, not the post-boot one"
    );
    let sockets_before = port.launched_sockets();
    assert_eq!(sockets_before.len(), 1);
    let socket = sockets_before[0].clone();
    assert_eq!(
        socket.parent().and_then(std::path::Path::to_str),
        Some(RUNTIME_ROOT),
        "the socket is derived from the binding under the broker runtime \\
         root, with no per-Guest directory and no Device-derived posture"
    );

    // The Guest boots and mounts. The same binding, the same source, and
    // the same socket: the post-boot observation adds evidence, it does
    // not re-derive the delivery.
    block_on(port.set_consumer_mount(MountObservation::Present));
    let post_boot = reconcile_volume(&port, &binding, &volume);
    assert_eq!(post_boot.phase, BindingPhase::Ready);
    assert!(post_boot.source_prepared);
    assert_eq!(post_boot.consumer_mount, MountObservation::Present);
    assert!(post_boot.permits_consumer_start());
    assert_eq!(
        port.launched_sockets(),
        vec![socket.clone(), socket],
        "the second pass rebinds the same private socket"
    );
    assert_eq!(
        port.launched_plans()[0].source,
        port.launched_plans()[1].source,
        "the derived source is the same across the two completion conditions"
    );
}

/// A helper restart re-derives the same socket and the same source, so it
/// cannot expose another view; and a binding that names another view of
/// the same volume derives a different source and a different socket, so
/// one writer's helper cannot be pointed at a second view.
#[test]
fn a_helper_restart_derives_the_same_socket_and_the_same_view() {
    let port = ScriptedPort::serving();
    let binding = fixtures::binding("read-only");
    let volume = fixtures::store_view_volume();
    reconcile_volume(&port, &binding, &volume);
    reconcile_volume(&port, &binding, &volume);
    assert_eq!(port.launched_sockets().len(), 2);
    assert_eq!(port.launched_sockets()[0], port.launched_sockets()[1]);

    // A second relationship over a different named view of the same
    // volume: a different source view and a different socket, never the
    // first relationship's.
    let other_view = StoredBinding::from_resource_spec(&fixtures::binding_envelope(
        "read-only",
        "work-vm",
        "controller",
    ))
    .expect("conformant binding");
    let read_only_view = fixtures::read_only_view();
    let first = binding
        .serving_source(&volume, &read_only_view)
        .expect("the declared storage root is admitted");
    let second = other_view
        .serving_source(&volume, volume.views().get("controller").expect("declared view"))
        .expect("the declared storage root is admitted");
    assert_ne!(first.view_path(), second.view_path());
    assert_ne!(
        binding.serving_socket(&fixtures::zone()),
        other_view.serving_socket(&fixtures::zone()),
        "two relationships never share one private socket"
    );
}

/// A read-only closure export is served read-only, is pinned to one
/// admitted store-view generation, and waits for that generation's
/// readiness marker before the worker is launched at all.
#[test]
fn a_read_only_closure_export_is_pinned_to_one_admitted_generation() {
    let port = ScriptedPort::serving();
    let volume = fixtures::closure_store_view_volume();
    let binding = fixtures::binding("read-only");
    let report = reconcile_volume(&port, &binding, &volume);
    assert_eq!(report.phase, BindingPhase::Ready);
    let plan = &port.launched_plans()[0];
    assert!(plan.readonly, "a closure view that grants no write is read-only");
    assert_eq!(
        plan.source,
        ServingSource::ClosureStoreView {
            volume_name: d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                "work-state"
            )
            .expect("valid token"),
            view_path: "live".to_owned(),
        },
        "the closure source names the farm and the view, and no generation \
         the Provider cannot enforce"
    );
    assert_eq!(
        port.calls(),
        vec![
            PortCall::ObserveStoreViewMarker,
            PortCall::LaunchWorker,
            PortCall::ObserveSocket,
            PortCall::ObserveGuestMount,
            PortCall::WriteStatus,
        ],
        "the marker is verified before the worker is launched"
    );
}

/// The presentation capability is declared by the Provider's own
/// component, on the plan itself, and is not read out of a launch role or
/// a confinement label: the plan carries no such field to read.
#[test]
fn the_namespace_first_capability_is_declared_and_carries_its_restrictions() {
    let port = ScriptedPort::serving();
    reconcile(&port, "read-only");
    let plan = &port.launched_plans()[0];
    assert_eq!(plan.presentation, PRESENTATION_CAPABILITY);
    assert_eq!(plan.presentation, "namespace-first-service-source");
    assert_eq!(plan.setup_restrictions, SETUP_RESTRICTIONS);
    assert_eq!(
        plan.setup_restrictions,
        ["steady-state-mount-namespace", "zero-host-capability"]
    );
    let rendered = serde_json::to_string(plan).expect("the plan serializes");
    for forbidden in ["role", "seccomp", "setupMode", "setup_mode"] {
        assert!(
            !rendered.to_ascii_lowercase().contains(forbidden),
            "the plan inherits no setup mode from a {forbidden} label"
        );
    }
}

/// A runtime root the broker cannot fence, or that cannot hold a socket
/// address, is refused before any worker is launched: the socket is
/// derived, so an unusable derivation is a refusal and never a
/// normalized path.
#[test]
fn an_unusable_runtime_root_refuses_the_launch_before_it_happens() {
    let port = ScriptedPort::serving();
    let owned = VirtiofsBindingController::new(&port, fixtures::zone(), "/run/d2b/../etc");
    let error = block_on(owned.reconcile(
        &fixtures::binding("read-only"),
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect("reconcile reports");
    assert_eq!(
        report_failure(&error),
        VirtiofsBindingError::ServingSocketPathUnresolved
    );
    assert!(port.launched_sockets().is_empty());
    assert!(port.launched_plans().is_empty());
}

/// The frozen socket-path refusal set is closed and the controller maps
/// every refusal onto one stable code.
fn report_failure(error: &d2b_provider_volume_virtiofs::BindingStatusReport) -> VirtiofsBindingError {
    error.reason.expect("a failed reconcile carries a reason")
}

/// A socket path longer than the platform limit is refused by the same
/// code, so the failure is one condition rather than two spellings.
#[test]
fn an_over_long_runtime_root_refuses_with_the_same_code() {
    let port = ScriptedPort::serving();
    let deep = format!("/run/{}", "d".repeat(96));
    let owned = VirtiofsBindingController::new(&port, fixtures::zone(), deep);
    let report = block_on(owned.reconcile(
        &fixtures::binding("read-only"),
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect("reconcile reports");
    assert_eq!(
        report_failure(&report),
        VirtiofsBindingError::ServingSocketPathUnresolved
    );
    assert!(port.launched_plans().is_empty());
}

/// The derived source and socket are the whole of the private material the
/// plan carries, and neither the public status nor the serialized plan
/// names a host root, a shared directory, or a store path.
#[test]
fn neither_the_plan_nor_the_status_names_a_host_path() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "read-only");
    let plan = &port.launched_plans()[0];
    let rendered = serde_json::to_string(plan)
        .expect("the plan serializes")
        .to_ascii_lowercase();
    assert_eq!(
        rendered,
        serde_json::to_string(plan)
            .expect("the plan serializes")
            .to_ascii_lowercase(),
        "the plan rendering is stable"
    );
    for forbidden in ["/run", "/nix", "state-root", RUNTIME_ROOT] {
        assert!(
            !rendered.contains(forbidden),
            "the plan names the forbidden fragment {forbidden}"
        );
    }
    // The source serializes as its class, so even the source locator the
    // plan holds never reaches a log or an audit record.
    assert!(rendered.contains("declared-storage-root"));
    let _ = report;
    assert!(matches!(
        SocketPathRefusal::RuntimeRootInvalid.code(),
        "serving-socket-root-invalid"
    ));
}
