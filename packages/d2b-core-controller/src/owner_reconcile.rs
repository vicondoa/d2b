//! Owner reverse index, bounded propagation, and desired-child repair plans.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{ResourceRef, ResourceUid, ZoneRevision};

use crate::hints::HintTarget;

/// Canonical owner propagation limits supplied by the controller toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerLimits {
    max_depth: usize,
    max_work_items: usize,
}

impl OwnerLimits {
    /// Bind the canonical toolkit limits without redefining them in Core.
    pub fn new(max_depth: usize, max_work_items: usize) -> Result<Self, OwnerGraphError> {
        if max_depth == 0 || max_work_items == 0 || max_depth > max_work_items {
            return Err(OwnerGraphError::InvalidLimits);
        }
        Ok(Self {
            max_depth,
            max_work_items,
        })
    }
}

/// One desired child body and digest.
#[derive(Clone, PartialEq, Eq)]
pub struct DesiredChild {
    target: ResourceRef,
    canonical_resource: Vec<u8>,
    payload_digest: String,
}

impl DesiredChild {
    /// Construct a desired child.
    pub fn new(
        target: ResourceRef,
        canonical_resource: Vec<u8>,
        payload_digest: impl Into<String>,
    ) -> Result<Self, OwnerReconcileError> {
        let payload_digest = payload_digest.into();
        if canonical_resource.is_empty() || payload_digest.is_empty() || payload_digest.len() > 128
        {
            return Err(OwnerReconcileError::InvalidChild);
        }
        Ok(Self {
            target,
            canonical_resource,
            payload_digest,
        })
    }

    /// Borrow the target reference.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Borrow the canonical desired child body.
    pub fn canonical_resource(&self) -> &[u8] {
        &self.canonical_resource
    }

    /// Borrow the desired body digest.
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }
}

impl core::fmt::Debug for DesiredChild {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DesiredChild")
            .field("target_type", self.target.resource_type())
            .field(
                "canonical_resource",
                &format_args!("<{} bytes>", self.canonical_resource.len()),
            )
            .field("has_payload_digest", &true)
            .finish()
    }
}

/// One complete observed child-index row.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservedChild {
    target: HintTarget,
    revision: ZoneRevision,
    payload_digest: String,
    deletion_requested: bool,
}

impl ObservedChild {
    /// Construct an observed index row.
    pub fn new(
        target: HintTarget,
        revision: ZoneRevision,
        payload_digest: impl Into<String>,
        deletion_requested: bool,
    ) -> Result<Self, OwnerReconcileError> {
        let payload_digest = payload_digest.into();
        if revision.get() == 0 || payload_digest.is_empty() || payload_digest.len() > 128 {
            return Err(OwnerReconcileError::InvalidChild);
        }
        Ok(Self {
            target,
            revision,
            payload_digest,
            deletion_requested,
        })
    }

    /// Borrow the indexed target.
    pub const fn target(&self) -> &HintTarget {
        &self.target
    }

    /// Borrow the observed body digest.
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }
}

impl core::fmt::Debug for ObservedChild {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ObservedChild")
            .field("target", &self.target)
            .field("revision", &self.revision)
            .field("has_payload_digest", &true)
            .field("deletion_requested", &self.deletion_requested)
            .finish()
    }
}

/// One optimistic owner repair operation.
#[derive(Clone, PartialEq, Eq)]
pub enum OwnerMutation {
    Create {
        target: ResourceRef,
        canonical_resource: Vec<u8>,
    },
    Repair {
        target: ResourceRef,
        expected_uid: ResourceUid,
        expected_revision: ZoneRevision,
        canonical_resource: Vec<u8>,
    },
    RequestDeletion {
        target: ResourceRef,
        expected_uid: ResourceUid,
        expected_revision: ZoneRevision,
    },
}

