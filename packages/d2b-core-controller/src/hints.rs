//! Watch-plan validation, suppression, leases, and fair hint admission.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2b_contracts::v3::{
    ControllerGeneration, ObservedGeneration, ResourceGeneration, ResourceRef, ResourceTypeName,
    ResourceUid, ZoneId, ZoneRevision,
};

/// Exact watched object surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeField {
    Spec,
    Status,
    Metadata,
    Finalizers,
    Deletion,
}

/// One exact selector declared by a controller.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatchSelector {
    resource_type: ResourceTypeName,
    field: ChangeField,
    exact_value: Option<String>,
}

impl WatchSelector {
    /// Construct an exact or whole-field selector.
    pub fn new(
        resource_type: ResourceTypeName,
        field: ChangeField,
        exact_value: Option<String>,
    ) -> Result<Self, WatchPlanError> {
        if exact_value
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 256)
        {
            return Err(WatchPlanError::InvalidSelector);
        }
        Ok(Self {
            resource_type,
            field,
            exact_value,
        })
    }

    /// Borrow the selected ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }
}

impl core::fmt::Debug for WatchSelector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WatchSelector")
            .field("resource_type", &self.resource_type)
            .field("field", &self.field)
            .field("has_exact_value", &self.exact_value.is_some())
            .finish()
    }
}

/// Validated watch intent for one controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchPlan {
    owned_types: Vec<ResourceTypeName>,
    selectors: Vec<WatchSelector>,
    consumes_owner_triggers: bool,
}

impl WatchPlan {
    /// Validate bounded exact selectors against the owned type set.
    pub fn new(
        mut owned_types: Vec<ResourceTypeName>,
        mut selectors: Vec<WatchSelector>,
        consumes_owner_triggers: bool,
    ) -> Result<Self, WatchPlanError> {
        owned_types.sort();
        let original_type_count = owned_types.len();
        owned_types.dedup();
        selectors.sort();
        let original_selector_count = selectors.len();
        selectors.dedup();
        if owned_types.is_empty()
            || owned_types.len() != original_type_count
            || selectors.is_empty()
            || selectors.len() != original_selector_count
            || selectors.len() > 128
            || selectors
                .iter()
                .any(|selector| !owned_types.contains(&selector.resource_type))
        {
            return Err(WatchPlanError::InvalidPlan);
        }
        Ok(Self {
            owned_types,
            selectors,
            consumes_owner_triggers,
        })
    }

    /// Borrow controller-owned ResourceTypes.
    pub fn owned_types(&self) -> &[ResourceTypeName] {
        &self.owned_types
    }

    /// Whether child changes must trigger this controller.
    pub const fn consumes_owner_triggers(&self) -> bool {
        self.consumes_owner_triggers
    }
}

/// Zone-qualified controller identity used by leases and fair queues.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControllerLeaseKey {
    zone: ZoneId,
    controller_ref: ResourceRef,
}

impl ControllerLeaseKey {
    /// Construct a Zone-qualified controller key.
    pub fn new(zone: ZoneId, controller_ref: ResourceRef) -> Result<Self, WatchPlanError> {
        if controller_ref.resource_type().as_str() != "Process" {
            return Err(WatchPlanError::InvalidIdentity);
        }
        Ok(Self {
            zone,
            controller_ref,
        })
    }

    /// Borrow the registered Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the controller Process.
    pub const fn controller_ref(&self) -> &ResourceRef {
        &self.controller_ref
    }
}

impl core::fmt::Debug for ControllerLeaseKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllerLeaseKey")
            .field("controller_type", self.controller_ref.resource_type())
            .field("has_zone", &true)
            .finish()
    }
}

/// Registered controller identity and renewable generation lease.
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerBinding {
    key: ControllerLeaseKey,
    provider_ref: ResourceRef,
    generation: ControllerGeneration,
    lease_expires_at_tick: u64,
}

