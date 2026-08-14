//! Daemon-owned composition of the fixed process Providers.
//!
//! The Provider crates remain pure controllers: they receive only the
//! core-owned effect ports. This module is the one production seam that
//! constructs those ports from the authenticated broker transport and the
//! trusted bundle. No Provider receives a broker socket or a bundle resolver.

use std::{path::PathBuf, time::Duration};

use d2b_contracts::broker_wire::BrokerCallerRole;
use d2b_core::bundle_resolver::BundleResolver;
use d2b_provider_supervisor::{
    BrokerProcessBackend, BrokerSystemdEffectOwner, BundleBackedLaunchResolver,
    ProviderSupervisor, SystemdProcessBackend,
};
use d2b_provider_system_minijail::MinijailProcessProvider;
use d2b_provider_system_systemd::SystemdProcessProvider;

/// The fixed process Provider names wired by the daemon.
pub const FIXED_PROCESS_PROVIDER_NAMES: [&str; 2] = ["system-minijail", "system-systemd"];

type BrokerProcessSupervisor =
    ProviderSupervisor<BrokerProcessBackend<BundleBackedLaunchResolver>>;
type BrokerSystemdSupervisor =
    ProviderSupervisor<SystemdProcessBackend<BrokerSystemdEffectOwner>>;

/// Production process Provider controllers.
///
/// The concrete supervisors are retained by the daemon for its whole
/// lifetime. Their internal handles and broker effect owners never cross the
/// Provider boundary; Provider code sees only the
/// `ProcessLaunchEffectPort` implemented by `ProviderSupervisor`.
pub struct ProductionProcessProviders {
    minijail: MinijailProcessProvider<BrokerProcessSupervisor>,
    systemd: SystemdProcessProvider<BrokerSystemdSupervisor>,
}

impl std::fmt::Debug for ProductionProcessProviders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionProcessProviders")
            .field("providers", &FIXED_PROCESS_PROVIDER_NAMES)
            .finish()
    }
}

impl ProductionProcessProviders {
    /// Construct both fixed process Providers over the authenticated broker.
    pub fn new(
        bundle: BundleResolver,
        broker_socket: impl Into<PathBuf>,
        caller_role: BrokerCallerRole,
    ) -> Self {
        let broker_socket = broker_socket.into();
        let resolver = BundleBackedLaunchResolver::new(bundle).with_observation_socket(
            broker_socket.clone(),
            Duration::from_secs(10),
            caller_role.clone(),
        );
        let minijail_backend = BrokerProcessBackend::with_socket_and_role(
            resolver.clone(),
            broker_socket.clone(),
            Duration::from_secs(10),
            caller_role.clone(),
        );
        let systemd_owner = BrokerSystemdEffectOwner::with_socket(
            resolver,
            broker_socket,
            Duration::from_secs(10),
            caller_role,
        );
        Self {
            minijail: MinijailProcessProvider::new(ProviderSupervisor::new(minijail_backend)),
            systemd: SystemdProcessProvider::new(ProviderSupervisor::new(
                SystemdProcessBackend::new(systemd_owner),
            )),
        }
    }

    /// Borrow the daemon-owned minijail Provider.
    pub const fn minijail(&self) -> &MinijailProcessProvider<BrokerProcessSupervisor> {
        &self.minijail
    }

    /// Borrow the daemon-owned systemd Provider.
    pub const fn systemd(&self) -> &SystemdProcessProvider<BrokerSystemdSupervisor> {
        &self.systemd
    }

    /// Return the fixed Provider names in contract order.
    pub const fn provider_names() -> &'static [&'static str; 2] {
        &FIXED_PROCESS_PROVIDER_NAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_composition_registers_only_fixed_process_providers() {
        assert_eq!(
            ProductionProcessProviders::provider_names(),
            &["system-minijail", "system-systemd"]
        );
    }
}
