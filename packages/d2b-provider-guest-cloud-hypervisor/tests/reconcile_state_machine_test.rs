use std::{
    collections::VecDeque,
    sync::Arc,
};

use tokio::sync::Mutex;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    BindingKind, BindingLifecycleState, BindingObservation, BindingSlot, BoundedToken, BudgetSpec,
    CanonicalJsonObject, ChildBindingRequest, CompletionCondition, DesiredLifecycle,
    DeviceAttachment, ExecutionDomain, ExecutionPolicy, NetworkAttachment, ReleaseOutcome,
    RequestedRights, ResourceGeneration, ResourcePhase, ResourceRef, ResourceUid,
    VolumeBindingRequest, VolumePresentation, ZoneId, ZoneRevision, volume::AttachmentAccess,
};
use d2b_provider_guest_cloud_hypervisor::{
    AdmittedGuestGraph, BindingAdoptionFence, BindingAdoptionStatus, BootstrapGraph, ChildRole,
    ChildSpecUpdate, CloudHypervisorController, CloudHypervisorError, CloudHypervisorResourceApi,
    CloudHypervisorResourceApiError, CommittedChild, GuestChildCommitResponse,
    GuestChildCreateBatch, GuestCondition, GuestConsumerCompletion, GuestDependencySnapshot,
    GuestFinalizationInput, GuestGenerationSet, GuestSnapshot, GuestStartGate,
    GuestStatusProjection, ObservedBindingRow, OwnedChildSnapshot, ProcessAdoptionStatus,
    ProcessState, SessionState, UpgradeReason, classify_binding_adoption,
    classify_guest_execution_parent,
};

mod common;

use common::{GUEST_UID, ZONE_UID, config, descriptor};

fn graph() -> BootstrapGraph {
    BootstrapGraph::new(
        vec![ResourceRef::parse("Device/kvm").unwrap()],
        vec![ResourceRef::parse("Network/work").unwrap()],
        vec![ResourceRef::parse("Volume/store").unwrap()],
        vec![ResourceRef::parse("VolumeBinding/store-share").unwrap()],
        vec![],
    )
    .unwrap()
}

/// The one `VolumeBinding` request this Guest consumes as itself (AE32).
fn guest_storage_request(name: &str, view: &str) -> VolumeBindingRequest {
    VolumeBindingRequest::new(
        ResourceRef::parse("Volume/store").unwrap(),
        ResourceRef::parse(&format!("Guest/{name}")).unwrap(),
        BindingSlot::parse("root").unwrap(),
        BoundedToken::parse(view).unwrap(),
        AttachmentAccess::ReadOnly,
        VolumePresentation::filesystem("/var/lib/d2b/store").unwrap(),
    )
    .unwrap()
}

fn observed(
    state: BindingLifecycleState,
    prepare: CompletionCondition,
    consumer: CompletionCondition,
    release: ReleaseOutcome,
) -> BindingObservation {
    BindingObservation::new(state, prepare, consumer, release)
}

/// The admitted graph for one Guest, with every relationship reported
/// `Pending` until an observation is supplied.
fn admitted_graph(name: &str, observations: &[BindingObservation]) -> AdmittedGuestGraph {
    let guest_ref = ResourceRef::parse(&format!("Guest/{name}")).unwrap();
    let parent = classify_guest_execution_parent(&ExecutionPolicy::system_default())
        .unwrap()
        .with_parent_use(&guest_ref, guest_storage_request(name, "store"))
        .unwrap();
    let mut graph = AdmittedGuestGraph::from_execution_parent(guest_ref, &parent).unwrap();
    let request = guest_storage_request(name, "store");
    let source_uid = ResourceUid::parse(GUEST_UID).unwrap();
    for observation in observations {
        graph
            .observe_binding(&request, &source_uid, *observation)
            .expect("the Guest consumes this relationship");
    }
    graph
}

/// A fragment whose only attachment inputs are a Device and a Network a
/// Guest declares for the workloads it runs (AE31).
fn ceiling_policy() -> ExecutionPolicy {
    ExecutionPolicy::new(
        ExecutionDomain::System,
        vec![ExecutionDomain::System],
        None,
        BudgetSpec::default(),
        vec![NetworkAttachment::new(ResourceRef::parse("Network/work").unwrap(), false).unwrap()],
        vec![DeviceAttachment::new(ResourceRef::parse("Device/kvm").unwrap(), true).unwrap()],
        Vec::new(),
    )
    .unwrap()
}

/// An admitted graph whose only classified input is the child's support
/// ceiling: a Guest that declares a Device and a Network for the workloads
/// it runs and consumes nothing of its own (AE31).
fn ceiling_only_graph(name: &str) -> AdmittedGuestGraph {
    let guest_ref = ResourceRef::parse(&format!("Guest/{name}")).unwrap();
    AdmittedGuestGraph::from_execution_parent(
        guest_ref,
        &classify_guest_execution_parent(&ceiling_policy()).unwrap(),
    )
    .unwrap()
}

fn make_admitted_controller(
    api: FakeApi,
    admitted: AdmittedGuestGraph,
) -> CloudHypervisorController<FakeApi> {
    make_controller(api).with_admitted_graph(admitted)
}

fn guest(name: &str, zone: &str, uid: &str, zone_uid: &str) -> GuestSnapshot {
    let resource_ref = format!("Guest/{name}");
    GuestSnapshot::new(
        ZoneId::parse(zone).unwrap(),
        ResourceUid::parse(zone_uid).unwrap(),
        ResourceRef::parse(&resource_ref).unwrap(),
        ResourceUid::parse(uid).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(7),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        Some("guest-system".to_owned()),
        GuestGenerationSet::all(1),
        false,
    )
    .unwrap()
}

