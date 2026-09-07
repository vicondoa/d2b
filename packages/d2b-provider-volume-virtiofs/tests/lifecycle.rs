//! Hermetic VolumeBinding lifecycle, sandbox, and privacy conformance.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    ResourceUid,
    volume_binding::{VolumeBindingReadinessFence, VolumeBindingStatusResource},
};
use d2b_provider_volume_virtiofs::testing::{PortCall, ScriptedPort, block_on, fixtures};
use d2b_provider_volume_virtiofs::{
    LaunchedWorker, StoredBinding, VOLUME_BINDING_FINALIZER, VOLUME_BINDING_RESOURCE_TYPE,
    VirtiofsBindingController, VirtiofsBindingEffectPort, VirtiofsBindingError, VirtiofsdWorkerPlan,
    virtiofs_runner_contract,
};

use d2b_provider_volume_virtiofs::BindingPhase;

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
        plan: &VirtiofsdWorkerPlan,
    ) -> Result<LaunchedWorker, VirtiofsBindingError> {
        (&self.inner).launch_worker(binding, plan).await
    }

    async fn observe_socket(&self, worker: &LaunchedWorker) -> Result<bool, VirtiofsBindingError> {
        (&self.inner).observe_socket(worker).await
    }

    async fn observe_guest_mount(
        &self,
        binding: &StoredBinding,
    ) -> Result<bool, VirtiofsBindingError> {
        (&self.inner).observe_guest_mount(binding).await
    }

    async fn delete_worker(&self, worker: &LaunchedWorker) -> Result<(), VirtiofsBindingError> {
        (&self.inner).delete_worker(worker).await
    }
}

fn reconcile(
    port: &ScriptedPort,
    access: &str,
) -> d2b_provider_volume_virtiofs::BindingStatusReport {
    let controller = VirtiofsBindingController::new(port);
    block_on(controller.reconcile(
        &fixtures::binding(access),
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect("reconcile reports")
}

#[test]
fn the_default_marker_and_status_probes_fail_closed() {
    let port = DefaultProbePort {
        inner: ScriptedPort::serving(),
    };
    let controller = VirtiofsBindingController::new(&port);
    // The default status write fails closed, so the reconcile itself
    // fails closed instead of reporting readiness without a validated
    // projection (KTD3).
    let error = block_on(controller.reconcile(
        &fixtures::binding("read-only"),
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect_err("default write rejected");
    assert_eq!(error, VirtiofsBindingError::UnauthorizedWriter);
    assert!(!port.inner.calls().contains(&PortCall::LaunchWorker));
}

#[test]
fn a_binding_reaches_ready_only_when_the_host_serves_and_the_guest_mounts() {
    let port = ScriptedPort::serving();
    let report = reconcile(&port, "read-only");
    assert_eq!(report.phase, BindingPhase::Ready);
    assert!(report.binding_ready);
    assert!(report.guest_mount_ready);
    assert!(report.reason.is_none());
    // KTD3: the fenced projection is written on every reconcile.
    assert_eq!(
        port.calls(),
        vec![
            PortCall::ObserveStoreViewMarker,
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
    assert!(!report.binding_ready);
    assert_eq!(report.reason, Some(VirtiofsBindingError::BindingNotReady));
    // The guest is never probed while the host side is not serving.
    assert!(!port.calls().contains(&PortCall::ObserveGuestMount));
    // Even a pending reconcile writes its fail-closed projection (KTD3).
    assert_eq!(port.status_writes().len(), 1);
    assert!(!port.status_writes()[0].ready);
}

#[test]
fn a_store_view_waits_for_its_zero_length_marker_before_launch() {
    let port = ScriptedPort::serving().store_view_marker_missing();
    let report = reconcile(&port, "read-only");
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
fn a_serving_host_whose_guest_does_not_mount_is_degraded() {
    let port = ScriptedPort::serving().guest_never_mounts();
    let report = reconcile(&port, "read-only");
    assert_eq!(report.phase, BindingPhase::Degraded);
    assert!(report.binding_ready);
    assert!(!report.guest_mount_ready);
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
    let controller = VirtiofsBindingController::new(&port);
    let report = block_on(controller.reconcile(
        &binding,
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect("reconcile reports");
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
    let controller = VirtiofsBindingController::new(&port);
    let error = block_on(controller.reconcile(
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
    let controller = VirtiofsBindingController::new(&port);
    let report = block_on(controller.reconcile(
        &binding,
        &fixtures::store_view_volume(),
        4,
        fixtures::principal(),
    ))
    .expect("reconcile reports");
    let worker = LaunchedWorker {
        process_ref: report.worker_process_ref.expect("worker exists"),
        socket: report.socket.expect("socket exists"),
    };
    assert!(block_on(controller.drain(&binding, &worker)).is_ok());
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
    let controller = VirtiofsBindingController::new(&port);
    let worker = LaunchedWorker {
        process_ref: ResourceRef::parse("Process/vol-work-state-virtiofsd-work-vm")
            .expect("valid ref"),
        socket: binding.socket_identity(&fixtures::zone()),
    };
    assert_eq!(
        block_on(controller.drain(&binding, &worker)).unwrap_err(),
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
    let controller = VirtiofsBindingController::new(&port);
    assert_eq!(controller.finalizer(), VOLUME_BINDING_FINALIZER);
    assert_eq!(controller.provider().as_str(), "volume-virtiofs");
    let contract = virtiofs_runner_contract();
    assert_eq!(contract.resource_type, VOLUME_BINDING_RESOURCE_TYPE);
    assert_eq!(contract.finalizer, VOLUME_BINDING_FINALIZER);
    assert!(contract.watched_configuration_is_dependency);
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
    assert!(StoredBinding::from_resource_spec(&foreign_owner).is_err());

    let mut tuned = fixtures::binding_envelope("read-only", "work-vm", "ro-store");
    tuned["spec"]["threadPoolSize"] = serde_json::json!(2);
    assert!(StoredBinding::from_resource_spec(&tuned).is_err());
}
