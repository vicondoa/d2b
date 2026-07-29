//! Explicit dependency reverse indexes and disruptive-upgrade ordering.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2b_contracts::v3::{ResourceRef, ZoneRevision};

use crate::hints::{ControllerLeaseKey, CoreTriggerReason, HintTarget};

const MAX_DEPENDENCIES_PER_RESOURCE: usize = 64;
const MAX_UPGRADE_RESOURCES: usize = 192;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DependentBinding {
    controller: ControllerLeaseKey,
    target: HintTarget,
}

/// One dependency-driven controller trigger.
#[derive(Clone, PartialEq, Eq)]
pub struct DependencyTrigger {
    controller: ControllerLeaseKey,
    target: HintTarget,
    dependency: HintTarget,
    revision: ZoneRevision,
    reason: CoreTriggerReason,
}

impl DependencyTrigger {
    /// Borrow the destination controller.
    pub const fn controller_ref(&self) -> &ResourceRef {
        self.controller.controller_ref()
    }

    /// Borrow the dependent target.
    pub const fn target(&self) -> &HintTarget {
        &self.target
    }

    /// Return the high-water dependency revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the dependency cause.
    pub const fn reason(&self) -> CoreTriggerReason {
        self.reason
    }
}

impl core::fmt::Debug for DependencyTrigger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DependencyTrigger")
            .field("controller", &self.controller)
            .field("target", &self.target)
            .field("dependency", &self.dependency)
            .field("revision", &self.revision)
            .field("reason", &self.reason)
            .finish()
    }
}

/// Ordered drain/recycle/restart projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeOrder {
    drain: Vec<HintTarget>,
    recycle: HintTarget,
    restart: Vec<HintTarget>,
}

impl UpgradeOrder {
    /// Dependents drain farthest-first.
    pub fn drain(&self) -> &[HintTarget] {
        &self.drain
    }

    /// Borrow the resource whose realization is recycled.
    pub const fn recycle(&self) -> &HintTarget {
        &self.recycle
    }

    /// Dependents restart nearest-first.
    pub fn restart(&self) -> &[HintTarget] {
        &self.restart
    }
}

/// Zone-local reverse dependency index.
#[derive(Clone, Default)]
pub struct DependencyIndex {
    reverse: BTreeMap<HintTarget, BTreeSet<DependentBinding>>,
    dependencies: BTreeMap<HintTarget, BTreeSet<HintTarget>>,
    target_controllers: BTreeMap<HintTarget, ControllerLeaseKey>,
    targets_by_controller: BTreeMap<ControllerLeaseKey, BTreeSet<HintTarget>>,
}

impl DependencyIndex {
    /// Replace exact dependencies for one target.
    pub fn register(
        &mut self,
        controller: ControllerLeaseKey,
        target: HintTarget,
        dependencies: BTreeSet<HintTarget>,
    ) -> Result<(), DependencyError> {
        if dependencies.len() > MAX_DEPENDENCIES_PER_RESOURCE
            || controller.zone() != target.zone()
            || dependencies.contains(&target)
            || dependencies
                .iter()
                .any(|dependency| dependency.zone() != target.zone())
        {
            return Err(DependencyError::InvalidDependency);
        }
        if self
            .target_controllers
            .get(&target)
            .is_some_and(|owner| owner != &controller)
        {
            return Err(DependencyError::OwnershipConflict);
        }
        let mut staged = self.clone();
        staged.remove_target(&target);
        for dependency in &dependencies {
            staged
                .reverse
                .entry(dependency.clone())
                .or_default()
                .insert(DependentBinding {
                    controller: controller.clone(),
                    target: target.clone(),
                });
        }
        staged.dependencies.insert(target.clone(), dependencies);
        staged
            .target_controllers
            .insert(target.clone(), controller.clone());
        staged
            .targets_by_controller
            .entry(controller)
            .or_default()
            .insert(target);
        if staged.has_cycle() {
            return Err(DependencyError::Cycle);
        }
        *self = staged;
        Ok(())
    }

    /// Emit every exact dependent after a durable dependency mutation.
    pub fn triggers(
        &self,
        dependency: &HintTarget,
        revision: ZoneRevision,
        ready: bool,
    ) -> Result<Vec<DependencyTrigger>, DependencyError> {
        if revision.get() == 0 {
            return Err(DependencyError::InvalidRevision);
        }
        Ok(self
            .reverse
            .get(dependency)
            .into_iter()
            .flatten()
            .map(|binding| DependencyTrigger {
                controller: binding.controller.clone(),
                target: binding.target.clone(),
                dependency: dependency.clone(),
                revision,
                reason: if ready {
                    CoreTriggerReason::DependencyReady
                } else {
                    CoreTriggerReason::DependencyChanged
                },
            })
            .collect())
    }

    /// Return active dependents that block a disruptive recycle.
    pub fn blockers(&self, dependency: &HintTarget) -> Vec<HintTarget> {
        self.reverse
            .get(dependency)
            .into_iter()
            .flatten()
            .map(|binding| binding.target.clone())
            .collect()
    }