impl ControllerBinding {
    /// Construct a registered binding.
    pub fn new(
        zone: ZoneId,
        controller_ref: ResourceRef,
        provider_ref: ResourceRef,
        generation: ControllerGeneration,
        lease_expires_at_tick: u64,
    ) -> Result<Self, WatchPlanError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(WatchPlanError::InvalidIdentity);
        }
        if lease_expires_at_tick == 0 {
            return Err(WatchPlanError::InvalidLease);
        }
        Ok(Self {
            key: ControllerLeaseKey::new(zone, controller_ref)?,
            provider_ref,
            generation,
            lease_expires_at_tick,
        })
    }

    /// Borrow the controller reference.
    pub const fn controller_ref(&self) -> &ResourceRef {
        self.key.controller_ref()
    }

    /// Borrow the registered Zone.
    pub const fn zone(&self) -> &ZoneId {
        self.key.zone()
    }

    /// Borrow the Zone-qualified lease key.
    pub const fn key(&self) -> &ControllerLeaseKey {
        &self.key
    }

    /// Return the generation.
    pub const fn generation(&self) -> ControllerGeneration {
        self.generation
    }

    /// Return lease expiry.
    pub const fn lease_expires_at_tick(&self) -> u64 {
        self.lease_expires_at_tick
    }
}

impl core::fmt::Debug for ControllerBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllerBinding")
            .field("key", &self.key)
            .field("provider_type", self.provider_ref.resource_type())
            .field("generation", &self.generation)
            .field("lease_expires_at_tick", &self.lease_expires_at_tick)
            .finish()
    }
}

struct RegisteredController {
    binding: ControllerBinding,
    plan: WatchPlan,
}

/// ResourceType ownership and controller lease registry.
#[derive(Default)]
pub struct WatchRegistry {
    controllers: BTreeMap<ControllerLeaseKey, RegisteredController>,
    type_owners: BTreeMap<(ZoneId, ResourceTypeName), ControllerLeaseKey>,
}

impl WatchRegistry {
    /// Register one plan, rejecting duplicate type ownership.
    pub fn register(
        &mut self,
        binding: ControllerBinding,
        plan: WatchPlan,
        now_tick: u64,
    ) -> Result<(), WatchPlanError> {
        if binding.lease_expires_at_tick <= now_tick {
            return Err(WatchPlanError::ExpiredLease);
        }
        if self.controllers.contains_key(&binding.key)
            || plan.owned_types.iter().any(|resource_type| {
                self.type_owners
                    .contains_key(&(binding.key.zone.clone(), resource_type.clone()))
            })
        {
            return Err(WatchPlanError::OwnershipConflict);
        }
        for resource_type in &plan.owned_types {
            self.type_owners.insert(
                (binding.key.zone.clone(), resource_type.clone()),
                binding.key.clone(),
            );
        }
        self.controllers
            .insert(binding.key.clone(), RegisteredController { binding, plan });
        Ok(())
    }

    /// Resolve an active type owner.
    pub fn owner(
        &self,
        zone: &ZoneId,
        resource_type: &ResourceTypeName,
        now_tick: u64,
    ) -> Option<&ControllerBinding> {
        let controller_key = self
            .type_owners
            .get(&(zone.clone(), resource_type.clone()))?;
        let registered = self.controllers.get(controller_key)?;
        (registered.binding.lease_expires_at_tick > now_tick).then_some(&registered.binding)
    }

    /// Whether an active owner consumes child-change triggers.
    pub fn consumes_owner_triggers(
        &self,
        controller_key: &ControllerLeaseKey,
        now_tick: u64,
    ) -> bool {
        self.controllers
            .get(controller_key)
            .is_some_and(|registered| {
                registered.binding.lease_expires_at_tick > now_tick
                    && registered.plan.consumes_owner_triggers
            })
    }

