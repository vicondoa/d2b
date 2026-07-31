//! Hierarchical execution budget reservation and emergency policy.

use std::collections::BTreeMap;

use d2b_contracts::v3::ResourceRef;

/// Closed budget dimensions used for policy and metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BudgetDimension {
    MilliCpu,
    MemoryBytes,
    Pids,
    FileDescriptors,
    NetworkEgressBytesPerSecond,
    Threads,
}

impl BudgetDimension {
    /// Return the fixed metric label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MilliCpu => "millicpu",
            Self::MemoryBytes => "memory-bytes",
            Self::Pids => "pids",
            Self::FileDescriptors => "file-descriptors",
            Self::NetworkEgressBytesPerSecond => "network-egress-bytes-per-second",
            Self::Threads => "threads",
        }
    }
}

/// Cardinality-safe label keys for budget metrics.
pub const BUDGET_METRIC_LABEL_KEYS: &[&str] = &["dimension", "outcome"];

/// One bounded capacity or reservation vector.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct BudgetVector(BTreeMap<BudgetDimension, u64>);

impl core::fmt::Debug for BudgetVector {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BudgetVector")
            .field("dimension_count", &self.0.len())
            .finish()
    }
}

impl BudgetVector {
    /// Construct a budget vector, rejecting zero-valued dimensions.
    pub fn new(
        values: impl IntoIterator<Item = (BudgetDimension, u64)>,
    ) -> Result<Self, BudgetError> {
        let values = values.into_iter().collect::<Vec<_>>();
        let original_len = values.len();
        let values = values.into_iter().collect::<BTreeMap<_, _>>();
        if values.len() != original_len || values.values().any(|value| *value == 0) {
            return Err(BudgetError::InvalidBudget);
        }
        Ok(Self(values))
    }

    /// Return one dimension or zero when unconstrained by this vector.
    pub fn get(&self, dimension: BudgetDimension) -> u64 {
        self.0.get(&dimension).copied().unwrap_or(0)
    }

    fn checked_add(&self, other: &Self) -> Result<Self, BudgetError> {
        let mut combined = self.0.clone();
        for (dimension, amount) in &other.0 {
            let entry = combined.entry(*dimension).or_default();
            *entry = entry
                .checked_add(*amount)
                .ok_or(BudgetError::ArithmeticOverflow)?;
        }
        Ok(Self(combined))
    }

    fn fits_within(&self, capacity: &Self) -> bool {
        self.0.iter().all(|(dimension, amount)| {
            capacity
                .0
                .get(dimension)
                .is_some_and(|limit| amount <= limit)
        })
    }
}

/// Emergency policy scope derived from trusted policy, never caller claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyScope {
    Provider,
    Host,
    Guest,
    Zone,
    Global,
}

/// Normal-resource actions required by emergency disable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyAction {
    RevokeRoutes,
    RevokeSessions,
    RevokeGrants,
    StopChildProcesses,
    PreserveIncidentHeldState,
}

/// Closed budget refusal reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    InvalidBudget,
    InvalidExecutionTarget,
    DuplicateReservation,
    Overcommit,
    ArithmeticOverflow,
    UnknownReservation,
    ChildCapacityExceeded,
}

impl BudgetError {
    /// Return a stable identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidBudget => "budget-invalid",
            Self::InvalidExecutionTarget => "budget-execution-target-invalid",
            Self::DuplicateReservation => "budget-reservation-duplicate",
            Self::Overcommit => "budget-overcommit",
            Self::ArithmeticOverflow => "budget-arithmetic-overflow",
            Self::UnknownReservation => "budget-reservation-unknown",
            Self::ChildCapacityExceeded => "budget-child-capacity-exceeded",
        }
    }
}

impl core::fmt::Display for BudgetError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for BudgetError {}

/// Pure aggregate reservation handler for one execution target.
pub struct BudgetHandler {
    execution_ref: ResourceRef,
    capacity: BudgetVector,
    reservations: BTreeMap<ResourceRef, BudgetVector>,
}

impl core::fmt::Debug for BudgetHandler {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BudgetHandler")
            .field("has_execution_target", &true)
            .field("reservation_count", &self.reservations.len())
            .finish_non_exhaustive()
    }
}

