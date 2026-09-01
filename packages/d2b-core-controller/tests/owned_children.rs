use std::collections::BTreeSet;

use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_core_controller::{
    CoreTriggerReason, DependencyEvent, DependencyIndex, DesiredChild, HintTarget, ObservedChild,
    OwnedChildKind, OwnerBatchResult, OwnerChildIdentity, OwnerIndex, OwnerLimits, OwnerMutation,
};

fn uid(suffix: u8) -> ResourceUid {
    ResourceUid::parse(format!("123e4567-e89b-42d3-a456-4266141741{suffix:02}")).unwrap()
}

fn owner() -> HintTarget {
    HintTarget::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Guest/app").unwrap(),
        uid(0),
    )
}

fn child(
    kind: OwnedChildKind,
    resource_type: &str,
    name: &str,
    dependencies: impl IntoIterator<Item = ResourceRef>,
) -> DesiredChild {
    let value = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": resource_type,
        "metadata": {
            "name": name,
            "zone": "work",
            "ownerRef": "Guest/app"
        },
        "spec": {}
    });
    let bytes = CanonicalJsonValue::parse(&serde_json::to_vec(&value).unwrap())
        .unwrap()
        .to_canonical_bytes();
    DesiredChild::new(
        ResourceRef::parse(&format!("{resource_type}/{name}")).unwrap(),
        bytes,
        format!("digest-{resource_type}-{name}"),
    )
    .unwrap()
    .with_kind(kind)
    .with_dependencies(dependencies)
    .unwrap()
}

fn observed(
    target: HintTarget,
    revision: u64,
    owner_generation: u64,
    dependencies: impl IntoIterator<Item = ResourceRef>,
) -> ObservedChild {
    ObservedChild::with_owner_and_dependencies(
        target,
        &owner(),
        ResourceGeneration::new(owner_generation).unwrap(),
        ZoneRevision::new(revision),
        "current",
        false,
        false,
        dependencies,
    )
    .unwrap()
}

fn target(resource_type: &str, name: &str, suffix: u8) -> HintTarget {
    HintTarget::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse(&format!("{resource_type}/{name}")).unwrap(),
        uid(suffix),
    )
}

#[test]
fn plans_uid_free_process_endpoint_volume_batch_and_recovers_uncertain_response() {
    let volume_ref = ResourceRef::parse("Volume/system").unwrap();
    let process_ref = ResourceRef::parse("Process/vmm").unwrap();
    let endpoint_ref = ResourceRef::parse("Endpoint/control").unwrap();
    let desired = vec![
        child(
            OwnedChildKind::Endpoint,
            "Endpoint",
            "control",
            [process_ref.clone()],
        ),
        child(OwnedChildKind::Volume, "Volume", "system", []),
        child(
            OwnedChildKind::Process,
            "Process",
            "vmm",
            [volume_ref.clone()],
        ),
    ];

    let mut index = OwnerIndex::new(OwnerLimits::new(8, 16).unwrap());
    let parent = owner();
    index
        .relist_with_owner_generation(
            parent.clone(),
            ResourceGeneration::new(4).unwrap(),
            Vec::new(),
        )
        .unwrap();
    let plan = index.plan_intents(&parent, desired).unwrap();

    assert_eq!(
        plan.creation_order(),
        &[
            volume_ref.clone(),
            process_ref.clone(),
            endpoint_ref.clone()
        ]
    );
    let batch = plan
        .create_batch()
        .expect("the complete graph is a create batch");
    assert_eq!(
        batch.refs(),
        &[
            volume_ref.clone(),
            process_ref.clone(),
            endpoint_ref.clone()
        ]
    );
    assert!(batch.children().iter().all(|child| {
        !child
            .canonical_resource()
            .windows(5)
            .any(|window| window == b"uid\"")
    }));

    let committed = OwnerBatchResult::committed([
        OwnerChildIdentity::new(volume_ref.clone(), uid(1), ZoneRevision::new(10)),
        OwnerChildIdentity::new(process_ref.clone(), uid(2), ZoneRevision::new(10)),
        OwnerChildIdentity::new(endpoint_ref.clone(), uid(3), ZoneRevision::new(10)),
    ]);
    let resolved = committed.resolve(&batch, &[]).unwrap();
    assert!(!resolved.was_relisted());
    assert_eq!(resolved.uid(&endpoint_ref), Some(&uid(3)));

    let uncertain = OwnerBatchResult::uncertain();
    let relisted = vec![
        observed(target("Volume", "system", 1), 10, 4, std::iter::empty()),
        observed(target("Process", "vmm", 2), 10, 4, [volume_ref.clone()]),
        observed(
            target("Endpoint", "control", 3),
            10,
            4,
            [process_ref.clone()],
        ),
    ];
    let recovered = uncertain.resolve(&batch, &relisted).unwrap();
    assert!(recovered.was_relisted());
    assert_eq!(recovered.uid(&volume_ref), Some(&uid(1)));
    assert_eq!(recovered.uid(&process_ref), Some(&uid(2)));
    assert_eq!(recovered.uid(&endpoint_ref), Some(&uid(3)));
}