    /// Renew only the exact registered generation.
    pub fn renew(
        &mut self,
        controller_key: &ControllerLeaseKey,
        generation: ControllerGeneration,
        lease_expires_at_tick: u64,
        now_tick: u64,
    ) -> Result<(), WatchPlanError> {
        let registered = self
            .controllers
            .get_mut(controller_key)
            .ok_or(WatchPlanError::UnknownController)?;
        if registered.binding.generation != generation
            || lease_expires_at_tick <= now_tick
            || lease_expires_at_tick <= registered.binding.lease_expires_at_tick
        {
            return Err(WatchPlanError::InvalidLease);
        }
        registered.binding.lease_expires_at_tick = lease_expires_at_tick;
        Ok(())
    }

    /// Withdraw expired registrations and all type ownership.
    pub fn withdraw_expired(&mut self, now_tick: u64) -> Vec<ControllerLeaseKey> {
        let expired: Vec<_> = self
            .controllers
            .iter()
            .filter_map(|(controller_key, registered)| {
                (registered.binding.lease_expires_at_tick <= now_tick)
                    .then_some((controller_key.clone(), registered.binding.generation))
            })
            .collect();
        for (controller_key, generation) in &expired {
            self.withdraw(controller_key, *generation);
        }
        expired.into_iter().map(|(key, _)| key).collect()
    }

    /// Withdraw one controller generation.
    pub fn withdraw(
        &mut self,
        controller_key: &ControllerLeaseKey,
        generation: ControllerGeneration,
    ) -> bool {
        let Some(registered) = self.controllers.get(controller_key) else {
            return false;
        };
        if registered.binding.generation != generation {
            return false;
        }
        let registered = self
            .controllers
            .remove(controller_key)
            .expect("generation was checked against this registration");
        for resource_type in registered.plan.owned_types {
            self.type_owners
                .remove(&(registered.binding.key.zone.clone(), resource_type));
        }
        true
    }
}

/// Invalid registration or watch plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchPlanError {
    InvalidSelector,
    InvalidPlan,
    InvalidIdentity,
    InvalidLease,
    ExpiredLease,
    OwnershipConflict,
    UnknownController,
}

impl core::fmt::Display for WatchPlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidSelector => "watch selector is empty or oversized",
            Self::InvalidPlan => "watch plan is empty, duplicated, broad, or out of ownership",
            Self::InvalidIdentity => "controller registration must bind a Process to a Provider",
            Self::InvalidLease => "controller lease generation or deadline is invalid",
            Self::ExpiredLease => "controller lease is already expired",
            Self::OwnershipConflict => "ResourceType already has a controller owner",
            Self::UnknownController => "controller is not registered",
        })
    }
}

impl std::error::Error for WatchPlanError {}

/// Closed reason set emitted by Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoreTriggerReason {
    SpecGenerationChanged,
    OwnedResourceChanged,
    DependencyChanged,
    DependencyReady,
    DeletionRequested,
    FinalizerRequired,
    ControllerGenerationChanged,
    ProviderGenerationChanged,
    PolicyChanged,
    SecurityPolicyChanged,
    ArtifactOrImageChanged,
    ExecutionStatusChanged,
    ScheduledObserve,
    AssessUpdateDue,
    UpgradeRequested,
    ExpeditedMutation,
    RetryDue,
    ManualReconcile,
    StartupRelist,
}

impl CoreTriggerReason {
    /// Whether suppression is forbidden for this reason.
    pub const fn prevents_suppression(self) -> bool {
        !matches!(
            self,
            Self::SpecGenerationChanged | Self::ExecutionStatusChanged
        )
    }
}

/// Immutable identity for hint coalescing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HintTarget {
    zone: ZoneId,
    resource_ref: ResourceRef,
    uid: ResourceUid,
}

impl HintTarget {
    /// Construct a target.
    pub fn new(zone: ZoneId, resource_ref: ResourceRef, uid: ResourceUid) -> Self {
        Self {
            zone,
            resource_ref,
            uid,
        }
    }

