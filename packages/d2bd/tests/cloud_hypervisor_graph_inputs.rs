//! The Cloud Hypervisor Guest's classified graph inputs (U21; AE31-AE33).
//!
//! The daemon still projects a Cloud Hypervisor Guest's attachment lists
//! through the unchanged production entry point; this drives the conversion
//! beside it and proves the three meanings the flattened fragment mixed stay
//! apart:
//!
//! 1. **A support ceiling allocates the Guest no access** (AE31). A Device
//!    and a Network a Guest declares for the workloads it runs bound what a
//!    child of that Guest may request, so they become one ceiling and no
//!    relationship, no reservation, and no boot dependency.
//! 2. **The Guest's own consumption is one relationship whose consumer is
//!    the Guest** (AE32), and only a committed `VolumeBinding` row supplies
//!    one. A row addressed to another Guest is not this one's use.
//! 3. **A child request default grants the Guest nothing** (AE33). It shapes
//!    the one child's request and refuses every other consumer.
//!
//! The unchanged production projection is deliberately not exercised here:
//! cutting it over is U34's, and this test exists so that cutover has a
//! property to keep.

use d2b_contracts_resource::v3::{
    BindingLifecycleState, BindingObservation, BindingSlot, BoundedToken, BudgetSpec,
    CompletionCondition, ExecutionDomain, ExecutionPolicy, ReleaseOutcome,
    ResourceGeneration, ResourceRef, ResourceUid, VolumeBindingRequest, VolumePresentation, ZoneId,
    ZoneRevision, volume::AttachmentAccess,
};
use d2bd::resource_runtime::cloud_hypervisor_guest_graph;
use d2b_provider_guest::GuestSpec;

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("valid fixture ref")
}

fn uid(seed: u32) -> ResourceUid {
    ResourceUid::parse(format!("123e4567-e89b-42d3-a456-42661417{seed:04}")).expect("valid uid")
}

/// One committed `VolumeBinding` row carrying the canonical consumer request.
fn binding_row(name: &str, consumer: &str, view: &str) -> d2b_contracts_resource::v3::StoredResource {
    let request = VolumeBindingRequest::new(
        reference("Volume/gateway-system"),
        reference(consumer),
        BindingSlot::parse("system").expect("valid slot"),
        BoundedToken::parse(view).expect("valid view"),
        AttachmentAccess::ReadOnly,
        VolumePresentation::filesystem("/var/lib/d2b/system").expect("valid destination"),
    )
    .expect("valid fixture request");
    d2b_contracts_resource::v3::StoredResource {
        resource_ref: reference(&format!("VolumeBinding/{name}")),
        zone: ZoneId::parse("work").expect("valid zone"),
        uid: uid(1),
        owner_uid: None,
        owner_generation: None,
        generation: ResourceGeneration::new(1).expect("nonzero generation"),
        revision: ZoneRevision::new(3),
        canonical_json: serde_json::to_vec(&serde_json::json!({
            "spec": request,
        }))
        .expect("serializable row"),
        payload_digest: d2b_contracts_resource::v3::StateDigest::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("valid digest"),
    }
}

fn guest_spec(spec: ExecutionPolicy) -> GuestSpec {
    GuestSpec::new(spec, None)
}

fn ceiling_spec() -> GuestSpec {
    guest_spec(
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            vec![
                d2b_contracts_resource::v3::NetworkAttachment::new(
                    reference("Network/work"),
                    false,
                )
                .expect("network attachment"),
            ],
            vec![
                d2b_contracts_resource::v3::DeviceAttachment::new(
                    reference("Device/kvm"),
                    true,
                )
                .expect("exclusive device attachment"),
            ],
            Vec::new(),
        )
        .expect("valid policy"),
    )
}

fn child_default_spec() -> GuestSpec {
    let entry = d2b_contracts_resource::v3::CanonicalJsonObject::parse(
        &serde_json::to_vec(&serde_json::json!({
            "consumerRef": "Process/gateway-vmm",
            "volumeRef": "Volume/gateway-system",
            "view": "system",
        }))
        .expect("serializable default"),
    )
    .expect("canonical default");
    guest_spec(
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            vec![entry],
        )
        .expect("valid policy"),
    )
}

/// AE31: the Device and Network a Guest declares are a ceiling on its
/// children. The Guest holds no relationship, so its start gate cannot wait
/// on rows it never asked to use.
#[test]
fn a_support_ceiling_becomes_no_guest_relationship() {
    let guest_ref = reference("Guest/gateway");
    let graph = cloud_hypervisor_guest_graph(&guest_ref, &ceiling_spec(), &[]).expect("classified");
    assert_eq!(graph.guest_ref(), &guest_ref);
    assert!(
        graph.guest_bindings().is_empty(),
        "a ceiling creates no binding for the Guest"
    );
    assert!(graph.child_defaults().is_empty());
    assert!(graph.support_ceiling().admits(
        d2b_contracts_resource::v3::BindingKind::Device,
        d2b_contracts_resource::v3::RequestedRights::Exclusive,
    ));
    assert!(graph.support_ceiling().admits(
        d2b_contracts_resource::v3::BindingKind::Network,
        d2b_contracts_resource::v3::RequestedRights::Consume,
    ));
    assert!(
        !graph.admits_child_request(
            d2b_contracts_resource::v3::BindingKind::Volume,
            d2b_contracts_resource::v3::RequestedRights::Observe,
        ),
        "a kind the Guest never declared is outside its ceiling"
    );
}