fn deleting_guest(name: &str, zone: &str, uid: &str, zone_uid: &str) -> GuestSnapshot {
    let resource_ref = format!("Guest/{name}");
    GuestSnapshot::new(
        ZoneId::parse(zone).unwrap(),
        ResourceUid::parse(zone_uid).unwrap(),
        ResourceRef::parse(&resource_ref).unwrap(),
        ResourceUid::parse(uid).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(7),
        ResourceRef::parse("Host/host-system").unwrap(),
        ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
        Some("guest-system".to_owned()),
        GuestGenerationSet::all(1),
        true,
    )
    .unwrap()
}

fn finalization_input(
    guest: &GuestSnapshot,
    children: &[OwnedChildSnapshot],
    session: SessionState,
    guest_local_drained: bool,
    process: ProcessState,
) -> GuestFinalizationInput {
    let fences = children
        .iter()
        .map(|child| {
            let role =
                d2b_provider_guest_cloud_hypervisor::child_role_for_ref(child.resource_ref())
                    .unwrap();
            d2b_provider_guest_cloud_hypervisor::FencedChild::new(
                role,
                child.resource_ref().clone(),
                child.uid().clone(),
                child.revision(),
            )
            .unwrap()
        })
        .collect();
    GuestFinalizationInput::new(
        guest.uid().clone(),
        session,
        guest_local_drained,
        process,
        fences,
        false,
        false,
        false,
    )
    .unwrap()
}

fn dependencies(
    devices_ready: bool,
    networks_ready: bool,
    volumes_ready: bool,
    bindings_ready: bool,
    setup_ready: bool,
) -> GuestDependencySnapshot {
    GuestDependencySnapshot::new(
        vec![(
            ResourceRef::parse("Device/kvm").unwrap(),
            if devices_ready {
                ResourcePhase::Ready
            } else {
                ResourcePhase::Pending
            },
        )],
        vec![(
            ResourceRef::parse("Network/work").unwrap(),
            if networks_ready {
                ResourcePhase::Ready
            } else {
                ResourcePhase::Pending
            },
        )],
        vec![(
            ResourceRef::parse("Volume/store").unwrap(),
            if volumes_ready {
                ResourcePhase::Ready
            } else {
                ResourcePhase::Pending
            },
        )],
        vec![(
            ResourceRef::parse("VolumeBinding/store-share").unwrap(),
            bindings_ready,
        )],
        setup_ready,
    )
    .unwrap()
}

fn committed_children(batch: &GuestChildCreateBatch) -> Vec<CommittedChild> {
    batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            CommittedChild::new(
                mutation.target().clone(),
                mutation.owner_ref().clone(),
                mutation.zone().clone(),
                ResourceUid::parse(format!("323e4567-e89b-42d3-a456-42661417{index:04}")).unwrap(),
                ZoneRevision::new(2),
            )
            .unwrap()
        })
        .collect()
}

fn matching_children(
    guest: &GuestSnapshot,
    batch: &GuestChildCreateBatch,
) -> Vec<OwnedChildSnapshot> {
    batch
        .mutations()
        .iter()
        .enumerate()
        .map(|(index, mutation)| {
            OwnedChildSnapshot::new(
                mutation.target().clone(),
                guest.zone().clone(),
                guest.resource_ref().clone(),
                ResourceUid::parse(format!("323e4567-e89b-42d3-a456-42661417{index:04}")).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ZoneRevision::new(2),
                batch.desired_digest(mutation.target()).unwrap(),
                ResourcePhase::Ready,
                if mutation.target().resource_type().as_str() == "Process" {
                    Some(DesiredLifecycle::Running)
                } else {
                    None
                },
                true,
            )
            .unwrap()
            .with_owner_uid(guest.uid().clone())
        })
        .collect()
}

#[derive(Default)]
struct ApiState {
    guest: Option<GuestSnapshot>,
    children: Vec<OwnedChildSnapshot>,
    dependencies: Option<GuestDependencySnapshot>,
    commits: Vec<GuestChildCreateBatch>,
    commit_responses: VecDeque<GuestChildCommitResponse>,
    updates: Vec<ChildSpecUpdate>,
    update_results: VecDeque<Result<CommittedChild, CloudHypervisorResourceApiError>>,
    statuses: Vec<GuestStatusProjection>,
    process_observation: Option<ProcessAdoptionStatus>,
    finalization: Option<GuestFinalizationInput>,
    upgrade_reason: Option<UpgradeReason>,
    lifecycle_events: Vec<String>,
    get_calls: usize,
    relist_calls: usize,
}

#[derive(Clone)]
struct FakeApi {
    state: Arc<Mutex<ApiState>>,
}

impl FakeApi {
    fn new(guest: GuestSnapshot, dependencies: GuestDependencySnapshot) -> Self {
        Self {
            state: Arc::new(Mutex::new(ApiState {
                guest: Some(guest),
                dependencies: Some(dependencies),
                ..ApiState::default()
            })),
        }
    }
}

#[async_trait]
impl CloudHypervisorResourceApi for FakeApi {
    async fn register(
        &self,
        _: &d2b_provider_guest_cloud_hypervisor::CloudHypervisorControllerRegistration,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        Ok(())
    }

    async fn get_guest(
        &self,
        _: &ResourceRef,
    ) -> Result<GuestSnapshot, CloudHypervisorResourceApiError> {
        let mut state = self.state.lock().await;
        state.get_calls += 1;
        state
            .guest
            .clone()
            .ok_or(CloudHypervisorResourceApiError::NotFound)
    }

    async fn relist_owned_children(
        &self,
        _: &GuestSnapshot,
        _: &[ResourceRef],
    ) -> Result<Vec<OwnedChildSnapshot>, CloudHypervisorResourceApiError> {
        let mut state = self.state.lock().await;
        state.relist_calls += 1;
        Ok(state.children.clone())
    }