    /// Borrow the Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the resource reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the immutable UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }
}

impl core::fmt::Debug for HintTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HintTarget")
            .field("resource_type", self.resource_ref.resource_type())
            .field("has_zone", &true)
            .field("has_uid", &true)
            .finish()
    }
}

/// One durable change evaluated for suppression.
#[derive(Clone, PartialEq, Eq)]
pub struct ChangeRecord {
    pub target: HintTarget,
    pub revision: ZoneRevision,
    pub generation: ResourceGeneration,
    pub observed_generation: ObservedGeneration,
    pub fields: BTreeSet<ChangeField>,
    pub reasons: BTreeSet<CoreTriggerReason>,
    pub type_is_bound: bool,
    pub relevant_field_changed: bool,
    pub own_status_only: bool,
    pub owner_consumer_exists: bool,
    pub dependency_consumer_exists: bool,
    pub controller_generation_current: bool,
    pub conditions_require_work: bool,
    pub unknown_requires_observation: bool,
}

impl core::fmt::Debug for ChangeRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChangeRecord")
            .field("target", &self.target)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .field("observed_generation", &self.observed_generation)
            .field("fields", &self.fields)
            .field("reasons", &self.reasons)
            .field("type_is_bound", &self.type_is_bound)
            .field("relevant_field_changed", &self.relevant_field_changed)
            .field("own_status_only", &self.own_status_only)
            .field("owner_consumer_exists", &self.owner_consumer_exists)
            .field(
                "dependency_consumer_exists",
                &self.dependency_consumer_exists,
            )
            .field(
                "controller_generation_current",
                &self.controller_generation_current,
            )
            .field("conditions_require_work", &self.conditions_require_work)
            .field(
                "unknown_requires_observation",
                &self.unknown_requires_observation,
            )
            .finish()
    }
}

/// Explicit suppression decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionDecision {
    Dispatch,
    SuppressUnbound,
    SuppressIrrelevant,
    SuppressOwnConvergedStatus,
    SuppressConverged,
}

impl ChangeRecord {
    /// Apply the normative no-loss suppression rules.
    pub fn suppression(&self) -> SuppressionDecision {
        if !self.type_is_bound {
            return SuppressionDecision::SuppressUnbound;
        }
        if self
            .reasons
            .iter()
            .any(|reason| reason.prevents_suppression())
        {
            return SuppressionDecision::Dispatch;
        }
        if !self.relevant_field_changed {
            return SuppressionDecision::SuppressIrrelevant;
        }
        if self.own_status_only && !self.owner_consumer_exists && !self.dependency_consumer_exists {
            return SuppressionDecision::SuppressOwnConvergedStatus;
        }
        if self.own_status_only && (self.owner_consumer_exists || self.dependency_consumer_exists) {
            return SuppressionDecision::Dispatch;
        }
        if self.generation.get() == self.observed_generation.get()
            && self.controller_generation_current
            && !self.conditions_require_work
            && !self.unknown_requires_observation
        {
            return SuppressionDecision::SuppressConverged;
        }
        SuppressionDecision::Dispatch
    }
}

/// Coalesced controller hint.
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerHint {
    controller: ControllerLeaseKey,
    target: HintTarget,
    revision: ZoneRevision,
    reasons: BTreeSet<CoreTriggerReason>,
}

impl ControllerHint {
    /// Construct a nonempty hint.
    pub fn new(
        controller: ControllerLeaseKey,
        target: HintTarget,
        revision: ZoneRevision,
        reasons: BTreeSet<CoreTriggerReason>,
    ) -> Result<Self, HintAdmissionError> {
        if revision.get() == 0 || reasons.is_empty() || controller.zone != target.zone {
            return Err(HintAdmissionError::InvalidHint);
        }
        Ok(Self {
            controller,
            target,
            revision,
            reasons,
        })
    }