impl core::fmt::Debug for OwnerMutation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Create {
                target,
                canonical_resource,
            } => f
                .debug_struct("OwnerMutation::Create")
                .field("target_type", target.resource_type())
                .field(
                    "canonical_resource",
                    &format_args!("<{} bytes>", canonical_resource.len()),
                )
                .finish(),
            Self::Repair {
                target,
                expected_revision,
                canonical_resource,
                ..
            } => f
                .debug_struct("OwnerMutation::Repair")
                .field("target_type", target.resource_type())
                .field("has_expected_uid", &true)
                .field("expected_revision", expected_revision)
                .field(
                    "canonical_resource",
                    &format_args!("<{} bytes>", canonical_resource.len()),
                )
                .finish(),
            Self::RequestDeletion {
                target,
                expected_revision,
                ..
            } => f
                .debug_struct("OwnerMutation::RequestDeletion")
                .field("target_type", target.resource_type())
                .field("has_expected_uid", &true)
                .field("expected_revision", expected_revision)
                .finish(),
        }
    }
}

/// Complete desired-vs-observed owner plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerReconcilePlan {
    mutations: Vec<OwnerMutation>,
}

impl OwnerReconcilePlan {
    /// Borrow optimistic operations.
    pub fn mutations(&self) -> &[OwnerMutation] {
        &self.mutations
    }

    /// Whether the complete child set is converged.
    pub fn is_converged(&self) -> bool {
        self.mutations.is_empty()
    }
}

/// Complete owner index, replaced only by an authoritative relist.
pub struct OwnerIndex {
    limits: OwnerLimits,
    children: BTreeMap<HintTarget, BTreeMap<ResourceRef, ObservedChild>>,
}

impl OwnerIndex {
    /// Construct an index bound by toolkit-owned limits.
    pub fn new(limits: OwnerLimits) -> Self {
        Self {
            limits,
            children: BTreeMap::new(),
        }
    }

    /// Replace one owner's complete child set.
    pub fn relist(
        &mut self,
        owner: HintTarget,
        observed: Vec<ObservedChild>,
    ) -> Result<(), OwnerReconcileError> {
        if observed.len() > self.limits.max_work_items
            || observed.iter().any(|child| {
                child.target.zone() != owner.zone()
                    || child.target.resource_ref() == owner.resource_ref()
            })
        {
            return Err(OwnerReconcileError::InvalidChild);
        }
        let mut indexed = BTreeMap::new();
        for child in observed {
            let child_ref = child.target.resource_ref().clone();
            if indexed.insert(child_ref, child).is_some() {
                return Err(OwnerReconcileError::DuplicateChild);
            }
        }
        self.children.insert(owner, indexed);
        Ok(())
    }