#[test]
fn rejects_partial_batch_and_fences_missing_extra_foreign_cross_zone_and_stale_children() {
    let parent = owner();
    let volume_ref = ResourceRef::parse("Volume/system").unwrap();
    let process_ref = ResourceRef::parse("Process/vmm").unwrap();
    let endpoint_ref = ResourceRef::parse("Endpoint/control").unwrap();
    let desired = vec![
        child(OwnedChildKind::Volume, "Volume", "system", []),
        child(
            OwnedChildKind::Process,
            "Process",
            "vmm",
            [volume_ref.clone()],
        ),
        child(
            OwnedChildKind::Endpoint,
            "Endpoint",
            "control",
            [process_ref.clone()],
        ),
    ];
    let mut index = OwnerIndex::new(OwnerLimits::new(8, 16).unwrap());
    index
        .relist_with_owner_generation(
            parent.clone(),
            ResourceGeneration::new(4).unwrap(),
            vec![observed(
                target("Volume", "system", 1),
                10,
                4,
                std::iter::empty(),
            )],
        )
        .unwrap();
    let plan = index.plan_intents(&parent, desired.clone()).unwrap();
    assert!(plan
        .mutations()
        .iter()
        .any(|mutation| matches!(mutation, OwnerMutation::Create { target, .. } if target == &process_ref)));
    let batch = plan.create_batch().unwrap();
    assert_eq!(batch.refs(), &[process_ref.clone(), endpoint_ref.clone()]);
    assert!(
        OwnerBatchResult::committed([OwnerChildIdentity::new(
            process_ref.clone(),
            uid(2),
            ZoneRevision::new(10),
        )])
        .resolve(&batch, &[])
        .is_err()
    );

    let foreign_owner = HintTarget::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Guest/other").unwrap(),
        uid(8),
    );
    let foreign = ObservedChild::with_owner(
        target("Process", "foreign", 9),
        &foreign_owner,
        ResourceGeneration::new(4).unwrap(),
        ZoneRevision::new(10),
        "current",
        false,
    )
    .unwrap();
    assert!(
        index
            .relist_with_owner_generation(
                parent.clone(),
                ResourceGeneration::new(4).unwrap(),
                vec![foreign],
            )
            .is_err()
    );

    let cross_zone = ObservedChild::new(
        HintTarget::new(
            ZoneId::parse("other").unwrap(),
            ResourceRef::parse("Process/foreign").unwrap(),
            uid(9),
        ),
        ZoneRevision::new(10),
        "current",
        false,
    )
    .unwrap();
    assert!(
        index
            .relist_with_owner_generation(
                parent.clone(),
                ResourceGeneration::new(4).unwrap(),
                vec![cross_zone],
            )
            .is_err()
    );

    let stale_owner = ObservedChild::with_owner_identity(
        target("Process", "stale-owner", 9),
        parent.resource_ref().clone(),
        uid(7),
        ResourceGeneration::new(4).unwrap(),
        ZoneRevision::new(10),
        "current",
        false,
        false,
        [],
    )
    .unwrap();
    assert!(
        index
            .relist_with_owner_generation(
                parent.clone(),
                ResourceGeneration::new(4).unwrap(),
                vec![stale_owner],
            )
            .is_err()
    );

    let stale_generation = observed(
        target("Process", "stale-generation", 9),
        10,
        3,
        std::iter::empty(),
    );
    assert!(
        index
            .relist_with_owner_generation(
                parent,
                ResourceGeneration::new(4).unwrap(),
                vec![stale_generation]
            )
            .is_err()
    );

    let mut extra_index = OwnerIndex::new(OwnerLimits::new(8, 16).unwrap());
    let extra = observed(target("Device", "extra", 10), 11, 4, std::iter::empty())
        .with_kind(OwnedChildKind::Other);
    extra_index
        .relist_with_owner_generation(
            owner(),
            ResourceGeneration::new(4).unwrap(),
            vec![
                observed(target("Volume", "system", 1), 10, 4, std::iter::empty()),
                extra.clone(),
            ],
        )
        .unwrap();
    let extra_plan = extra_index
        .plan(
            &owner(),
            vec![child(OwnedChildKind::Volume, "Volume", "system", [])],
        )
        .unwrap();
    assert!(extra_plan.mutations().iter().any(
        |mutation| matches!(mutation, OwnerMutation::RequestDeletion { target, .. } if target == extra.target().resource_ref())
    ));

    let duplicate = observed(target("Volume", "system", 1), 10, 4, std::iter::empty());
    assert!(
        extra_index
            .relist_with_owner_generation(
                owner(),
                ResourceGeneration::new(4).unwrap(),
                vec![duplicate.clone(), duplicate],
            )
            .is_err()
    );
}