    /// Borrow the controller.
    pub const fn controller_ref(&self) -> &ResourceRef {
        &self.controller.controller_ref
    }

    /// Borrow the Zone-qualified controller key.
    pub const fn controller(&self) -> &ControllerLeaseKey {
        &self.controller
    }

    /// Borrow the target.
    pub const fn target(&self) -> &HintTarget {
        &self.target
    }

    /// Return high-water revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Borrow coalesced reasons.
    pub const fn reasons(&self) -> &BTreeSet<CoreTriggerReason> {
        &self.reasons
    }

    fn coalesce(&mut self, newer: Self) {
        self.revision = self.revision.max(newer.revision);
        self.reasons.extend(newer.reasons);
    }
}

impl core::fmt::Debug for ControllerHint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllerHint")
            .field("controller", &self.controller)
            .field("target", &self.target)
            .field("revision", &self.revision)
            .field("reasons", &self.reasons)
            .finish()
    }
}

#[derive(Default)]
struct ControllerQueue {
    hints: BTreeMap<HintTarget, ControllerHint>,
    order: VecDeque<HintTarget>,
    scheduled: bool,
}

/// Bounded round-robin admission across controller queues.
pub struct FairAdmission {
    max_total: usize,
    max_per_controller: usize,
    total: usize,
    queues: BTreeMap<ControllerLeaseKey, ControllerQueue>,
    controllers: VecDeque<ControllerLeaseKey>,
}

impl FairAdmission {
    /// Construct explicit fair queue bounds.
    pub fn new(max_total: usize, max_per_controller: usize) -> Self {
        Self {
            max_total,
            max_per_controller,
            total: 0,
            queues: BTreeMap::new(),
            controllers: VecDeque::new(),
        }
    }

    /// Admit or coalesce without cross-resource eviction.
    pub fn push(&mut self, hint: ControllerHint) -> Result<(), HintAdmissionError> {
        let controller = hint.controller.clone();
        let queue = self.queues.entry(controller.clone()).or_default();
        if let Some(existing) = queue.hints.get_mut(&hint.target) {
            existing.coalesce(hint);
            return Ok(());
        }
        if self.total >= self.max_total || queue.hints.len() >= self.max_per_controller {
            return Err(HintAdmissionError::Backpressure);
        }
        queue.order.push_back(hint.target.clone());
        queue.hints.insert(hint.target.clone(), hint);
        self.total += 1;
        if !queue.scheduled {
            queue.scheduled = true;
            self.controllers.push_back(controller);
        }
        Ok(())
    }

    /// Pop one controller turn, rotating nonempty queues.
    pub fn pop(&mut self) -> Option<ControllerHint> {
        while let Some(controller) = self.controllers.pop_front() {
            let queue = self
                .queues
                .get_mut(&controller)
                .expect("scheduled controller has a queue");
            let Some(target) = queue.order.pop_front() else {
                queue.scheduled = false;
                continue;
            };
            let hint = queue
                .hints
                .remove(&target)
                .expect("controller order and hints agree");
            self.total -= 1;
            if queue.order.is_empty() {
                queue.scheduled = false;
            } else {
                self.controllers.push_back(controller);
            }
            return Some(hint);
        }
        None
    }

    /// Withdraw every pending hint for an expired controller lease.
    pub fn withdraw_controller(&mut self, controller: &ControllerLeaseKey) -> usize {
        let Some(queue) = self.queues.remove(controller) else {
            return 0;
        };
        let removed = queue.hints.len();
        self.total -= removed;
        self.controllers.retain(|queued| queued != controller);
        removed
    }