    /// Build a topological drain/recycle/restart order.
    pub fn upgrade_order(&self, root: &HintTarget) -> Result<UpgradeOrder, DependencyError> {
        let mut depth = BTreeMap::from([(root.clone(), 0_usize)]);
        let mut pending = VecDeque::from([root.clone()]);
        while let Some(resource) = pending.pop_front() {
            let current_depth = depth[&resource];
            for dependent in self.blockers(&resource) {
                let next_depth = current_depth + 1;
                match depth.get(&dependent) {
                    Some(existing) if *existing >= next_depth => {}
                    _ => {
                        depth.insert(dependent.clone(), next_depth);
                        pending.push_back(dependent);
                    }
                }
                if depth.len() > MAX_UPGRADE_RESOURCES {
                    return Err(DependencyError::UpgradeSetTooLarge);
                }
            }
        }
        depth.remove(root);
        let mut ordered: Vec<_> = depth.into_iter().collect();
        ordered.sort_by(|(left_target, left_depth), (right_target, right_depth)| {
            left_depth
                .cmp(right_depth)
                .then_with(|| left_target.cmp(right_target))
        });
        let restart = ordered
            .iter()
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>();
        let drain = ordered
            .into_iter()
            .rev()
            .map(|(target, _)| target)
            .collect();
        Ok(UpgradeOrder {
            drain,
            recycle: root.clone(),
            restart,
        })
    }

    /// Remove every edge owned by a withdrawn controller lease.
    pub fn withdraw_controller(&mut self, controller: &ControllerLeaseKey) -> usize {
        let Some(targets) = self.targets_by_controller.get(controller).cloned() else {
            return 0;
        };
        let mut removed = 0;
        for target in targets {
            removed += self.remove_target(&target);
        }
        removed
    }

    fn remove_target(&mut self, target: &HintTarget) -> usize {
        let Some(previous) = self.dependencies.remove(target) else {
            return 0;
        };
        let removed = previous.len();
        for dependency in &previous {
            if let Some(bindings) = self.reverse.get_mut(dependency) {
                bindings.retain(|binding| binding.target != *target);
                if bindings.is_empty() {
                    self.reverse.remove(dependency);
                }
            }
        }
        if let Some(controller) = self.target_controllers.remove(target)
            && let Some(targets) = self.targets_by_controller.get_mut(&controller)
        {
            targets.remove(target);
            if targets.is_empty() {
                self.targets_by_controller.remove(&controller);
            }
        }
        removed
    }

    fn has_cycle(&self) -> bool {
        fn visit(
            node: &HintTarget,
            graph: &BTreeMap<HintTarget, BTreeSet<HintTarget>>,
            visiting: &mut BTreeSet<HintTarget>,
            visited: &mut BTreeSet<HintTarget>,
        ) -> bool {
            if visited.contains(node) {
                return false;
            }
            if !visiting.insert(node.clone()) {
                return true;
            }
            if graph.get(node).is_some_and(|dependencies| {
                dependencies
                    .iter()
                    .any(|dependency| visit(dependency, graph, visiting, visited))
            }) {
                return true;
            }
            visiting.remove(node);
            visited.insert(node.clone());
            false
        }

        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        self.dependencies
            .keys()
            .any(|node| visit(node, &self.dependencies, &mut visiting, &mut visited))
    }
}

/// Invalid dependency graph operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyError {
    InvalidDependency,
    InvalidRevision,
    OwnershipConflict,
    Cycle,
    UpgradeSetTooLarge,
}

impl core::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidDependency => "dependency is self, cross-Zone, or over the bound",
            Self::InvalidRevision => "dependency trigger requires a durable revision",
            Self::OwnershipConflict => "dependent target belongs to another controller lease",
            Self::Cycle => "dependency graph must be acyclic",
            Self::UpgradeSetTooLarge => "dependency upgrade set exceeds its bound",
        })
    }
}