    async fn observe_dependencies(
        &self,
        _: &GuestSnapshot,
        _: &BootstrapGraph,
    ) -> Result<GuestDependencySnapshot, CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .dependencies
            .clone()
            .ok_or(CloudHypervisorResourceApiError::NotFound)
    }

    async fn commit_batch(
        &self,
        batch: GuestChildCreateBatch,
    ) -> Result<GuestChildCommitResponse, CloudHypervisorResourceApiError> {
        let mut state = self.state.lock().await;
        let response = state
            .commit_responses
            .pop_front()
            .unwrap_or_else(|| GuestChildCommitResponse::Committed(committed_children(&batch)));
        state.commits.push(batch);
        Ok(response)
    }

    async fn update_spec(
        &self,
        update: ChildSpecUpdate,
    ) -> Result<CommittedChild, CloudHypervisorResourceApiError> {
        let mut state = self.state.lock().await;
        let result = state.update_results.pop_front();
        state.updates.push(update.clone());
        result.unwrap_or_else(|| {
            Ok(CommittedChild::new(
                update.target().clone(),
                ResourceRef::parse("Guest/gateway").unwrap(),
                ZoneId::parse("work").unwrap(),
                update.expected_uid().clone(),
                ZoneRevision::new(update.expected_revision().get().saturating_add(1)),
            )
            .unwrap())
        })
    }

    async fn update_status(
        &self,
        _: &GuestSnapshot,
        status: GuestStatusProjection,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state.lock().await.statuses.push(status);
        Ok(())
    }

    async fn observe_process_adoption(
        &self,
        _: &GuestSnapshot,
        _: &OwnedChildSnapshot,
    ) -> Result<ProcessAdoptionStatus, CloudHypervisorResourceApiError> {
        Ok(self
            .state
            .lock()
            .await
            .process_observation
            .unwrap_or(ProcessAdoptionStatus::Current))
    }

    async fn assess_update(
        &self,
        _: &GuestSnapshot,
    ) -> Result<Option<UpgradeReason>, CloudHypervisorResourceApiError> {
        Ok(self.state.lock().await.upgrade_reason)
    }

    async fn observe_finalization(
        &self,
        _: &GuestSnapshot,
        _: &[OwnedChildSnapshot],
    ) -> Result<GuestFinalizationInput, CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .finalization
            .clone()
            .ok_or(CloudHypervisorResourceApiError::InvalidResponse)
    }

    async fn drain_guest_local(
        &self,
        _: &GuestSnapshot,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .lifecycle_events
            .push("drain-guest-local".to_owned());
        Ok(())
    }

    async fn close_guest_session(
        &self,
        _: &GuestSnapshot,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .lifecycle_events
            .push("close-session".to_owned());
        Ok(())
    }

    async fn delete_child(
        &self,
        _: &GuestSnapshot,
        child: d2b_provider_guest_cloud_hypervisor::FencedChild,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .lifecycle_events
            .push(format!("delete-{}", child.role().suffix()));
        Ok(())
    }

    async fn clear_guest_finalizer(
        &self,
        _: &GuestSnapshot,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .lifecycle_events
            .push("clear-finalizer".to_owned());
        Ok(())
    }

    async fn invalidate_guest_session(
        &self,
        _: &GuestSnapshot,
        minimum_generation: u64,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        self.state
            .lock()
            .await
            .lifecycle_events
            .push(format!("invalidate-session-{minimum_generation}"));
        Ok(())
    }
}

fn make_controller(api: FakeApi) -> CloudHypervisorController<FakeApi> {
    CloudHypervisorController::from_verified_descriptor(
        config(),
        graph(),
        descriptor(),
        Arc::new(api),
    )
    .unwrap()
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn dependency_gate_keeps_process_stopped_until_every_dependency_is_ready() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, false, false));
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api);
    controller.register().await.unwrap();

    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(outcome.is_pending());

    let state = state.lock().await;
    assert_eq!(state.commits.len(), 1);
    assert!(state.updates.is_empty());
    let process = state.commits[0]
        .mutations()
        .iter()
        .find(|mutation| mutation.target().resource_type().as_str() == "Process")
        .unwrap();
    let process_payload = state.commits[0]
        .canonical_payload(process.target())
        .unwrap();
    let process_payload: serde_json::Value = serde_json::from_slice(&process_payload).unwrap();
    assert_eq!(process_payload["spec"]["desiredLifecycle"], "stopped");
    assert!(state.statuses.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn non_ready_binding_keeps_the_gate_closed_and_reports_the_binding_condition() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, false, true));
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();

    // First reconcile mints the deterministic children; the VMM is created
    // stopped because the binding is not current.
    controller.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    // Second reconcile with every child present but the binding not Ready
    // under a current fence: the VMM is driven back to stopped and the
    // renamed binding condition appears in status.
    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(outcome.status().has_condition(
        d2b_provider_guest_cloud_hypervisor::GuestCondition::BindingDependencyNotReady
    ));

    let state = state.lock().await;
    assert_eq!(state.commits.len(), 1);
    let process_target = batch
        .mutations()
        .iter()
        .map(|mutation| mutation.target().clone())
        .find(|target| target.resource_type().as_str() == "Process")
        .unwrap();
    let stop = state
        .updates
        .iter()
        .find(|update| *update.target() == process_target)
        .expect("VMM stop update");
    assert_eq!(stop.desired_lifecycle(), Some(DesiredLifecycle::Stopped));
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn current_binding_readiness_leaves_the_vmm_running() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();

    controller.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    // With the binding Ready under its current fence the VMM stays running
    // and no binding condition is reported.
    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(!outcome.status().has_condition(
        d2b_provider_guest_cloud_hypervisor::GuestCondition::BindingDependencyNotReady
    ));
    assert!(!outcome
        .status()
        .has_condition(d2b_provider_guest_cloud_hypervisor::GuestCondition::ProcessStopped));
    assert!(state.lock().await.updates.is_empty());
}

