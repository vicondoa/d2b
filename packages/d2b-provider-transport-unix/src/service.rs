//! Service-local lifecycle wrapper for the Unix transport portal.

use crate::{
    admission::OpenTransportRequest,
    graph_binding::{AdmittedTransportBinding, TransportAttachEvidence, TransportBindingRegistry},
    portal::{OpenedTransport, PortalError, TransportPortal},
};
use rustix::fd::OwnedFd;

/// The service-owned local transport state.
///
/// The service holds the two things a local transport needs and nothing else:
/// the bounded portal that owns descriptors, and the bounded registry of
/// admitted relationships that says who may attach at all.
#[derive(Debug, Default)]
pub struct TransportService {
    portal: TransportPortal,
    bindings: TransportBindingRegistry,
}

impl TransportService {
    /// Create an empty service-local transport portal and relationship
    /// registry.
    pub fn new() -> Self {
        Self {
            portal: TransportPortal::new(),
            bindings: TransportBindingRegistry::new(),
        }
    }

    /// Borrow the service's bounded portal.
    pub const fn portal(&self) -> &TransportPortal {
        &self.portal
    }

    /// Borrow the service's bounded registry of admitted relationships.
    pub const fn bindings(&self) -> &TransportBindingRegistry {
        &self.bindings
    }

    /// Open one transport under the live admitted relationship.
    ///
    /// The relationship the caller holds is only a claim: the service attaches
    /// against the one its registry currently admits under that key, so a
    /// revoked, drained, or forgotten relationship cannot be opened by
    /// presenting a value that was admitted earlier.
    ///
    /// # Errors
    ///
    /// Returns `BindingNotAdmitted` when no admitted relationship carries the
    /// key, and otherwise the same refusals the portal's graph-bound open
    /// returns.
    pub fn open_under_binding(
        &self,
        binding: &AdmittedTransportBinding,
        evidence: &TransportAttachEvidence,
        request: OpenTransportRequest,
        fd: OwnedFd,
    ) -> Result<OpenedTransport, PortalError> {
        let live = self
            .bindings
            .binding(binding.key())
            .ok_or(PortalError::BindingNotAdmitted)?;
        self.portal
            .open_under_binding(&live, evidence, request, fd)
    }

    /// Finalize only descriptors and relationships owned by this service.
    pub fn finalize(&self) {
        self.portal.finalize();
        self.bindings.finalize();
    }
}