impl std::error::Error for DependencyError {}

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

    fn controller(name: &str) -> ControllerLeaseKey {
        ControllerLeaseKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(&format!("Process/{name}")).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn dependency_change_reaches_every_exact_dependent_without_loss() {
        let network = target("work", "Network", "lan", 1);
        let guest = target("work", "Guest", "app", 2);
        let process = target("work", "Process", "worker", 3);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("guest-controller"),
                guest.clone(),
                BTreeSet::from([network.clone()]),
            )
            .unwrap();
        index
            .register(
                controller("process-controller"),
                process.clone(),
                BTreeSet::from([network.clone()]),
            )
            .unwrap();

        let triggers = index
            .triggers(&network, ZoneRevision::new(9), false)
            .unwrap();
        assert_eq!(triggers.len(), 2);
        assert_eq!(triggers[0].target(), &guest);
        assert_eq!(triggers[1].target(), &process);
        assert!(
            triggers
                .iter()
                .all(|trigger| trigger.reason() == CoreTriggerReason::DependencyChanged)
        );
    }

    #[test]
    fn dependency_ready_uses_distinct_non_droppable_reason() {
        let volume = target("work", "Volume", "root", 1);
        let guest = target("work", "Guest", "app", 2);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("guest-controller"),
                guest,
                BTreeSet::from([volume.clone()]),
            )
            .unwrap();
        assert_eq!(
            index.triggers(&volume, ZoneRevision::new(5), true).unwrap()[0].reason(),
            CoreTriggerReason::DependencyReady
        );
    }

    #[test]
    fn cross_zone_dependency_is_rejected_before_indexing() {
        let mut index = DependencyIndex::default();
        assert_eq!(
            index
                .register(
                    controller("guest-controller"),
                    target("work", "Guest", "app", 1),
                    BTreeSet::from([target("personal", "Volume", "root", 2)]),
                )
                .unwrap_err(),
            DependencyError::InvalidDependency
        );
    }

    #[test]
    fn cycle_is_rejected_without_installing_attempted_edges() {
        let one = target("work", "Process", "one", 1);
        let two = target("work", "Process", "two", 2);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("one-controller"),
                one.clone(),
                BTreeSet::from([two.clone()]),
            )
            .unwrap();
        assert_eq!(
            index
                .register(
                    controller("two-controller"),
                    two.clone(),
                    BTreeSet::from([one.clone()]),
                )
                .unwrap_err(),
            DependencyError::Cycle
        );
        assert!(
            index
                .triggers(&one, ZoneRevision::new(3), false)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejected_cycle_preserves_existing_edges_for_the_same_controller() {
        let one = target("work", "Process", "one", 1);
        let two = target("work", "Process", "two", 2);
        let three = target("work", "Process", "three", 3);
        let owner = controller("shared-controller");
        let mut index = DependencyIndex::default();
        index
            .register(owner.clone(), one.clone(), BTreeSet::from([two.clone()]))
            .unwrap();
        index
            .register(owner.clone(), three.clone(), BTreeSet::from([one.clone()]))
            .unwrap();

        assert_eq!(
            index
                .register(owner, two.clone(), BTreeSet::from([three.clone()]))
                .unwrap_err(),
            DependencyError::Cycle
        );
        assert_eq!(
            index.triggers(&two, ZoneRevision::new(4), false).unwrap()[0].target(),
            &one
        );
        assert_eq!(
            index.triggers(&one, ZoneRevision::new(4), false).unwrap()[0].target(),
            &three
        );
    }

    #[test]
    fn another_controller_cannot_replace_a_targets_dependency_edges() {
        let network = target("work", "Network", "lan", 1);
        let volume = target("work", "Volume", "root", 2);
        let guest = target("work", "Guest", "app", 3);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("first-controller"),
                guest.clone(),
                BTreeSet::from([network.clone()]),
            )
            .unwrap();

        assert_eq!(
            index
                .register(
                    controller("second-controller"),
                    guest.clone(),
                    BTreeSet::from([volume]),
                )
                .unwrap_err(),
            DependencyError::OwnershipConflict
        );
        assert_eq!(
            index
                .triggers(&network, ZoneRevision::new(2), false)
                .unwrap()[0]
                .target(),
            &guest
        );
    }

    #[test]
    fn dependency_triggers_require_a_durable_revision() {
        let index = DependencyIndex::default();
        assert_eq!(
            index
                .triggers(
                    &target("work", "Network", "lan", 1),
                    ZoneRevision::new(0),
                    false,
                )
                .unwrap_err(),
            DependencyError::InvalidRevision
        );
    }

    #[test]
    fn gpu_upgrade_drains_dependents_then_recycles_then_restarts() {
        let gpu = target("work", "Device", "gpu", 1);
        let guest = target("work", "Guest", "desktop", 2);
        let process = target("work", "Process", "compositor", 3);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("guest-controller"),
                guest.clone(),
                BTreeSet::from([gpu.clone()]),
            )
            .unwrap();
        index
            .register(
                controller("process-controller"),
                process.clone(),
                BTreeSet::from([guest.clone()]),
            )
            .unwrap();

        assert_eq!(index.blockers(&gpu), vec![guest.clone()]);
        let order = index.upgrade_order(&gpu).unwrap();
        assert_eq!(order.drain(), &[process.clone(), guest.clone()]);
        assert_eq!(order.recycle(), &gpu);
        assert_eq!(order.restart(), &[guest, process]);
    }

    #[test]
    fn lease_withdrawal_removes_only_its_dependency_edges() {
        let network = target("work", "Network", "lan", 1);
        let guest = target("work", "Guest", "app", 2);
        let process = target("work", "Process", "worker", 3);
        let mut index = DependencyIndex::default();
        index
            .register(
                controller("guest-controller"),
                guest,
                BTreeSet::from([network.clone()]),
            )
            .unwrap();
        index
            .register(
                controller("process-controller"),
                process.clone(),
                BTreeSet::from([network.clone()]),
            )
            .unwrap();

        assert_eq!(
            index.withdraw_controller(&controller("guest-controller")),
            1
        );
        let triggers = index
            .triggers(&network, ZoneRevision::new(4), false)
            .unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].target(), &process);
    }
}