/// AE6 and AE21: a Guest whose storage export is prepared but not yet
/// mounted starts, and the outstanding mount is reported afterwards. Waiting
/// for the consumer's own completion before permitting the start would make
/// the start depend on the thing the start enables.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn prepared_storage_permits_boot_and_mount_completion_follows() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, false, true));
    let state = Arc::clone(&api.state);
    let admitted = admitted_graph(
        "gateway",
        &[observed(
            BindingLifecycleState::Active,
            CompletionCondition::Complete,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        )],
    );
    assert_eq!(admitted.start_gate(), GuestStartGate::Permitted);
    assert_eq!(
        admitted.consumer_completion(),
        GuestConsumerCompletion::Incomplete
    );
    let mut controller = make_admitted_controller(api.clone(), admitted);
    controller.register().await.unwrap();

    controller.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(
        !outcome
            .status()
            .has_condition(GuestCondition::AdmittedBindingSourceNotPrepared),
        "a prepared source clears the pre-start condition"
    );
    assert!(
        outcome
            .status()
            .has_condition(GuestCondition::AdmittedBindingConsumerIncomplete),
        "the outstanding mount is reported rather than waited on"
    );
    assert!(
        !outcome.status().has_condition(GuestCondition::ProcessStopped),
        "the VMM runs on prepared storage"
    );
    assert!(state.lock().await.updates.is_empty());
}

/// The other half of AE21: an unprepared source still holds the VMM stopped,
/// and the consumer side is reported as not yet observable rather than as a
/// failed mount. The two conditions stay apart instead of folding into one
/// readiness both sides wait on.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unprepared_source_keeps_the_vmm_stopped() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let state = Arc::clone(&api.state);
    let admitted = admitted_graph(
        "gateway",
        &[observed(
            BindingLifecycleState::Admitted,
            CompletionCondition::Pending,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        )],
    );
    assert_eq!(admitted.start_gate(), GuestStartGate::SourcePending);
    assert_eq!(
        admitted.consumer_completion(),
        GuestConsumerCompletion::NotYetRunning
    );
    let mut controller = make_admitted_controller(api.clone(), admitted);
    controller.register().await.unwrap();

    controller.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(
        outcome
            .status()
            .has_condition(GuestCondition::AdmittedBindingSourceNotPrepared)
    );
    let process_target = batch
        .mutations()
        .iter()
        .map(|mutation| mutation.target().clone())
        .find(|target| target.resource_type().as_str() == "Process")
        .unwrap();
    let state = state.lock().await;
    let stop = state
        .updates
        .iter()
        .find(|update| *update.target() == process_target)
        .expect("VMM stop update");
    assert_eq!(stop.desired_lifecycle(), Some(DesiredLifecycle::Stopped));
}

/// Scenario 2: a Device and a Network a Guest declared for the workloads it
/// runs are a child support ceiling. They allocate the Guest no access, so
/// the flattened families reporting them not Ready no longer hold its boot,
/// even though they keep reporting their own condition.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_support_ceiling_creates_no_binding_and_holds_no_boot() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(false, false, true, true, true));
    let state = Arc::clone(&api.state);
    let admitted = ceiling_only_graph("gateway");
    assert!(
        admitted.guest_bindings().is_empty(),
        "a support ceiling creates no Guest relationship"
    );
    assert!(
        admitted
            .support_ceiling()
            .admits(BindingKind::Device, RequestedRights::Exclusive)
    );
    let mut controller = make_admitted_controller(api.clone(), admitted);
    controller.register().await.unwrap();

    controller.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(
        !outcome.status().has_condition(GuestCondition::ProcessStopped),
        "a ceiling is an admission constraint, not a boot dependency"
    );
    assert!(
        outcome
            .status()
            .has_condition(GuestCondition::DeviceDependencyNotReady)
            && outcome
                .status()
                .has_condition(GuestCondition::NetworkDependencyNotReady),
        "the cutover keeps the old families' own verdicts observable"
    );
    assert!(state.lock().await.updates.is_empty());
}

/// AE32 and AE33: the Guest's own consumption is one relationship whose
/// consumer is the Guest, while a child's request default shapes that
/// child's draft and never becomes one.
#[test]
fn guest_consumption_is_a_guest_binding_and_child_defaults_are_not() {
    let default_entry = CanonicalJsonObject::parse(
        &serde_json::to_vec(&serde_json::json!({
            "consumerRef": "Process/gateway-vmm",
            "volumeRef": "Volume/store",
            "view": "store",
        }))
        .unwrap(),
    )
    .unwrap();
    let policy = ExecutionPolicy::new(
        ExecutionDomain::System,
        vec![ExecutionDomain::System],
        None,
        BudgetSpec::default(),
        Vec::new(),
        Vec::new(),
        vec![default_entry],
    )
    .unwrap();
    let guest_ref = ResourceRef::parse("Guest/gateway").unwrap();
    let parent = classify_guest_execution_parent(&policy)
        .unwrap()
        .with_parent_use(&guest_ref, guest_storage_request("gateway", "store"))
        .unwrap();
    let graph = AdmittedGuestGraph::from_execution_parent(guest_ref.clone(), &parent).unwrap();

    assert_eq!(graph.guest_bindings().len(), 1);
    assert_eq!(
        graph.guest_bindings()[0].request().consumer_ref(),
        &guest_ref,
        "the only Guest relationship is the one whose consumer is the Guest"
    );
    assert_eq!(graph.child_defaults().len(), 1);

    let draft = ChildBindingRequest::new(
        ResourceRef::parse("Process/gateway-vmm").unwrap(),
        BindingKind::Volume,
    )
    .unwrap();
    assert!(
        graph.shape_child_request(&draft).is_err(),
        "a Volume default is outside a ceiling that declares only Device and Network"
    );
    assert_eq!(
        graph.guest_bindings().len(),
        1,
        "shaping a child's request adds no Guest relationship"
    );
}

