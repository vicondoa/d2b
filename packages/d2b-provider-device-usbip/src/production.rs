//! Production composition seam for the USBIP supervisor.
//!
//! The Provider owns ordering and retention.  The daemon supplies one
//! dispatcher that maps these typed calls to Core authority admission,
//! typed Process-resource effects and the closed broker projection operation.
//! This module contains no broker dependency and therefore cannot bypass the
//! typed supervisor boundary.

use d2b_contracts_resource::v3::ResourceUid;

use crate::{
    AttachProcessIdentity, BindingIdentity, BindingLifecycleError, BindingPort, BindingProxyLease,
    BindingSlotLease, OwnedBusBinding, PhysicalAuthorityLease, ServiceLifecycleError, ServicePort,
    ServiceRelayLease,
};

/// Daemon-owned USBIP dispatch surface.
///
/// Implementations must reserve Host-global claims before `bind_owned` or
/// `ensure_attach_process`, keep every lease until its close operation
/// confirms, and submit attach work as a typed Process-resource effect.
pub trait UsbipBrokerDispatcher {
    /// Reserve the exact physical USB backing.
    fn reserve_physical(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<PhysicalAuthorityLease, ServiceLifecycleError>;

    /// Reserve the one shared Network relay.
    fn reserve_relay(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<ServiceRelayLease, ServiceLifecycleError>;

    /// Bind only the physical device admitted by the retained authority.
    fn bind_owned(
        &mut self,
        physical: &PhysicalAuthorityLease,
    ) -> Result<OwnedBusBinding, ServiceLifecycleError>;

    /// Unbind only the owned physical device.
    fn unbind_owned(&mut self, binding: &OwnedBusBinding) -> Result<(), ServiceLifecycleError>;

    /// Release relay authority after unbind and projection removal.
    fn release_relay(&mut self, relay: ServiceRelayLease) -> Result<(), ServiceLifecycleError>;

    /// Release physical authority after every process is closed.
    fn release_physical(
        &mut self,
        physical: PhysicalAuthorityLease,
    ) -> Result<(), ServiceLifecycleError>;

    /// Reserve a Service slot for one exact Binding.
    fn acquire_slot(
        &mut self,
        binding: &BindingIdentity,
    ) -> Result<BindingSlotLease, BindingLifecycleError>;

    /// Start one Binding-private proxy.
    fn start_proxy(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<BindingProxyLease, BindingLifecycleError>;

    /// Ensure one attach EphemeralProcess child.
    fn ensure_attach_process(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError>;

    /// Observe one persisted attach Process child by its identity.
    fn observe_attach_process(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<crate::AttachmentObservation, BindingLifecycleError>;

    /// Delete the Binding-private Guest Endpoint.
    fn delete_guest_endpoint(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError>;

    /// Delete one exact attach Process after Endpoint deletion.
    fn delete_attach_process(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError>;

    /// Close one exact Binding-private proxy.
    fn close_proxy(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError>;

    /// Release one Service slot after proxy closure.
    fn release_slot(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<(), BindingLifecycleError>;
}

/// Typed Provider ports backed by the daemon dispatcher.
pub struct ProductionPort<D> {
    dispatcher: D,
}

impl<D> ProductionPort<D> {
    /// Bind the Provider lifecycle to a daemon-owned dispatcher.
    pub const fn new(dispatcher: D) -> Self {
        Self { dispatcher }
    }

    /// Borrow the dispatcher for status and diagnostics.
    pub const fn dispatcher(&self) -> &D {
        &self.dispatcher
    }

    /// Mutably borrow the dispatcher for one supervisor pass.
    pub const fn dispatcher_mut(&mut self) -> &mut D {
        &mut self.dispatcher
    }
}

impl<D: UsbipBrokerDispatcher> ServicePort for ProductionPort<D> {
    fn reserve_physical(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<PhysicalAuthorityLease, ServiceLifecycleError> {
        self.dispatcher.reserve_physical(service_uid)
    }

    fn reserve_relay(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<ServiceRelayLease, ServiceLifecycleError> {
        self.dispatcher.reserve_relay(service_uid)
    }

    fn bind_owned(
        &mut self,
        physical: &PhysicalAuthorityLease,
    ) -> Result<OwnedBusBinding, ServiceLifecycleError> {
        self.dispatcher.bind_owned(physical)
    }

    fn unbind_owned(&mut self, binding: &OwnedBusBinding) -> Result<(), ServiceLifecycleError> {
        self.dispatcher.unbind_owned(binding)
    }

    fn release_relay(&mut self, relay: ServiceRelayLease) -> Result<(), ServiceLifecycleError> {
        self.dispatcher.release_relay(relay)
    }

    fn release_physical(
        &mut self,
        physical: PhysicalAuthorityLease,
    ) -> Result<(), ServiceLifecycleError> {
        self.dispatcher.release_physical(physical)
    }
}

impl<D: UsbipBrokerDispatcher> BindingPort for ProductionPort<D> {
    fn acquire_slot(
        &mut self,
        binding: &BindingIdentity,
    ) -> Result<BindingSlotLease, BindingLifecycleError> {
        self.dispatcher.acquire_slot(binding)
    }

    fn start_proxy(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<BindingProxyLease, BindingLifecycleError> {
        self.dispatcher.start_proxy(binding, slot)
    }

    fn ensure_attach_process(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError> {
        self.dispatcher.ensure_attach_process(binding, proxy)
    }

    fn observe_attach_process(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<crate::AttachmentObservation, BindingLifecycleError> {
        self.dispatcher.observe_attach_process(binding, identity)
    }

    fn delete_guest_endpoint(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.dispatcher.delete_guest_endpoint(binding, proxy)
    }

    fn delete_attach_process(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError> {
        self.dispatcher.delete_attach_process(binding, identity)
    }

    fn close_proxy(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        self.dispatcher.close_proxy(binding, proxy)
    }

    fn release_slot(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<(), BindingLifecycleError> {
        self.dispatcher.release_slot(binding, slot)
    }
}