    /// Compare complete desired children with the latest relist.
    pub fn plan(
        &self,
        owner: &HintTarget,
        desired: Vec<DesiredChild>,
    ) -> Result<OwnerReconcilePlan, OwnerReconcileError> {
        if desired.len() > self.limits.max_work_items {
            return Err(OwnerReconcileError::TooManyChildren);
        }
        let mut desired_by_ref = BTreeMap::new();
        for child in desired {
            if &child.target == owner.resource_ref() {
                return Err(OwnerReconcileError::InvalidChild);
            }
            if desired_by_ref.insert(child.target.clone(), child).is_some() {
                return Err(OwnerReconcileError::DuplicateChild);
            }
        }
        let observed = self
            .children
            .get(owner)
            .cloned()
            .ok_or(OwnerReconcileError::OwnerNotRelisted)?;
        let mut mutations = Vec::new();
        for (target, desired) in &desired_by_ref {
            match observed.get(target) {
                None => mutations.push(OwnerMutation::Create {
                    target: target.clone(),
                    canonical_resource: desired.canonical_resource.clone(),
                }),
                Some(actual) if actual.payload_digest != desired.payload_digest => {
                    mutations.push(OwnerMutation::Repair {
                        target: target.clone(),
                        expected_uid: actual.target.uid().clone(),
                        expected_revision: actual.revision,
                        canonical_resource: desired.canonical_resource.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        for (target, actual) in observed {
            if !desired_by_ref.contains_key(&target) && !actual.deletion_requested {
                mutations.push(OwnerMutation::RequestDeletion {
                    target,
                    expected_uid: actual.target.uid().clone(),
                    expected_revision: actual.revision,
                });
            }
        }
        Ok(OwnerReconcilePlan { mutations })
    }

    /// Number of children in the latest complete relist.
    pub fn child_count(&self, owner: &HintTarget) -> usize {
        self.children.get(owner).map_or(0, BTreeMap::len)
    }
}

/// One owner-change trigger.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerTrigger {
    owner: HintTarget,
    child: HintTarget,
    revision: ZoneRevision,
    depth: usize,
}

impl OwnerTrigger {
    /// Borrow the owner target.
    pub const fn owner(&self) -> &HintTarget {
        &self.owner
    }

    /// Borrow the changed child at this hop.
    pub const fn child(&self) -> &HintTarget {
        &self.child
    }

    /// Return the coalesced revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return one-based ancestor depth.
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Coalesce the same immutable owner-child binding.
    pub fn coalesce(&mut self, newer: Self) -> Result<(), OwnerGraphError> {
        if self.owner != newer.owner || self.child != newer.child || self.depth != newer.depth {
            return Err(OwnerGraphError::DifferentBinding);
        }
        self.revision = self.revision.max(newer.revision);
        Ok(())
    }
}

impl core::fmt::Debug for OwnerTrigger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OwnerTrigger")
            .field("owner", &self.owner)
            .field("child", &self.child)
            .field("revision", &self.revision)
            .field("depth", &self.depth)
            .finish()
    }
}

/// Acyclic singular-owner graph.
pub struct OwnerGraph {
    limits: OwnerLimits,
    parents: BTreeMap<HintTarget, HintTarget>,
}

impl OwnerGraph {
    /// Construct an owner graph bound by toolkit-owned limits.
    pub fn new(limits: OwnerLimits) -> Self {
        Self {
            limits,
            parents: BTreeMap::new(),
        }
    }

    /// Bind one child to one same-Zone owner.
    pub fn bind(&mut self, child: HintTarget, owner: HintTarget) -> Result<(), OwnerGraphError> {
        if child == owner || child.zone() != owner.zone() {
            return Err(OwnerGraphError::InvalidBinding);
        }
        let previous = self.parents.insert(child.clone(), owner);
        if self
            .parents
            .keys()
            .any(|candidate| self.validate_from(candidate).is_err())
        {
            if let Some(previous) = previous {
                self.parents.insert(child, previous);
            } else {
                self.parents.remove(&child);
            }
            return Err(OwnerGraphError::CycleOrDepth);
        }
        Ok(())
    }

    /// Remove a child binding.
    pub fn unbind(&mut self, child: &HintTarget) -> bool {
        self.parents.remove(child).is_some()
    }

    /// Propagate one durable mutation to every bounded ancestor.
    pub fn propagate(
        &self,
        changed_child: &HintTarget,
        revision: ZoneRevision,
    ) -> Result<Vec<OwnerTrigger>, OwnerGraphError> {
        if revision.get() == 0 {
            return Err(OwnerGraphError::InvalidRevision);
        }
        let mut triggers = Vec::new();
        let mut child = changed_child.clone();
        let mut visited = BTreeSet::from([child.clone()]);
        while let Some(owner) = self.parents.get(&child) {
            if !visited.insert(owner.clone()) {
                return Err(OwnerGraphError::CycleOrDepth);
            }
            let depth = triggers.len() + 1;
            if depth > self.limits.max_depth || depth > self.limits.max_work_items {
                return Err(OwnerGraphError::CycleOrDepth);
            }
            triggers.push(OwnerTrigger {
                owner: owner.clone(),
                child: child.clone(),
                revision,
                depth,
            });
            child = owner.clone();
        }
        Ok(triggers)
    }

    /// Remove every binding whose child or owner belongs to a withdrawn set.
    pub fn withdraw(&mut self, resources: &BTreeSet<HintTarget>) -> usize {
        let before = self.parents.len();
        self.parents
            .retain(|child, owner| !resources.contains(child) && !resources.contains(owner));
        before - self.parents.len()
    }

    fn validate_from(&self, child: &HintTarget) -> Result<(), OwnerGraphError> {
        let mut current = child;
        let mut visited = BTreeSet::from([child.clone()]);
        let mut depth = 0;
        while let Some(owner) = self.parents.get(current) {
            depth += 1;
            if depth > self.limits.max_depth || !visited.insert(owner.clone()) {
                return Err(OwnerGraphError::CycleOrDepth);
            }
            current = owner;
        }
        Ok(())
    }
}

/// Invalid desired/observed child set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerReconcileError {
    InvalidChild,
    DuplicateChild,
    TooManyChildren,
    OwnerNotRelisted,
}

impl core::fmt::Display for OwnerReconcileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidChild => "owner child is malformed, cross-Zone, or over the bound",
            Self::DuplicateChild => "complete owner child set contains a duplicate reference",
            Self::TooManyChildren => "owner desired child set exceeds its work bound",
            Self::OwnerNotRelisted => {
                "owner planning requires a complete authoritative child relist"
            }
        })
    }
}