#[test]
fn teardown_is_deterministic_and_leaves_are_deleted_before_standard_children() {
    let volume = target("Volume", "system", 1);
    let process = target("Process", "vmm", 2);
    let endpoint = target("Endpoint", "control", 3);
    let leaf = target("Device", "leaf", 4);
    let volume_ref = volume.resource_ref().clone();
    let process_ref = process.resource_ref().clone();
    let endpoint_ref = endpoint.resource_ref().clone();
    let leaf_ref = leaf.resource_ref().clone();
    let mut index = OwnerIndex::new(OwnerLimits::new(8, 16).unwrap());
    index
        .relist(
            owner(),
            vec![
                observed(volume.clone(), 2, 4, std::iter::empty()),
                observed(process.clone(), 3, 4, [volume_ref.clone()]),
                observed(endpoint.clone(), 4, 4, [process_ref.clone()]),
                observed(leaf, 5, 4, [endpoint_ref.clone()]).with_kind(OwnedChildKind::Other),
            ],
        )
        .unwrap();
    let plan = index.plan(&owner(), Vec::new()).unwrap();

    assert_eq!(
        plan.deletion_order(),
        &[leaf_ref, endpoint_ref, process_ref, volume_ref]
    );
    assert_eq!(plan.mutations().len(), 4);
    assert!(plan.mutations().iter().zip(plan.deletion_order()).all(
        |(mutation, target)| matches!(mutation, OwnerMutation::RequestDeletion { target: actual, .. } if actual == target)
    ));
}

#[test]
fn dependency_reasons_union_without_losing_ready_or_changed_trigger() {
    let network = target("Network", "lan", 1);
    let volume = target("Volume", "system", 2);
    let guest = target("Guest", "app", 3);
    let controller = d2b_core_controller::ControllerLeaseKey::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Process/guest-controller").unwrap(),
    )
    .unwrap();
    let mut index = DependencyIndex::default();
    index
        .register(
            controller,
            guest.clone(),
            BTreeSet::from([network.clone(), volume.clone()]),
        )
        .unwrap();

    let merged = index
        .triggers_for([
            DependencyEvent::new(network, ZoneRevision::new(8), false).unwrap(),
            DependencyEvent::new(volume, ZoneRevision::new(9), true).unwrap(),
        ])
        .unwrap();

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].target(), &guest);
    assert_eq!(merged[0].revision(), ZoneRevision::new(9));
    assert_eq!(
        merged[0].reasons(),
        &BTreeSet::from([
            CoreTriggerReason::DependencyChanged,
            CoreTriggerReason::DependencyReady
        ])
    );
}
