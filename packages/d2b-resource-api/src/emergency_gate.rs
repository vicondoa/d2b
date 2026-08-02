//! Pure EmergencyPolicy admission and effect signals.

use d2b_contracts::v3::{EmergencyPolicySpec, EmergencyScope, effective_scope};

/// Effective Zone-wide emergency gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmergencyGate {
    scope: EmergencyScope,
    drain_deadline_seconds: Option<u32>,
}

impl EmergencyGate {
    /// Build the union of all enabled policies.
    pub fn from_policies<'a>(policies: impl IntoIterator<Item = &'a EmergencyPolicySpec>) -> Self {
        effective_scope(policies)
            .map(|(scope, deadline)| Self {
                scope,
                drain_deadline_seconds: Some(deadline),
            })
            .unwrap_or_default()
    }

    /// Build a gate from one already-computed effective scope.
    pub const fn new(scope: EmergencyScope, drain_deadline_seconds: Option<u32>) -> Self {
        Self {
            scope,
            drain_deadline_seconds,
        }
    }

    /// Return the effective scope.
    pub const fn scope(self) -> EmergencyScope {
        self.scope
    }

    /// Return the tightest drain deadline.
    pub const fn drain_deadline_seconds(self) -> Option<u32> {
        self.drain_deadline_seconds
    }

    /// Reject a new Resource API admission when the union says so.
    pub const fn admit_request(self) -> Result<(), EmergencyGateError> {
        if self.scope.stop_new_admissions() {
            Err(EmergencyGateError::AdmissionsStopped)
        } else {
            Ok(())
        }
    }

    /// Whether a new Provider component Process may launch.
    pub const fn permit_provider_process_launch(self) -> Result<(), EmergencyGateError> {
        if self.scope.stop_provider_processes() {
            Err(EmergencyGateError::ProviderProcessesStopped)
        } else {
            Ok(())
        }
    }

    /// Whether active ZoneLinks should receive a graceful disconnect signal.
    pub const fn disconnect_zone_links(self) -> bool {
        self.scope.disconnect_zone_links()
    }

    /// Whether ongoing operations should drain before shutdown.
    pub const fn drain_operations(self) -> bool {
        self.scope.drain_ongoing_operations()
    }
}

/// Emergency gate refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyGateError {
    /// New requests are stopped.
    AdmissionsStopped,
    /// Provider process launches are stopped.
    ProviderProcessesStopped,
}

impl core::fmt::Display for EmergencyGateError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::AdmissionsStopped => "emergency-admissions-stopped",
            Self::ProviderProcessesStopped => "emergency-provider-processes-stopped",
        })
    }
}

impl std::error::Error for EmergencyGateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_stops_admission_without_deleting_process_resources() {
        let policy = EmergencyPolicySpec::new(
            true,
            EmergencyScope::new(true, true, true, false),
            10,
            "maintenance",
        )
        .unwrap();
        let gate = EmergencyGate::from_policies([&policy]);
        assert_eq!(
            gate.admit_request(),
            Err(EmergencyGateError::AdmissionsStopped)
        );
        assert_eq!(
            gate.permit_provider_process_launch(),
            Err(EmergencyGateError::ProviderProcessesStopped)
        );
        assert!(gate.disconnect_zone_links());
        assert!(!gate.drain_operations());
    }

    #[test]
    fn deactivation_restores_all_effects() {
        let gate = EmergencyGate::from_policies([&EmergencyPolicySpec::default()]);
        assert!(gate.admit_request().is_ok());
        assert!(gate.permit_provider_process_launch().is_ok());
        assert!(!gate.disconnect_zone_links());
    }
}
