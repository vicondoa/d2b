//! Per-Zone coordination for configuration and provider controllers.
//!
//! The coordinator is deliberately an ordinary value owned by the core
//! authority index.  It replaces process-global coordination state with
//! Zone-keyed state and therefore cannot let one Zone suppress reconciliation
//! or activation in another Zone.

use std::collections::BTreeMap;

use d2b_contracts::v3::{ResourceBundleGenerationId, ZoneId};

/// Closed failure from the per-Zone coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorError {
    /// The requested Zone has not been registered.
    ZoneNotRegistered,
    /// A USBIP reconciliation pass is already active for this Zone.
    UsbipReconcileInFlight,
    /// An activation lock is already held for this Zone.
    ActivationInFlight,
    /// The caller attempted to release a lock that is not held.
    LockNotHeld,
    /// A shutdown generation cannot be zero.
    InvalidShutdownGeneration,
}

impl CoordinatorError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoneNotRegistered => "zone-coordinator-zone-not-registered",
            Self::UsbipReconcileInFlight => "zone-coordinator-usbip-reconcile-in-flight",
            Self::ActivationInFlight => "zone-coordinator-activation-in-flight",
            Self::LockNotHeld => "zone-coordinator-lock-not-held",
            Self::InvalidShutdownGeneration => "zone-coordinator-shutdown-generation-invalid",
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
    pending: Option<ResourceBundleGenerationId>,
    active: Option<ResourceBundleGenerationId>,
}

impl ConfigurationStaging {
    /// Create empty staging for one Zone.
    pub const fn empty() -> Self {
        Self {
            pending: None,
            active: None,
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
}

impl core::fmt::Debug for ConfigurationStaging {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ConfigurationStaging")
            .field("has_pending", &self.pending.is_some())
            .field("has_active", &self.active.is_some())
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
}

impl ZoneCoordinator {
    /// Construct an empty authority-index coordinator.
    pub const fn new() -> Self {
        Self {
            zones: BTreeMap::new(),
        }
    }

    /// Register one Zone before it can acquire coordination state.
    pub fn register_zone(&mut self, zone: ZoneId) -> bool {
        self.zones
            .insert(zone, ZoneCoordinatorState::new())
            .is_none()
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

    /// Clear a pending staging record after an aborted activation.
    pub fn abort_configuration(&mut self, zone: &ZoneId) -> Result<(), CoordinatorError> {
        self.state_mut(zone)?.staging.pending = None;
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
        assert_eq!(
            coordinator.begin_usbip_reconcile(&personal),
            Ok(()),
            "one Zone cannot suppress another Zone"
        );
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
}
