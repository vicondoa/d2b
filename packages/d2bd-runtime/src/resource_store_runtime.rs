//! Provider-neutral broker admission for a Zone resource store.

use std::{os::fd::OwnedFd, sync::Arc};

use d2b_contracts_broker::broker_wire::OpenZoneStoreResponse;
use d2b_core_controller::authority::ExternalNicRecoveryInventory;

/// Maximum number of Zone runtimes owned by one daemon.
pub const MAX_ZONE_RUNTIMES: usize = 64;

/// Broker client result required by the daemon's Zone resource runtime.
pub struct OpenedZoneStore {
    /// Opaque broker response metadata.
    pub response: OpenZoneStoreResponse,
    /// The one owned database descriptor received from the broker.
    pub database_fd: OwnedFd,
    /// Host/bundle-owned trusted external-NIC inventory, when available.
    pub external_inventory: Option<Arc<dyn ExternalNicRecoveryInventory>>,
}

impl core::fmt::Debug for OpenedZoneStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OpenedZoneStore")
            .field("response", &self.response)
            .field("has_external_inventory", &self.external_inventory.is_some())
            .finish()
    }
}