impl BudgetHandler {
    /// Bind capacity to one Host or Guest execution target.
    pub fn new(execution_ref: ResourceRef, capacity: BudgetVector) -> Result<Self, BudgetError> {
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(BudgetError::InvalidExecutionTarget);
        }
        Ok(Self {
            execution_ref,
            capacity,
            reservations: BTreeMap::new(),
        })
    }

    /// Reserve one exact Process budget without overcommit.
    pub fn reserve(
        &mut self,
        process_ref: ResourceRef,
        reservation: BudgetVector,
    ) -> Result<(), BudgetError> {
        if process_ref.resource_type().as_str() != "Process" {
            return Err(BudgetError::InvalidExecutionTarget);
        }
        if self.reservations.contains_key(&process_ref) {
            return Err(BudgetError::DuplicateReservation);
        }
        let projected = self.total()?.checked_add(&reservation)?;
        if !projected.fits_within(&self.capacity) {
            return Err(BudgetError::Overcommit);
        }
        self.reservations.insert(process_ref, reservation);
        Ok(())
    }

    /// Release one exact Process reservation.
    pub fn release(&mut self, process_ref: &ResourceRef) -> Result<(), BudgetError> {
        self.reservations
            .remove(process_ref)
            .map(|_| ())
            .ok_or(BudgetError::UnknownReservation)
    }

    /// Aggregate all current reservations.
    pub fn total(&self) -> Result<BudgetVector, BudgetError> {
        self.reservations
            .values()
            .try_fold(BudgetVector::default(), |total, item| {
                total.checked_add(item)
            })
    }

    /// Validate a child Zone capacity as a narrowing of remaining capacity.
    pub fn validate_child_capacity(&self, child: &BudgetVector) -> Result<(), BudgetError> {
        let used = self.total()?;
        let projected = used.checked_add(child)?;
        if projected.fits_within(&self.capacity) {
            Ok(())
        } else {
            Err(BudgetError::ChildCapacityExceeded)
        }
    }

    /// Return normal-resource actions for any trusted emergency scope.
    pub fn emergency_actions(_scope: EmergencyScope) -> &'static [EmergencyAction] {
        &[
            EmergencyAction::RevokeRoutes,
            EmergencyAction::RevokeSessions,
            EmergencyAction::RevokeGrants,
            EmergencyAction::StopChildProcesses,
            EmergencyAction::PreserveIncidentHeldState,
        ]
    }

    /// Borrow the execution target for authorized resource mutations.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(cpu: u64, memory: u64) -> BudgetVector {
        BudgetVector::new([
            (BudgetDimension::MilliCpu, cpu),
            (BudgetDimension::MemoryBytes, memory),
        ])
        .unwrap()
    }

    #[test]
    fn reservations_aggregate_within_capacity() {
        let mut handler = BudgetHandler::new(
            ResourceRef::parse("Host/system").unwrap(),
            vector(2_000, 4_096),
        )
        .unwrap();
        handler
            .reserve(ResourceRef::parse("Process/one").unwrap(), vector(500, 512))
            .unwrap();
        handler
            .reserve(
                ResourceRef::parse("Process/two").unwrap(),
                vector(700, 1_024),
            )
            .unwrap();
        assert_eq!(
            handler.total().unwrap().get(BudgetDimension::MilliCpu),
            1_200
        );
    }

    #[test]
    fn overcommit_is_rejected_without_changing_reservations() {
        let mut handler = BudgetHandler::new(
            ResourceRef::parse("Guest/app").unwrap(),
            vector(1_000, 1_024),
        )
        .unwrap();
        handler
            .reserve(ResourceRef::parse("Process/one").unwrap(), vector(800, 512))
            .unwrap();
        assert_eq!(
            handler.reserve(ResourceRef::parse("Process/two").unwrap(), vector(300, 512)),
            Err(BudgetError::Overcommit)
        );
        assert_eq!(handler.total().unwrap().get(BudgetDimension::MilliCpu), 800);
    }

    #[test]
    fn child_zone_capacity_can_only_narrow_remaining_capacity() {
        let mut handler = BudgetHandler::new(
            ResourceRef::parse("Host/system").unwrap(),
            vector(1_000, 1_024),
        )
        .unwrap();
        handler
            .reserve(ResourceRef::parse("Process/one").unwrap(), vector(600, 512))
            .unwrap();
        assert_eq!(handler.validate_child_capacity(&vector(400, 512)), Ok(()));
        assert_eq!(
            handler.validate_child_capacity(&vector(401, 512)),
            Err(BudgetError::ChildCapacityExceeded)
        );
    }

    #[test]
    fn emergency_policy_preserves_incident_held_state() {
        let actions = BudgetHandler::emergency_actions(EmergencyScope::Zone);
        assert!(actions.contains(&EmergencyAction::StopChildProcesses));
        assert!(actions.contains(&EmergencyAction::PreserveIncidentHeldState));
    }

    #[test]
    fn metric_labels_are_closed_and_identity_free() {
        assert_eq!(BUDGET_METRIC_LABEL_KEYS, &["dimension", "outcome"]);
        for dimension in [
            BudgetDimension::MilliCpu,
            BudgetDimension::MemoryBytes,
            BudgetDimension::Pids,
            BudgetDimension::FileDescriptors,
            BudgetDimension::NetworkEgressBytesPerSecond,
            BudgetDimension::Threads,
        ] {
            assert!(!dimension.label().contains("resource"));
        }
    }
}