/// Scenario 3: a restart adopts binding evidence only when the row still
/// names the same source and the same consumer under their store-assigned
/// identities. A cached readiness under any other fence is refused.
#[test]
fn restart_adopts_binding_evidence_only_under_its_own_fence() {
    let guest_ref = ResourceRef::parse("Guest/gateway").unwrap();
    let source_ref = ResourceRef::parse("Volume/store").unwrap();
    let source_uid = ResourceUid::parse(ZONE_UID).unwrap();
    let view = BoundedToken::parse("store").unwrap();
    let consumer_uid = ResourceUid::parse(GUEST_UID).unwrap();
    let fence = BindingAdoptionFence::new(
        source_ref.clone(),
        source_uid.clone(),
        view.clone(),
        guest_ref.clone(),
        consumer_uid.clone(),
    );
    let row = ObservedBindingRow {
        source_ref: &source_ref,
        source_uid: &source_uid,
        view: &view,
        consumer_ref: &guest_ref,
        consumer_uid: &consumer_uid,
    };
    assert_eq!(
        classify_binding_adoption(&fence, Some(row)),
        BindingAdoptionStatus::Adopted
    );
    assert_eq!(
        classify_binding_adoption(&fence, None),
        BindingAdoptionStatus::Absent
    );

    let reassigned_uid = ResourceUid::parse("523e4567-e89b-42d3-a456-426614174000").unwrap();
    assert_eq!(
        classify_binding_adoption(
            &fence,
            Some(ObservedBindingRow {
                consumer_uid: &reassigned_uid,
                ..row
            }),
        ),
        BindingAdoptionStatus::Refused,
        "a reassigned Guest makes the row another relationship's evidence"
    );
    let replaced_source_uid = ResourceUid::parse("623e4567-e89b-42d3-a456-426614174000").unwrap();
    assert_eq!(
        classify_binding_adoption(
            &fence,
            Some(ObservedBindingRow {
                source_uid: &replaced_source_uid,
                ..row
            }),
        ),
        BindingAdoptionStatus::Refused,
        "a replaced source makes the row another relationship's evidence"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn uncertain_batch_relist_does_not_create_a_duplicate_incarnation() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    {
        api.state
            .lock()
            .await
            .commit_responses
            .push_back(GuestChildCommitResponse::Uncertain);
    }
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();
    assert!(
        controller
            .reconcile(guest.resource_ref())
            .await
            .unwrap()
            .is_pending()
    );

    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);
    let mut restarted = make_controller(api);
    restarted.register().await.unwrap();
    restarted.reconcile(guest.resource_ref()).await.unwrap();

    let state = state.lock().await;
    assert_eq!(state.commits.len(), 1);
    assert!(state.relist_calls >= 2);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn truncated_batch_response_stays_pending_without_update_spec() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    api.state
        .lock()
        .await
        .commit_responses
        .push_back(GuestChildCommitResponse::Truncated);
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();

    assert!(
        controller
            .reconcile(guest.resource_ref())
            .await
            .unwrap()
            .is_pending()
    );
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);
    let mut restarted = make_controller(api);
    restarted.register().await.unwrap();
    restarted.reconcile(guest.resource_ref()).await.unwrap();

    let state = state.lock().await;
    assert_eq!(state.commits.len(), 1);
    assert!(state.updates.is_empty());
    assert!(state.relist_calls >= 2);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn child_uid_or_revision_conflict_is_retryable_and_relists_before_replacement_update() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let batch = {
        let verified = descriptor();
        BootstrapGraph::plan_children(
            guest.zone().clone(),
            guest.resource_ref().clone(),
            guest.execution_ref().clone(),
            &verified,
        )
        .unwrap()
        .child_batch()
        .clone()
    };
    let process = batch
        .mutations()
        .iter()
        .find(|mutation| mutation.target().resource_type().as_str() == "Process")
        .unwrap();
    let process_child = OwnedChildSnapshot::new(
        process.target().clone(),
        guest.zone().clone(),
        guest.resource_ref().clone(),
        ResourceUid::parse("323e4567-e89b-42d3-a456-426614170099").unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(3),
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned(),
        ResourcePhase::Ready,
        Some(DesiredLifecycle::Stopped),
        true,
    )
    .unwrap()
    .with_owner_uid(guest.uid().clone());
    {
        let mut children = matching_children(
            &guest,
            &GuestChildCreateBatch::new(
                &guest,
                &batch,
                batch
                    .mutations()
                    .iter()
                    .map(|mutation| mutation.target().clone()),
            )
            .unwrap(),
        );
        let process_index = children
            .iter()
            .position(|child| child.resource_ref() == process.target())
            .unwrap();
        children[process_index] = process_child;
        let mut state = api.state.lock().await;
        state.children = children;
        state
            .update_results
            .push_back(Err(CloudHypervisorResourceApiError::Conflict));
    }
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();
    assert!(
        controller
            .reconcile(guest.resource_ref())
            .await
            .unwrap()
            .is_pending()
    );

    let create_batch = GuestChildCreateBatch::new(
        &guest,
        &batch,
        batch
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone()),
    )
    .unwrap();
    state.lock().await.children = matching_children(&guest, &create_batch);
    let mut replacement = make_controller(api);
    replacement.register().await.unwrap();
    replacement.reconcile(guest.resource_ref()).await.unwrap();

    let state = state.lock().await;
    assert_eq!(state.relist_calls, 2);
    assert_eq!(state.updates.len(), 1);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn restart_converges_from_resource_state_without_direct_effects() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let state = Arc::clone(&api.state);
    let mut first = make_controller(api.clone());
    first.register().await.unwrap();
    first.reconcile(guest.resource_ref()).await.unwrap();
    let batch = state.lock().await.commits[0].clone();
    state.lock().await.children = matching_children(&guest, &batch);

    let mut restarted = make_controller(api);
    restarted.register().await.unwrap();
    let outcome = restarted.reconcile(guest.resource_ref()).await.unwrap();

    let state = state.lock().await;
    assert_eq!(state.commits.len(), 1);
    assert!(state.updates.is_empty());
    assert!(outcome.is_pending(), "unexpected restart outcome: {outcome:?}");
    assert!(outcome.status().has_condition(
        d2b_provider_guest_cloud_hypervisor::GuestCondition::SessionNotReady
    ));
    assert!(state.get_calls >= 2);
    assert!(state.relist_calls >= 2);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn foreign_child_owner_fails_closed_before_any_mutation() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    api.state.lock().await.children = vec![
        OwnedChildSnapshot::new(
            ResourceRef::parse("Process/gateway-vmm").unwrap(),
            guest.zone().clone(),
            ResourceRef::parse("Guest/other").unwrap(),
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614170099").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(2),
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned(),
            ResourcePhase::Ready,
            Some(DesiredLifecycle::Stopped),
            true,
        )
        .unwrap()
        .with_owner_uid(guest.uid().clone()),
    ];
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api);
    controller.register().await.unwrap();
    assert_eq!(
        controller
            .reconcile(guest.resource_ref())
            .await
            .unwrap_err(),
        CloudHypervisorError::ChildConflict
    );
    let state = state.lock().await;
    assert!(state.commits.is_empty());
    assert!(state.updates.is_empty());
    assert!(state.statuses.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn restart_adopts_only_the_exact_process_identity() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let batch = {
        let plan = BootstrapGraph::plan_children(
            guest.zone().clone(),
            guest.resource_ref().clone(),
            guest.execution_ref().clone(),
            &descriptor(),
        )
        .unwrap();
        GuestChildCreateBatch::new(
            &guest,
            plan.child_batch(),
            plan.child_batch()
                .mutations()
                .iter()
                .map(|mutation| mutation.target().clone()),
        )
        .unwrap()
    };
    {
        let mut state = api.state.lock().await;
        state.children = matching_children(&guest, &batch)
            .into_iter()
            .map(|child| {
                if child.resource_ref().resource_type().as_str() != "Process" {
                    return child;
                }
                OwnedChildSnapshot::new(
                    child.resource_ref().clone(),
                    child.zone().clone(),
                    child.owner_ref().clone(),
                    child.uid().clone(),
                    child.generation(),
                    child.revision(),
                    child.spec_digest(),
                    ResourcePhase::Pending,
                    child.desired_lifecycle(),
                    child.healthy(),
                )
                .unwrap()
                .with_owner_uid(guest.uid().clone())
            })
            .collect();
        state.process_observation = Some(ProcessAdoptionStatus::Adopted);
    }
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api);
    controller.register().await.unwrap();
    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();

    assert!(
        !outcome.status().has_condition(
            d2b_provider_guest_cloud_hypervisor::GuestCondition::AdoptionAmbiguous
        )
    );
    let state = state.lock().await;
    assert!(state.commits.is_empty());
    assert!(state.updates.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn matching_process_resource_avoids_direct_adoption_effects() {
    for observation in [
        ProcessAdoptionStatus::Quarantined,
        ProcessAdoptionStatus::Unavailable,
    ] {
        let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
        let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
        let batch = {
            let plan = BootstrapGraph::plan_children(
                guest.zone().clone(),
                guest.resource_ref().clone(),
                guest.execution_ref().clone(),
                &descriptor(),
            )
            .unwrap();
            GuestChildCreateBatch::new(
                &guest,
                plan.child_batch(),
                plan.child_batch()
                    .mutations()
                    .iter()
                    .map(|mutation| mutation.target().clone()),
            )
            .unwrap()
        };
        let state = Arc::clone(&api.state);
        {
            let mut state = state.lock().await;
            state.children = matching_children(&guest, &batch);
            state.process_observation = Some(observation);
        }
        let mut controller = make_controller(api);
        controller.register().await.unwrap();
        let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
        assert!(outcome.is_pending(), "unexpected adoption outcome: {outcome:?}");
        assert!(outcome.status().has_condition(
            d2b_provider_guest_cloud_hypervisor::GuestCondition::SessionNotReady
        ));
        assert!(!outcome.status().has_condition(
            d2b_provider_guest_cloud_hypervisor::GuestCondition::AdoptionAmbiguous
        ));
        assert!(state.lock().await.updates.is_empty());
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn vmm_exit_is_bounded_degraded_and_retries_through_process_resource() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let plan = BootstrapGraph::plan_children(
        guest.zone().clone(),
        guest.resource_ref().clone(),
        guest.execution_ref().clone(),
        &descriptor(),
    )
    .unwrap();
    let batch = GuestChildCreateBatch::new(
        &guest,
        plan.child_batch(),
        plan.child_batch()
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone()),
    )
    .unwrap();
    let mut children = matching_children(&guest, &batch);
    let process_index = children
        .iter()
        .position(|child| child.resource_ref().resource_type().as_str() == "Process")
        .unwrap();
    let process_mutation = batch
        .mutations()
        .iter()
        .find(|mutation| mutation.target().resource_type().as_str() == "Process")
        .unwrap();
    children[process_index] = OwnedChildSnapshot::new(
        process_mutation.target().clone(),
        guest.zone().clone(),
        guest.resource_ref().clone(),
        children[process_index].uid().clone(),
        ResourceGeneration::new(1).unwrap(),
        ZoneRevision::new(2),
        batch.desired_digest(process_mutation.target()).unwrap(),
        ResourcePhase::Degraded,
        Some(DesiredLifecycle::Running),
        false,
    )
    .unwrap()
    .with_owner_uid(guest.uid().clone());
    {
        let mut state = api.state.lock().await;
        state.children = children;
        state.process_observation = Some(ProcessAdoptionStatus::Absent);
    }
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api);
    controller.register().await.unwrap();
    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();

    assert!(matches!(
        outcome,
        d2b_provider_guest_cloud_hypervisor::CloudHypervisorReconcileOutcome::Degraded(_)
    ));
    assert!(
        outcome
            .status()
            .has_condition(d2b_provider_guest_cloud_hypervisor::GuestCondition::VmmProcessExited)
    );
    let state = state.lock().await;
    assert_eq!(state.updates.len(), 1);
    assert_eq!(
        state.updates[0].desired_lifecycle(),
        Some(DesiredLifecycle::Running)
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn deletion_executes_reverse_order_and_clears_finalizer_only_after_absence() {
    let guest = deleting_guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let plan = BootstrapGraph::plan_children(
        guest.zone().clone(),
        guest.resource_ref().clone(),
        guest.execution_ref().clone(),
        &descriptor(),
    )
    .unwrap();
    let batch = GuestChildCreateBatch::new(
        &guest,
        plan.child_batch(),
        plan.child_batch()
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone()),
    )
    .unwrap();
    {
        let mut state = api.state.lock().await;
        state.children = matching_children(&guest, &batch);
        state.finalization = Some(finalization_input(
            &guest,
            &state.children,
            SessionState::Active,
            false,
            ProcessState::Running {
                identity_verified: true,
            },
        ));
    }
    let state = Arc::clone(&api.state);
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();
    controller.reconcile(guest.resource_ref()).await.unwrap();
    {
        let state = state.lock().await;
        assert_eq!(state.lifecycle_events, vec!["drain-guest-local"]);
    }

    {
        let mut state = state.lock().await;
        state.finalization = Some(finalization_input(
            &guest,
            &state.children,
            SessionState::Active,
            true,
            ProcessState::Running {
                identity_verified: true,
            },
        ));
    }
    controller.reconcile(guest.resource_ref()).await.unwrap();
    {
        let state = state.lock().await;
        assert_eq!(
            state.lifecycle_events,
            vec!["drain-guest-local", "close-session"]
        );
    }

    {
        let mut state = state.lock().await;
        state.finalization = Some(finalization_input(
            &guest,
            &state.children,
            SessionState::Closed,
            true,
            ProcessState::Running {
                identity_verified: true,
            },
        ));
    }
    controller.reconcile(guest.resource_ref()).await.unwrap();
    {
        let state = state.lock().await;
        assert_eq!(
            state.updates[0].desired_lifecycle(),
            Some(DesiredLifecycle::Stopped)
        );
        assert_eq!(state.lifecycle_events.len(), 2);
    }

    for expected_role in [
        ChildRole::ChApiEndpoint,
        ChildRole::GuestControlEndpoint,
        ChildRole::VmmProcess,
        ChildRole::SystemVolume,
    ] {
        {
            let mut state = state.lock().await;
            state.finalization = Some(finalization_input(
                &guest,
                &state.children,
                SessionState::Closed,
                true,
                ProcessState::Stopped,
            ));
        }
        controller.reconcile(guest.resource_ref()).await.unwrap();
        {
            let mut state = state.lock().await;
            let event = format!("delete-{}", expected_role.suffix());
            assert_eq!(
                state.lifecycle_events.last().map(String::as_str),
                Some(event.as_str())
            );
            state.children.retain(|child| {
                d2b_provider_guest_cloud_hypervisor::child_role_for_ref(child.resource_ref())
                    != Some(expected_role)
            });
        }
    }

    state.lock().await.finalization = Some(finalization_input(
        &guest,
        &[],
        SessionState::Closed,
        true,
        ProcessState::Absent,
    ));
    controller.reconcile(guest.resource_ref()).await.unwrap();
    assert_eq!(
        state
            .lock()
            .await
            .lifecycle_events
            .last()
            .map(String::as_str),
        Some("clear-finalizer")
    );
}

/// Scenario 4: every descendant is drained first, and the Guest's own
/// admitted use is released before its finalizer clears. Retiring the Guest
/// with use outstanding would leave a relationship whose consumer no longer
/// exists.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn stop_drains_descendants_and_use_before_clearing_the_guest_finalizer() {
    let guest = deleting_guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let state = Arc::clone(&api.state);
    {
        let mut api_state = api.state.lock().await;
        api_state.children = Vec::new();
        api_state.finalization = Some(finalization_input(
            &guest,
            &[],
            SessionState::Closed,
            true,
            ProcessState::Absent,
        ));
    }
    let outstanding = admitted_graph(
        "gateway",
        &[observed(
            BindingLifecycleState::Active,
            CompletionCondition::Complete,
            CompletionCondition::Complete,
            ReleaseOutcome::Outstanding,
        )],
    );
    assert!(outstanding.use_outstanding());
    let mut controller = make_admitted_controller(api.clone(), outstanding);
    controller.register().await.unwrap();

    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(
        outcome
            .status()
            .has_condition(GuestCondition::AdmittedBindingUseOutstanding)
    );
    assert!(
        !state
            .lock()
            .await
            .lifecycle_events
            .iter()
            .any(|event| event == "clear-finalizer"),
        "the finalizer is retained while the Guest still holds admitted use"
    );

    // The same Guest with its use released clears the finalizer, so the gate
    // is the relationship's state and not a blanket refusal.
    let released = admitted_graph(
        "gateway",
        &[observed(
            BindingLifecycleState::Released,
            CompletionCondition::Complete,
            CompletionCondition::Complete,
            ReleaseOutcome::Released,
        )],
    );
    assert!(!released.use_outstanding());
    let mut released_controller = make_admitted_controller(api.clone(), released);
    released_controller.register().await.unwrap();
    released_controller
        .reconcile(guest.resource_ref())
        .await
        .unwrap();
    assert_eq!(
        state
            .lock()
            .await
            .lifecycle_events
            .last()
            .map(String::as_str),
        Some("clear-finalizer")
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn interrupted_upgrade_preserves_volume_and_fences_old_transient_uids() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let plan = BootstrapGraph::plan_children(
        guest.zone().clone(),
        guest.resource_ref().clone(),
        guest.execution_ref().clone(),
        &descriptor(),
    )
    .unwrap();
    let batch = GuestChildCreateBatch::new(
        &guest,
        plan.child_batch(),
        plan.child_batch()
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone()),
    )
    .unwrap();
    let observed = matching_children(&guest, &batch);
    let observed_map = observed
        .iter()
        .cloned()
        .map(|child| (child.resource_ref().clone(), child))
        .collect::<std::collections::BTreeMap<_, _>>();
    let state = Arc::clone(&api.state);
    state.lock().await.children = observed.clone();
    let mut controller = make_controller(api.clone());
    controller.register().await.unwrap();
    let upgrade = controller
        .plan_upgrade(
            &guest,
            &observed_map,
            d2b_provider_guest_cloud_hypervisor::UpgradeReason::ProviderGenerationChanged,
        )
        .unwrap();
    let durable_uid = upgrade.durable_volumes()[0].uid().clone();
    assert_eq!(upgrade.next_session_generation(), 1);
    {
        let mut state = state.lock().await;
        state.finalization = Some(finalization_input(
            &guest,
            &state.children,
            SessionState::Closed,
            true,
            ProcessState::Running {
                identity_verified: true,
            },
        ));
    }
    controller
        .execute_upgrade(&guest, &plan, &upgrade)
        .await
        .unwrap();

    {
        let state = state.lock().await;
        assert_eq!(upgrade.durable_volumes()[0].uid(), &durable_uid);
        assert!(
            state
                .lifecycle_events
                .contains(&"invalidate-session-1".to_owned())
        );
        assert!(
            !state
                .lifecycle_events
                .iter()
                .any(|event| event == "delete-system")
        );
        assert_eq!(
            state
                .updates
                .last()
                .and_then(ChildSpecUpdate::desired_lifecycle),
            Some(DesiredLifecycle::Stopped)
        );
    }

    {
        let mut state = state.lock().await;
        state.finalization = Some(finalization_input(
            &guest,
            &state.children,
            SessionState::Closed,
            true,
            ProcessState::Stopped,
        ));
    }
    for expected_role in [
        ChildRole::ChApiEndpoint,
        ChildRole::GuestControlEndpoint,
        ChildRole::VmmProcess,
    ] {
        {
            let mut state = state.lock().await;
            state.finalization = Some(finalization_input(
                &guest,
                &state.children,
                SessionState::Closed,
                true,
                ProcessState::Stopped,
            ));
        }
        controller
            .execute_upgrade(&guest, &plan, &upgrade)
            .await
            .unwrap();
        {
            let mut state = state.lock().await;
            let event = format!("delete-{}", expected_role.suffix());
            assert_eq!(
                state.lifecycle_events.last().map(String::as_str),
                Some(event.as_str())
            );
            state.children.retain(|child| {
                d2b_provider_guest_cloud_hypervisor::child_role_for_ref(child.resource_ref())
                    != Some(expected_role)
            });
            state.finalization = Some(finalization_input(
                &guest,
                &state.children,
                SessionState::Closed,
                true,
                ProcessState::Stopped,
            ));
        }
        controller
            .execute_upgrade(&guest, &plan, &upgrade)
            .await
            .unwrap();
    }
    state.lock().await.finalization = Some(finalization_input(
        &guest,
        &[],
        SessionState::Closed,
        true,
        ProcessState::Absent,
    ));
    controller
        .execute_upgrade(&guest, &plan, &upgrade)
        .await
        .unwrap();

    state.lock().await.children = observed.clone();
    let old_result = controller.reconcile(guest.resource_ref()).await;
    assert_eq!(old_result, Err(CloudHypervisorError::ChildConflict));

    let replacement_children = observed
        .into_iter()
        .enumerate()
        .map(|(index, child)| {
            let child_uid =
                if d2b_provider_guest_cloud_hypervisor::child_role_for_ref(child.resource_ref())
                    == Some(ChildRole::SystemVolume)
                {
                    child.uid().clone()
                } else {
                    ResourceUid::parse(format!("423e4567-e89b-42d3-a456-42661417{index:04}"))
                        .unwrap()
                };
            OwnedChildSnapshot::new(
                child.resource_ref().clone(),
                child.zone().clone(),
                child.owner_ref().clone(),
                child_uid,
                child.generation(),
                child.revision(),
                child.spec_digest().to_owned(),
                ResourcePhase::Ready,
                child.desired_lifecycle(),
                true,
            )
            .unwrap()
            .with_owner_uid(guest.uid().clone())
        })
        .collect::<Vec<_>>();
    state.lock().await.children = replacement_children;
    assert!(controller.reconcile(guest.resource_ref()).await.is_ok());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn disruptive_update_reports_upgrade_required_without_in_place_repair() {
    let guest = guest("gateway", "work", GUEST_UID, ZONE_UID);
    let api = FakeApi::new(guest.clone(), dependencies(true, true, true, true, true));
    let plan = BootstrapGraph::plan_children(
        guest.zone().clone(),
        guest.resource_ref().clone(),
        guest.execution_ref().clone(),
        &descriptor(),
    )
    .unwrap();
    let batch = GuestChildCreateBatch::new(
        &guest,
        plan.child_batch(),
        plan.child_batch()
            .mutations()
            .iter()
            .map(|mutation| mutation.target().clone()),
    )
    .unwrap();
    let state = Arc::clone(&api.state);
    {
        let mut state = state.lock().await;
        state.children = matching_children(&guest, &batch);
        state.upgrade_reason = Some(UpgradeReason::ImageOrSystemGenerationChanged);
    }
    let mut controller = make_controller(api);
    controller.register().await.unwrap();
    let outcome = controller.reconcile(guest.resource_ref()).await.unwrap();
    assert!(matches!(
        outcome,
        d2b_provider_guest_cloud_hypervisor::CloudHypervisorReconcileOutcome::Degraded(_)
    ));
    assert!(
        outcome
            .status()
            .has_condition(d2b_provider_guest_cloud_hypervisor::GuestCondition::UpgradeRequired)
    );
    assert!(state.lock().await.updates.is_empty());
}
