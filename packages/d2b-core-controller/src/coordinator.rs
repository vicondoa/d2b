//! Per-Zone coordination for configuration and provider controllers.
//!
//! The coordinator is deliberately an ordinary value owned by the core
//! authority index.  It replaces process-global coordination state with
//! Zone-keyed state and therefore cannot let one Zone suppress reconciliation
//! or activation in another Zone.

use std::collections::{BTreeMap, btree_map::Entry};

use d2b_contracts::v3::{ResourceBundleGenerationId, ZoneId};

/// Closed failure from the per-Zone coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorError {
    /// The requested Zone has not been registered.
    ZoneNotRegistered,
    /// The requested VM has not been bound to an authoritative Zone.
    VmNotRegistered,
    /// A VM was already bound to a different Zone.
    VmZoneConflict,
    /// A USBIP reconciliation pass is already active for this Zone.
    UsbipReconcileInFlight,
    /// An activation lock is already held for this Zone.
    ActivationInFlight,
    /// A different configuration generation is already staged for this Zone.
    ConfigurationStagingInFlight,
    /// The caller attempted to release a lock that is not held.
    LockNotHeld,
    /// A shutdown generation cannot be zero.
    InvalidShutdownGeneration,
    /// A shutdown generation cannot advance any further.
    ShutdownGenerationExhausted,
    /// A configuration generation cannot be zero.
    InvalidConfigurationGeneration,
}

impl CoordinatorError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoneNotRegistered => "zone-coordinator-zone-not-registered",
            Self::VmNotRegistered => "zone-coordinator-vm-not-registered",
            Self::VmZoneConflict => "zone-coordinator-vm-zone-conflict",
            Self::UsbipReconcileInFlight => "zone-coordinator-usbip-reconcile-in-flight",
            Self::ActivationInFlight => "zone-coordinator-activation-in-flight",
            Self::ConfigurationStagingInFlight => {
                "zone-coordinator-configuration-staging-in-flight"
            }
            Self::LockNotHeld => "zone-coordinator-lock-not-held",
            Self::InvalidShutdownGeneration => "zone-coordinator-shutdown-generation-invalid",
            Self::ShutdownGenerationExhausted => "zone-coordinator-shutdown-generation-exhausted",
            Self::InvalidConfigurationGeneration => {
                "zone-coordinator-configuration-generation-invalid"
            }
        }
    }
}

impl core::fmt::Display for CoordinatorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for CoordinatorError {}

/// Per-Zone configuration staging state.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigurationStaging {
    pub(crate) pending: Option<ResourceBundleGenerationId>,
    pub(crate) active: Option<ResourceBundleGenerationId>,
    pub(crate) pending_ordinal: Option<u64>,
    pub(crate) active_ordinal: Option<u64>,
}

impl ConfigurationStaging {
    /// Create empty staging for one Zone.
    pub const fn empty() -> Self {
        Self {
            pending: None,
            active: None,
            pending_ordinal: None,
            active_ordinal: None,
        }
    }

    /// Borrow the staged outgoing or candidate generation.
    pub const fn pending(&self) -> Option<&ResourceBundleGenerationId> {
        self.pending.as_ref()
    }

    /// Borrow the active generation known to this coordinator.
    pub const fn active(&self) -> Option<&ResourceBundleGenerationId> {
        self.active.as_ref()
    }

    /// Return the staged configuration ordinal used by the daemon publisher.
    pub const fn pending_ordinal(&self) -> Option<u64> {
        self.pending_ordinal
    }

    /// Return the active configuration ordinal used by the daemon publisher.
    pub const fn active_ordinal(&self) -> Option<u64> {
        self.active_ordinal
    }
}

impl core::fmt::Debug for ConfigurationStaging {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ConfigurationStaging")
            .field("has_pending", &self.pending.is_some())
            .field("has_active", &self.active.is_some())
            .field("has_pending_ordinal", &self.pending_ordinal.is_some())
            .field("has_active_ordinal", &self.active_ordinal.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ZoneCoordinatorState {
    usbip_reconcile_active: bool,
    force_shutdown_generation: Option<u64>,
    activation_lock_held: bool,
    staging: ConfigurationStaging,
}

impl ZoneCoordinatorState {
    const fn new() -> Self {
        Self {
            usbip_reconcile_active: false,
            force_shutdown_generation: None,
            activation_lock_held: false,
            staging: ConfigurationStaging::empty(),
        }
    }
}

/// A snapshot of state owned by one Zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneCoordinatorSnapshot {
    usbip_reconcile_active: bool,
    force_shutdown_generation: Option<u64>,
    activation_lock_held: bool,
    staging: ConfigurationStaging,
}

impl ZoneCoordinatorSnapshot {
    /// Whether this Zone currently owns a USBIP reconcile pass.
    pub const fn usbip_reconcile_active(&self) -> bool {
        self.usbip_reconcile_active
    }

