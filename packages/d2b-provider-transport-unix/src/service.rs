//! Service-local lifecycle wrapper for the Unix transport portal.

use crate::TransportPortal;

/// The service-owned local transport state.
#[derive(Debug, Default)]
pub struct TransportService {
    portal: TransportPortal,
}

impl TransportService {
    /// Create an empty service-local transport portal.
    pub fn new() -> Self {
        Self {
            portal: TransportPortal::new(),
        }
    }

    /// Borrow the service's bounded portal.
    pub const fn portal(&self) -> &TransportPortal {
        &self.portal
    }

    /// Finalize only descriptors owned by this service.
    pub fn finalize(&self) {
        self.portal.finalize();
    }
}