    /// Rebuild startup hints from one authoritative relist.
    pub fn rebuild_controller(
        &mut self,
        controller: &ControllerLeaseKey,
        resources: Vec<(HintTarget, ZoneRevision)>,
    ) -> Result<(), HintAdmissionError> {
        let mut seen = BTreeSet::new();
        let mut replacement = Vec::with_capacity(resources.len());
        for (target, revision) in resources {
            if target.zone != controller.zone || revision.get() == 0 || !seen.insert(target.clone())
            {
                return Err(HintAdmissionError::InvalidHint);
            }
            replacement.push(ControllerHint::new(
                controller.clone(),
                target,
                revision,
                BTreeSet::from([CoreTriggerReason::StartupRelist]),
            )?);
        }
        let current = self
            .queues
            .get(controller)
            .map_or(0, |queue| queue.hints.len());
        if replacement.len() > self.max_per_controller
            || self.total - current + replacement.len() > self.max_total
        {
            return Err(HintAdmissionError::Backpressure);
        }
        self.withdraw_controller(controller);
        for hint in replacement {
            self.push(hint)
                .expect("validated replacement fits the reserved queue capacity");
        }
        Ok(())
    }

    /// Number of pending resource identities.
    pub const fn len(&self) -> usize {
        self.total
    }

    /// Whether no admitted resource is pending.
    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// Hint admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintAdmissionError {
    InvalidHint,
    Backpressure,
}

impl core::fmt::Display for HintAdmissionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidHint => "controller hint requires revision and reasons",
            Self::Backpressure => "fair controller queue bound reached",
        })
    }
}