    /// Return the force-shutdown generation, if one is recorded.
    pub const fn force_shutdown_generation(&self) -> Option<u64> {
        self.force_shutdown_generation
    }

    /// Whether the per-Zone activation lock is held.
    pub const fn activation_lock_held(&self) -> bool {
        self.activation_lock_held
    }

    /// Borrow per-Zone configuration staging.
    pub const fn staging(&self) -> &ConfigurationStaging {
        &self.staging
    }
}

/// Core-owned coordinator keyed by Zone authority identity.
#[derive(Default)]
pub struct ZoneCoordinator {
    zones: BTreeMap<ZoneId, ZoneCoordinatorState>,
    vm_zones: BTreeMap<String, ZoneId>,
}

impl ZoneCoordinator {
    /// Construct an empty authority-index coordinator.
    pub const fn new() -> Self {
        Self {
            zones: BTreeMap::new(),
            vm_zones: BTreeMap::new(),
        }
    }

    /// Register one Zone before it can acquire coordination state.
    ///
    /// Registration is idempotent and never resets state for an already
    /// admitted Zone. The caller is expected to supply Zones from the trusted
    /// authority index, not from a public lifecycle request.
    pub fn register_zone(&mut self, zone: ZoneId) -> bool {
        match self.zones.entry(zone) {
            Entry::Vacant(entry) => {
                entry.insert(ZoneCoordinatorState::new());
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Bind one trusted VM resource to its owning Zone.
    ///
    /// This is the routing part of the authority index. Coordination methods
    /// can then resolve a VM without accepting a caller-supplied Zone or
    /// authority.
    pub fn bind_vm(
        &mut self,
        vm: impl Into<String>,
        zone: &ZoneId,
    ) -> Result<bool, CoordinatorError> {
        if !self.zones.contains_key(zone) {
            return Err(CoordinatorError::ZoneNotRegistered);
        }
        let vm = vm.into();
        match self.vm_zones.get(&vm) {
            Some(existing) if existing != zone => Err(CoordinatorError::VmZoneConflict),
            Some(_) => Ok(false),
            None => {
                self.vm_zones.insert(vm, zone.clone());
                Ok(true)
            }
        }
    }

    /// Resolve a trusted VM resource to its authoritative Zone.
    pub fn zone_for_vm(&self, vm: &str) -> Result<&ZoneId, CoordinatorError> {
        self.vm_zones
            .get(vm)
            .ok_or(CoordinatorError::VmNotRegistered)
    }

    /// Return the number of independently coordinated Zones.
    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    /// Return a redacted snapshot of one Zone's coordination state.
    pub fn snapshot(&self, zone: &ZoneId) -> Result<ZoneCoordinatorSnapshot, CoordinatorError> {
        let state = self
            .zones
            .get(zone)
            .ok_or(CoordinatorError::ZoneNotRegistered)?;
        Ok(ZoneCoordinatorSnapshot {
            usbip_reconcile_active: state.usbip_reconcile_active,
            force_shutdown_generation: state.force_shutdown_generation,
            activation_lock_held: state.activation_lock_held,
            staging: state.staging.clone(),
        })
    }

    /// Acquire the per-Zone USBIP reconcile lease.
    pub fn begin_usbip_reconcile(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        let state = self.state_mut(zone)?;
        if state.usbip_reconcile_active {
            return Err(CoordinatorError::UsbipReconcileInFlight);
        }
        state.usbip_reconcile_active = true;
        Ok(())
    }

    /// Release the per-Zone USBIP reconcile lease.
    pub fn finish_usbip_reconcile(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        let state = self.state_mut(zone)?;
        if !state.usbip_reconcile_active {
            return Err(CoordinatorError::LockNotHeld);
        }
        state.usbip_reconcile_active = false;
        Ok(())
    }

    /// Acquire the configuration activation lock for one Zone.
    pub fn begin_activation(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        let state = self.state_mut(zone)?;
        if state.activation_lock_held {
            return Err(CoordinatorError::ActivationInFlight);
        }
        state.activation_lock_held = true;
        Ok(())
    }

    /// Release the configuration activation lock for one Zone.
    pub fn finish_activation(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        let state = self.state_mut(zone)?;
        if !state.activation_lock_held {
            return Err(CoordinatorError::LockNotHeld);
        }
        state.activation_lock_held = false;
        Ok(())
    }

    /// Record a force-shutdown generation only in the selected Zone.
    pub fn set_force_shutdown_generation(
        &mut self,
        zone: &ZoneId,
        generation: u64,
    ) -> Result<(), CoordinatorError> {
        if generation == 0 {
            return Err(CoordinatorError::InvalidShutdownGeneration);
        }
        self.state_mut(zone)?.force_shutdown_generation = Some(generation);
        Ok(())
    }

    /// Advance and record a force-shutdown generation for one Zone.
    pub fn note_force_shutdown_request(&mut self, zone: &ZoneId) -> Result<u64, CoordinatorError> {
        let state = self.state_mut(zone)?;
        let generation = state
            .force_shutdown_generation
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(CoordinatorError::ShutdownGenerationExhausted)?;
        state.force_shutdown_generation = Some(generation);
        Ok(generation)
    }

    /// Clear a force-shutdown generation after the matching Zone teardown.
    pub fn clear_force_shutdown_generation(
        &mut self,
        zone: &ZoneId,
        generation: u64,
    ) -> Result<(), CoordinatorError> {
        let state = self.state_mut(zone)?;
        if state.force_shutdown_generation == Some(generation) {
            state.force_shutdown_generation = None;
        }
        Ok(())
    }

    /// Stage a candidate generation for exactly one Zone.
    pub fn stage_configuration(
        &mut self,
        zone: &ZoneId,
        generation: ResourceBundleGenerationId,
    ) -> Result<(), CoordinatorError> {
        self.state_mut(zone)?.staging.pending = Some(generation);
        Ok(())
    }

    /// Stage a broker-published configuration ordinal for exactly one Zone.
    pub fn stage_configuration_ordinal(
        &mut self,
        zone: &ZoneId,
        ordinal: u64,
    ) -> Result<(), CoordinatorError> {
        if ordinal == 0 {
            return Err(CoordinatorError::InvalidConfigurationGeneration);
        }
        let state = self.state_mut(zone)?;
        if state
            .staging
            .pending_ordinal
            .is_some_and(|pending| pending != ordinal)
        {
            return Err(CoordinatorError::ConfigurationStagingInFlight);
        }
        state.staging.pending_ordinal = Some(ordinal);
        Ok(())
    }

    /// Commit the staged generation after the durable generation record commit.
    pub fn commit_configuration(
        &mut self,
        zone: &ZoneId,
    ) -> Result<Option<ResourceBundleGenerationId>, CoordinatorError> {
        let state = self.state_mut(zone)?;
        let pending = state.staging.pending.take();
        if let Some(generation) = pending.clone() {
            state.staging.active = Some(generation);
        }
        Ok(pending)
    }

    /// Commit the broker-published ordinal after its durable generation record.
    pub fn commit_configuration_ordinal(
        &mut self,
        zone: &ZoneId,
    ) -> Result<Option<u64>, CoordinatorError> {
        let state = self.state_mut(zone)?;
        let pending = state.staging.pending_ordinal.take();
        if let Some(ordinal) = pending {
            state.staging.active_ordinal = Some(ordinal);
        }
        Ok(pending)
    }

    /// Clear a pending staging record after an aborted activation.
    pub fn abort_configuration(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        self.state_mut(zone)?.staging.pending = None;
        Ok(())
    }

    /// Clear a pending broker-published ordinal after an aborted activation.
    pub fn abort_configuration_ordinal(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        self.state_mut(zone)?.staging.pending_ordinal = None;
        Ok(())
    }

    fn state_mut(&mut self, zone: &ZoneId) -> Result<&mut ZoneCoordinatorState, CoordinatorError> {
        self.zones
            .get_mut(zone)
            .ok_or(CoordinatorError::ZoneNotRegistered)
    }
}

impl core::fmt::Debug for ZoneCoordinator {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneCoordinator")
            .field("zone_count", &self.zones.len())
            .field("vm_count", &self.vm_zones.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> ZoneId {
        ZoneId::parse(name).expect("valid Zone")
    }

    fn generation(byte: char) -> ResourceBundleGenerationId {
        ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64)))
            .expect("valid generation")
    }

    #[test]
    fn coordination_state_is_isolated_by_zone() {
        let mut coordinator = ZoneCoordinator::new();
        let work = zone("work");
        let personal = zone("personal");
        assert!(coordinator.register_zone(work.clone()));
        assert!(coordinator.register_zone(personal.clone()));
        assert_eq!(coordinator.bind_vm("work-vm", &work), Ok(true));
        assert_eq!(coordinator.bind_vm("personal-vm", &personal), Ok(true));

        coordinator.begin_usbip_reconcile(&work).unwrap();
        coordinator.begin_activation(&work).unwrap();
        coordinator
            .stage_configuration(&work, generation('a'))
            .unwrap();
        coordinator.set_force_shutdown_generation(&work, 7).unwrap();

        let untouched = coordinator.snapshot(&personal).unwrap();
        assert!(!untouched.usbip_reconcile_active());
        assert!(!untouched.activation_lock_held());
        assert_eq!(untouched.force_shutdown_generation(), None);
        assert_eq!(untouched.staging().pending(), None);
        assert_eq!(untouched.staging().pending_ordinal(), None);
        assert_eq!(
            coordinator.begin_usbip_reconcile(&personal),
            Ok(()),
            "one Zone cannot suppress another Zone"
        );
        assert_eq!(coordinator.zone_for_vm("work-vm"), Ok(&work));
        assert_eq!(coordinator.zone_for_vm("personal-vm"), Ok(&personal));
    }

    #[test]
    fn staged_generation_is_committed_only_for_its_zone() {
        let mut coordinator = ZoneCoordinator::new();
        let work = zone("work");
        let personal = zone("personal");
        coordinator.register_zone(work.clone());
        coordinator.register_zone(personal.clone());
        coordinator
            .stage_configuration(&work, generation('a'))
            .unwrap();
        assert_eq!(coordinator.commit_configuration(&personal), Ok(None));
        assert_eq!(
            coordinator.commit_configuration(&work).unwrap(),
            Some(generation('a'))
        );
        assert_eq!(
            coordinator.snapshot(&work).unwrap().staging().active(),
            Some(&generation('a'))
        );
        assert_eq!(
            coordinator.snapshot(&personal).unwrap().staging().active(),
            None
        );
    }

    #[test]
    fn vm_binding_is_authoritative_and_does_not_cross_zone_state() {
        let mut coordinator = ZoneCoordinator::new();
        let work = zone("work");
        let personal = zone("personal");
        coordinator.register_zone(work.clone());
        coordinator.register_zone(personal.clone());
        assert_eq!(coordinator.bind_vm("guest", &work), Ok(true));
        assert_eq!(
            coordinator.bind_vm("guest", &personal),
            Err(CoordinatorError::VmZoneConflict)
        );
        assert_eq!(
            coordinator.zone_for_vm("unknown"),
            Err(CoordinatorError::VmNotRegistered)
        );

        coordinator.note_force_shutdown_request(&work).unwrap();
        assert_eq!(
            coordinator
                .snapshot(&work)
                .unwrap()
                .force_shutdown_generation(),
            Some(1)
        );
        assert_eq!(
            coordinator
                .snapshot(&personal)
                .unwrap()
                .force_shutdown_generation(),
            None
        );

        coordinator.stage_configuration_ordinal(&work, 4).unwrap();
        assert_eq!(
            coordinator.stage_configuration_ordinal(&work, 5),
            Err(CoordinatorError::ConfigurationStagingInFlight)
        );
        assert_eq!(
            coordinator
                .snapshot(&personal)
                .unwrap()
                .staging()
                .pending_ordinal(),
            None
        );
        assert_eq!(coordinator.commit_configuration_ordinal(&work), Ok(Some(4)));
        assert_eq!(
            coordinator
                .snapshot(&work)
                .unwrap()
                .staging()
                .active_ordinal(),
            Some(4)
        );
    }
}