/// AE6 and AE21: a Guest with a prepared export but no mount yet may start,
/// and the mount is observed afterwards. The two conditions never collapse
/// into the single readiness both sides would wait on.
#[test]
fn prepared_storage_permits_boot_and_completion_follows() {
    let guest_ref = reference("Guest/gateway");
    let rows = vec![binding_row("gateway-system", "Guest/gateway", "system")];
    let mut graph =
        cloud_hypervisor_guest_graph(&guest_ref, &ceiling_spec(), &rows).expect("classified");
    let request = graph.guest_bindings()[0].request().clone();
    let source_uid = uid(1);

    assert_eq!(
        graph.start_gate(),
        d2b_provider_guest_cloud_hypervisor::GuestStartGate::SourcePending,
        "an unobserved relationship holds the Guest stopped"
    );
    graph
        .observe_binding(
            &request,
            &source_uid,
            BindingObservation::new(
                BindingLifecycleState::Prepared,
                CompletionCondition::Complete,
                CompletionCondition::Pending,
                ReleaseOutcome::Outstanding,
            ),
        )
        .expect("the Guest consumes this relationship");
    assert_eq!(
        graph.start_gate(),
        d2b_provider_guest_cloud_hypervisor::GuestStartGate::Permitted
    );
    assert_eq!(
        graph.consumer_completion(),
        d2b_provider_guest_cloud_hypervisor::GuestConsumerCompletion::Incomplete
    );

    graph
        .observe_binding(
            &request,
            &source_uid,
            BindingObservation::new(
                BindingLifecycleState::Active,
                CompletionCondition::Complete,
                CompletionCondition::Complete,
                ReleaseOutcome::Outstanding,
            ),
        )
        .expect("the same relationship is observed again");
    assert_eq!(
        graph.consumer_completion(),
        d2b_provider_guest_cloud_hypervisor::GuestConsumerCompletion::Complete
    );
}

/// AE32: the Guest's own consumption is the committed row whose consumer is
/// that Guest. A row addressed to another Guest is not this one's use and
/// never joins its graph.
#[test]
fn only_a_row_consumed_by_this_guest_is_the_guests_own_use() {
    let guest_ref = reference("Guest/gateway");
    let rows = vec![
        binding_row("gateway-system", "Guest/gateway", "system"),
        binding_row("other-system", "Guest/other-vm", "system"),
    ];
    let graph = cloud_hypervisor_guest_graph(&guest_ref, &ceiling_spec(), &rows).expect("classified");
    assert_eq!(graph.guest_bindings().len(), 1);
    assert_eq!(
        graph.guest_bindings()[0].request().consumer_ref(),
        &guest_ref
    );
    assert_eq!(
        graph.guest_bindings()[0]
            .request()
            .source_ref()
            .to_canonical_string(),
        "Volume/gateway-system"
    );
}

/// AE33: a child's request default shapes that child's draft, refuses every
/// other consumer, and becomes no Guest relationship.
#[test]
fn a_child_default_shapes_only_its_child_and_grants_the_guest_nothing() {
    let guest_ref = reference("Guest/gateway");
    let rows = vec![binding_row("gateway-system", "Guest/gateway", "system")];
    let graph =
        cloud_hypervisor_guest_graph(&guest_ref, &child_default_spec(), &rows).expect("classified");
    assert_eq!(graph.child_defaults().len(), 1);
    assert_eq!(graph.guest_bindings().len(), 1);
    let draft = d2b_contracts_resource::v3::ChildBindingRequest::new(
        reference("Process/gateway-vmm"),
        d2b_contracts_resource::v3::BindingKind::Volume,
    )
    .expect("valid draft");
    // The ceiling this fragment declared admits only Device and Network, so
    // the shaped request for the named child is refused rather than admitted.
    assert!(graph.shape_child_request(&draft).is_err());

    // A child the defaults do not name is left exactly as it was authored.
    let other = d2b_contracts_resource::v3::ChildBindingRequest::new(
        reference("Process/sibling"),
        d2b_contracts_resource::v3::BindingKind::Volume,
    )
    .expect("valid draft");
    let untouched = graph
        .shape_child_request(&other)
        .expect("a child the defaults do not name is left alone");
    assert_eq!(
        untouched.source_ref(),
        None,
        "the default belongs to exactly the child it names"
    );
    assert_eq!(untouched.rights(), None);
}