impl std::error::Error for HintAdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Process/{name}")).unwrap()
    }

    fn controller_key(name: &str) -> ControllerLeaseKey {
        ControllerLeaseKey::new(ZoneId::parse("work").unwrap(), controller(name)).unwrap()
    }

    fn target(name: &str, suffix: u8) -> HintTarget {
        HintTarget::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(&format!("Guest/{name}")).unwrap(),
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-4266141740{suffix:02}")).unwrap(),
        )
    }

    fn plan(resource_type: &str) -> WatchPlan {
        let resource_type = ResourceTypeName::parse(resource_type).unwrap();
        WatchPlan::new(
            vec![resource_type.clone()],
            vec![WatchSelector::new(resource_type, ChangeField::Spec, None).unwrap()],
            true,
        )
        .unwrap()
    }

    fn binding(name: &str, expires: u64) -> ControllerBinding {
        ControllerBinding::new(
            ZoneId::parse("work").unwrap(),
            controller(name),
            ResourceRef::parse("Provider/system-core").unwrap(),
            ControllerGeneration::new(1).unwrap(),
            expires,
        )
        .unwrap()
    }

    fn hint(controller_name: &str, target: HintTarget, revision: u64) -> ControllerHint {
        ControllerHint::new(
            controller_key(controller_name),
            target,
            ZoneRevision::new(revision),
            BTreeSet::from([CoreTriggerReason::DependencyChanged]),
        )
        .unwrap()
    }

    fn change(reasons: BTreeSet<CoreTriggerReason>) -> ChangeRecord {
        ChangeRecord {
            target: target("app", 1),
            revision: ZoneRevision::new(4),
            generation: ResourceGeneration::new(2).unwrap(),
            observed_generation: ObservedGeneration::new(2),
            fields: BTreeSet::from([ChangeField::Status]),
            reasons,
            type_is_bound: true,
            relevant_field_changed: true,
            own_status_only: true,
            owner_consumer_exists: false,
            dependency_consumer_exists: false,
            controller_generation_current: true,
            conditions_require_work: false,
            unknown_requires_observation: false,
        }
    }

    #[test]
    fn watch_plan_rejects_selector_outside_owned_types() {
        let result = WatchPlan::new(
            vec![ResourceTypeName::parse("Guest").unwrap()],
            vec![
                WatchSelector::new(
                    ResourceTypeName::parse("Process").unwrap(),
                    ChangeField::Spec,
                    None,
                )
                .unwrap(),
            ],
            false,
        );
        assert_eq!(result.unwrap_err(), WatchPlanError::InvalidPlan);
    }

    #[test]
    fn one_zone_type_has_exactly_one_live_owner() {
        let mut registry = WatchRegistry::default();
        registry
            .register(binding("one", 10), plan("Guest"), 1)
            .unwrap();
        assert_eq!(
            registry
                .register(binding("two", 10), plan("Guest"), 1)
                .unwrap_err(),
            WatchPlanError::OwnershipConflict
        );
        assert_eq!(
            registry
                .owner(
                    &ZoneId::parse("work").unwrap(),
                    &ResourceTypeName::parse("Guest").unwrap(),
                    2,
                )
                .unwrap()
                .controller_ref(),
            &controller("one")
        );
    }

    #[test]
    fn equal_controller_names_in_different_zones_never_alias() {
        let mut registry = WatchRegistry::default();
        for zone in ["work", "personal"] {
            registry
                .register(
                    ControllerBinding::new(
                        ZoneId::parse(zone).unwrap(),
                        controller("shared"),
                        ResourceRef::parse("Provider/system-core").unwrap(),
                        ControllerGeneration::new(1).unwrap(),
                        10,
                    )
                    .unwrap(),
                    plan("Guest"),
                    1,
                )
                .unwrap();
        }

        for zone in ["work", "personal"] {
            assert_eq!(
                registry
                    .owner(
                        &ZoneId::parse(zone).unwrap(),
                        &ResourceTypeName::parse("Guest").unwrap(),
                        2,
                    )
                    .unwrap()
                    .zone()
                    .as_str(),
                zone
            );
        }
    }

    #[test]
    fn lease_withdrawal_removes_type_owner_and_pending_hints() {
        let mut registry = WatchRegistry::default();
        registry
            .register(binding("one", 5), plan("Guest"), 1)
            .unwrap();
        let mut admission = FairAdmission::new(4, 4);
        admission.push(hint("one", target("app", 1), 2)).unwrap();

        let expired = registry.withdraw_expired(5);
        assert_eq!(expired, vec![controller_key("one")]);
        assert_eq!(admission.withdraw_controller(&expired[0]), 1);
        assert!(admission.is_empty());
        assert!(
            registry
                .owner(
                    &ZoneId::parse("work").unwrap(),
                    &ResourceTypeName::parse("Guest").unwrap(),
                    5,
                )
                .is_none()
        );
    }

    #[test]
    fn stale_generation_cannot_withdraw_a_live_controller_lease() {
        let mut registry = WatchRegistry::default();
        registry
            .register(binding("one", 10), plan("Guest"), 1)
            .unwrap();

        assert!(!registry.withdraw(
            &controller_key("one"),
            ControllerGeneration::new(2).unwrap(),
        ));
        assert!(
            registry
                .owner(
                    &ZoneId::parse("work").unwrap(),
                    &ResourceTypeName::parse("Guest").unwrap(),
                    2,
                )
                .is_some()
        );
    }

    #[test]
    fn protected_causes_are_never_suppressed() {
        for reason in [
            CoreTriggerReason::OwnedResourceChanged,
            CoreTriggerReason::DeletionRequested,
            CoreTriggerReason::FinalizerRequired,
            CoreTriggerReason::PolicyChanged,
            CoreTriggerReason::ProviderGenerationChanged,
            CoreTriggerReason::ScheduledObserve,
            CoreTriggerReason::RetryDue,
        ] {
            assert_eq!(
                change(BTreeSet::from([reason])).suppression(),
                SuppressionDecision::Dispatch,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn converged_self_status_is_suppressed_only_without_consumers() {
        let mut record = change(BTreeSet::from([CoreTriggerReason::ExecutionStatusChanged]));
        assert_eq!(
            record.suppression(),
            SuppressionDecision::SuppressOwnConvergedStatus
        );
        record.owner_consumer_exists = true;
        assert_eq!(record.suppression(), SuppressionDecision::Dispatch);
    }

    #[test]
    fn same_resource_coalesces_without_losing_reasons() {
        let mut admission = FairAdmission::new(4, 4);
        let target = target("app", 1);
        admission.push(hint("one", target.clone(), 2)).unwrap();
        admission
            .push(
                ControllerHint::new(
                    controller_key("one"),
                    target,
                    ZoneRevision::new(7),
                    BTreeSet::from([CoreTriggerReason::DeletionRequested]),
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(admission.len(), 1);
        let hint = admission.pop().unwrap();
        assert_eq!(hint.revision(), ZoneRevision::new(7));
        assert!(
            hint.reasons()
                .contains(&CoreTriggerReason::DependencyChanged)
        );
        assert!(
            hint.reasons()
                .contains(&CoreTriggerReason::DeletionRequested)
        );
    }

    #[test]
    fn round_robin_prevents_busy_controller_starvation() {
        let mut admission = FairAdmission::new(8, 8);
        admission.push(hint("one", target("one-a", 1), 2)).unwrap();
        admission.push(hint("one", target("one-b", 2), 2)).unwrap();
        admission.push(hint("two", target("two-a", 3), 2)).unwrap();

        assert_eq!(
            admission.pop().unwrap().controller_ref(),
            &controller("one")
        );
        assert_eq!(
            admission.pop().unwrap().controller_ref(),
            &controller("two")
        );
        assert_eq!(
            admission.pop().unwrap().controller_ref(),
            &controller("one")
        );
    }

    #[test]
    fn full_queue_returns_backpressure_without_eviction() {
        let mut admission = FairAdmission::new(1, 1);
        let first = target("first", 1);
        admission.push(hint("one", first.clone(), 2)).unwrap();
        assert_eq!(
            admission
                .push(hint("two", target("second", 2), 2))
                .unwrap_err(),
            HintAdmissionError::Backpressure
        );
        assert_eq!(admission.pop().unwrap().target(), &first);
    }

    #[test]
    fn cross_zone_hint_is_rejected_before_admission() {
        let foreign = HintTarget::new(
            ZoneId::parse("personal").unwrap(),
            ResourceRef::parse("Guest/app").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174009").unwrap(),
        );
        assert_eq!(
            ControllerHint::new(
                controller_key("one"),
                foreign,
                ZoneRevision::new(2),
                BTreeSet::from([CoreTriggerReason::ManualReconcile]),
            )
            .unwrap_err(),
            HintAdmissionError::InvalidHint
        );
    }

    #[test]
    fn startup_relist_replaces_stale_controller_queue() {
        let mut admission = FairAdmission::new(4, 4);
        admission.push(hint("one", target("stale", 1), 2)).unwrap();
        let fresh = target("fresh", 2);
        admission
            .rebuild_controller(
                &controller_key("one"),
                vec![(fresh.clone(), ZoneRevision::new(8))],
            )
            .unwrap();

        let hint = admission.pop().unwrap();
        assert_eq!(hint.target(), &fresh);
        assert_eq!(
            hint.reasons(),
            &BTreeSet::from([CoreTriggerReason::StartupRelist])
        );
    }

    #[test]
    fn rejected_startup_relist_preserves_the_previous_queue_atomically() {
        let mut admission = FairAdmission::new(1, 1);
        let stale = target("stale", 1);
        admission.push(hint("one", stale.clone(), 2)).unwrap();

        assert_eq!(
            admission
                .rebuild_controller(
                    &controller_key("one"),
                    vec![
                        (target("fresh-a", 2), ZoneRevision::new(8)),
                        (target("fresh-b", 3), ZoneRevision::new(8)),
                    ],
                )
                .unwrap_err(),
            HintAdmissionError::Backpressure
        );
        assert_eq!(admission.pop().unwrap().target(), &stale);
    }
}