impl std::error::Error for OwnerReconcileError {}

/// Invalid singular-owner graph operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerGraphError {
    InvalidLimits,
    InvalidBinding,
    InvalidRevision,
    CycleOrDepth,
    DifferentBinding,
}

impl core::fmt::Display for OwnerGraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "owner propagation limits are empty or inconsistent",
            Self::InvalidBinding => "owner binding is self-owned or cross-Zone",
            Self::InvalidRevision => "owner propagation requires a durable revision",
            Self::CycleOrDepth => "owner propagation is cyclic or exceeds its depth bound",
            Self::DifferentBinding => "only one immutable owner-child binding may coalesce",
        })
    }
}

impl std::error::Error for OwnerGraphError {}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{ResourceUid, ZoneId};

    use super::*;

    fn target(zone: &str, resource_type: &str, name: &str, suffix: u8) -> HintTarget {
        HintTarget::new(
            ZoneId::parse(zone).unwrap(),
            ResourceRef::parse(&format!("{resource_type}/{name}")).unwrap(),
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-4266141740{suffix:02}")).unwrap(),
        )
    }

    fn desired(resource_type: &str, name: &str, digest: &str) -> DesiredChild {
        DesiredChild::new(
            ResourceRef::parse(&format!("{resource_type}/{name}")).unwrap(),
            format!("{{\"name\":\"{name}\"}}").into_bytes(),
            digest,
        )
        .unwrap()
    }

    fn limits() -> OwnerLimits {
        OwnerLimits::new(8, 64).unwrap()
    }

    fn observed(
        resource_type: &str,
        name: &str,
        suffix: u8,
        revision: u64,
        digest: &str,
    ) -> ObservedChild {
        ObservedChild::new(
            target("work", resource_type, name, suffix),
            ZoneRevision::new(revision),
            digest,
            false,
        )
        .unwrap()
    }

    #[test]
    fn complete_relist_drives_create_repair_and_delete_plan() {
        let owner = target("work", "Guest", "desktop", 1);
        let mut index = OwnerIndex::new(limits());
        index
            .relist(
                owner.clone(),
                vec![
                    observed("Process", "drifted", 2, 4, "old"),
                    observed("Process", "extra", 3, 5, "extra"),
                ],
            )
            .unwrap();
        let plan = index
            .plan(
                &owner,
                vec![
                    desired("Process", "missing", "new"),
                    desired("Process", "drifted", "new"),
                ],
            )
            .unwrap();

        assert_eq!(plan.mutations().len(), 3);
        assert!(matches!(plan.mutations()[0], OwnerMutation::Repair { .. }));
        assert!(matches!(plan.mutations()[1], OwnerMutation::Create { .. }));
        assert!(matches!(
            plan.mutations()[2],
            OwnerMutation::RequestDeletion { .. }
        ));
    }

    #[test]
    fn repair_and_delete_keep_exact_uid_revision_preconditions() {
        let owner = target("work", "Guest", "desktop", 1);
        let drifted = observed("Process", "app", 2, 9, "old");
        let expected_uid = drifted.target().uid().clone();
        let mut index = OwnerIndex::new(limits());
        index.relist(owner.clone(), vec![drifted]).unwrap();

        let repair = index
            .plan(&owner, vec![desired("Process", "app", "new")])
            .unwrap();
        assert!(matches!(
            &repair.mutations()[0],
            OwnerMutation::Repair {
                expected_uid: uid,
                expected_revision,
                ..
            } if uid == &expected_uid && *expected_revision == ZoneRevision::new(9)
        ));

        let delete = index.plan(&owner, Vec::new()).unwrap();
        assert!(matches!(
            &delete.mutations()[0],
            OwnerMutation::RequestDeletion {
                expected_uid: uid,
                expected_revision,
                ..
            } if uid == &expected_uid && *expected_revision == ZoneRevision::new(9)
        ));
    }

    #[test]
    fn authoritative_relist_replaces_stale_children() {
        let owner = target("work", "Guest", "desktop", 1);
        let mut index = OwnerIndex::new(limits());
        index
            .relist(owner.clone(), vec![observed("Process", "old", 2, 2, "old")])
            .unwrap();
        index
            .relist(owner.clone(), vec![observed("Process", "new", 3, 3, "new")])
            .unwrap();

        assert_eq!(index.child_count(&owner), 1);
        let plan = index
            .plan(&owner, vec![desired("Process", "new", "new")])
            .unwrap();
        assert!(plan.is_converged());
    }

    #[test]
    fn planning_fails_closed_until_the_owner_index_is_relisted() {
        let owner = target("work", "Guest", "desktop", 1);
        let index = OwnerIndex::new(limits());

        assert_eq!(
            index
                .plan(&owner, vec![desired("Process", "app", "new")])
                .unwrap_err(),
            OwnerReconcileError::OwnerNotRelisted
        );
    }

    #[test]
    fn owner_cannot_be_listed_as_its_own_child() {
        let owner = target("work", "Guest", "desktop", 1);
        let mut index = OwnerIndex::new(limits());

        assert_eq!(
            index
                .relist(
                    owner.clone(),
                    vec![
                        ObservedChild::new(
                            target("work", "Guest", "desktop", 2),
                            ZoneRevision::new(1),
                            "digest",
                            false,
                        )
                        .unwrap(),
                    ],
                )
                .unwrap_err(),
            OwnerReconcileError::InvalidChild
        );
        index.relist(owner.clone(), Vec::new()).unwrap();
        assert_eq!(
            index
                .plan(&owner, vec![desired("Guest", "desktop", "digest")])
                .unwrap_err(),
            OwnerReconcileError::InvalidChild
        );
    }

    #[test]
    fn child_mutation_propagates_through_each_ancestor() {
        let zone = target("work", "Zone", "work", 1);
        let guest = target("work", "Guest", "desktop", 2);
        let process = target("work", "Process", "app", 3);
        let endpoint = target("work", "Endpoint", "socket", 4);
        let mut graph = OwnerGraph::new(limits());
        graph.bind(endpoint.clone(), process.clone()).unwrap();
        graph.bind(process.clone(), guest.clone()).unwrap();
        graph.bind(guest.clone(), zone.clone()).unwrap();

        let triggers = graph.propagate(&endpoint, ZoneRevision::new(11)).unwrap();
        assert_eq!(triggers.len(), 3);
        assert_eq!(triggers[0].owner(), &process);
        assert_eq!(triggers[1].owner(), &guest);
        assert_eq!(triggers[2].owner(), &zone);
        assert_eq!(triggers[2].depth(), 3);
    }

    #[test]
    fn owner_propagation_requires_a_durable_revision() {
        let child = target("work", "Process", "child", 1);
        let graph = OwnerGraph::new(limits());

        assert_eq!(
            graph.propagate(&child, ZoneRevision::new(0)).unwrap_err(),
            OwnerGraphError::InvalidRevision
        );
    }

    #[test]
    fn owner_graph_rejects_cross_zone_and_cycles() {
        let one = target("work", "Process", "one", 1);
        let two = target("work", "Process", "two", 2);
        let foreign = target("personal", "Process", "foreign", 3);
        let mut graph = OwnerGraph::new(limits());
        assert_eq!(
            graph.bind(one.clone(), foreign).unwrap_err(),
            OwnerGraphError::InvalidBinding
        );
        graph.bind(one.clone(), two.clone()).unwrap();
        assert_eq!(
            graph.bind(two, one).unwrap_err(),
            OwnerGraphError::CycleOrDepth
        );
    }

    #[test]
    fn owner_trigger_coalescing_keeps_high_water_revision() {
        let owner = target("work", "Guest", "desktop", 1);
        let child = target("work", "Process", "app", 2);
        let mut trigger = OwnerTrigger {
            owner: owner.clone(),
            child: child.clone(),
            revision: ZoneRevision::new(4),
            depth: 1,
        };
        trigger
            .coalesce(OwnerTrigger {
                owner,
                child,
                revision: ZoneRevision::new(8),
                depth: 1,
            })
            .unwrap();
        assert_eq!(trigger.revision(), ZoneRevision::new(8));
    }

    #[test]
    fn owner_chain_depth_bound_is_enforced_during_binding() {
        let resources: Vec<_> = (0..=9)
            .map(|index| {
                target(
                    "work",
                    "Process",
                    &format!("node-{index}"),
                    u8::try_from(index + 1).unwrap(),
                )
            })
            .collect();
        let mut graph = OwnerGraph::new(limits());
        for pair in resources.windows(2).take(limits().max_depth) {
            graph.bind(pair[0].clone(), pair[1].clone()).unwrap();
        }
        assert_eq!(
            graph
                .bind(
                    resources[limits().max_depth].clone(),
                    resources[limits().max_depth + 1].clone(),
                )
                .unwrap_err(),
            OwnerGraphError::CycleOrDepth
        );
    }

    #[test]
    fn owner_diagnostics_redact_body_digest_names_and_uids() {
        const NAME: &str = "owner-debug-sentinel";
        const UID: &str = "deadbeef-dead-4bad-8bad-deadbeef0008";
        const BODY: &str = "owner-body-debug-sentinel";
        const DIGEST: &str = "owner-digest-debug-sentinel";
        let desired = DesiredChild::new(
            ResourceRef::parse(&format!("Process/{NAME}")).unwrap(),
            BODY.as_bytes().to_vec(),
            DIGEST,
        )
        .unwrap();
        let observed = ObservedChild::new(
            HintTarget::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse(&format!("Process/{NAME}")).unwrap(),
                ResourceUid::parse(UID).unwrap(),
            ),
            ZoneRevision::new(3),
            DIGEST,
            false,
        )
        .unwrap();
        assert_eq!(desired.target().name().as_str(), NAME);
        assert_eq!(desired.canonical_resource(), BODY.as_bytes());
        assert_eq!(desired.payload_digest(), DIGEST);
        assert_eq!(observed.target().uid().as_str(), UID);
        assert_eq!(observed.payload_digest(), DIGEST);

        for debug in [format!("{desired:?}"), format!("{observed:?}")] {
            for sentinel in [NAME, UID, BODY, DIGEST] {
                assert!(!debug.contains(sentinel), "{debug}");
            }
        }
    }
}
