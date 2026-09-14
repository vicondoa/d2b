//! Daemon-owned USBIP Provider dispatcher.
//!
//! This is the composition point between the typed USBIP supervisor and the
//! existing daemon/broker control plane.  Admission is held in a bounded
//! Host-global ledger, bind/unbind use opaque bundle references, and attach
//! uses a typed Process-resource effect before the Provider observes it.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use d2b_contracts::types::BundleOpId;
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, BrokerRequest, BrokerResponse, UsbipBindRequest, UsbipUnbindRequest,
};
use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_device_usbip::{
    AttachProcessIdentity, AttachmentObservation, BindingIdentity, BindingLifecycleError,
    BindingProxyLease, BindingSlotLease, OwnedBusBinding, PhysicalAuthorityLease, ProductionPort,
    ServiceLifecycleError, ServiceRelayLease, UsbipBrokerDispatcher,
};

#[cfg(test)]
use d2b_core::device_usbip_adapter::UsbipCoreAdapter;

use crate::{ServerState, dispatch_broker_request_as};

/// Trusted context resolved by Core for one Service/Binding pair.
#[derive(Clone, PartialEq, Eq)]
pub struct UsbipBindingContext {
    vm_id: String,
    env: String,
    bind_intent_ref: String,
    runner_intent_ref: String,
    physical_key: [u8; 32],
}

impl core::fmt::Debug for UsbipBindingContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("UsbipBindingContext")
            .field("has_vm", &true)
            .field("has_env", &true)
            .field("has_bind_intent", &true)
            .field("has_runner_intent", &true)
            .field("has_physical_key", &true)
            .finish()
    }
}

impl UsbipBindingContext {
    /// Construct a context only from Core-resolved opaque references.
    pub fn new(
        vm_id: impl Into<String>,
        env: impl Into<String>,
        bind_intent_ref: impl Into<String>,
        runner_intent_ref: impl Into<String>,
        physical_key: [u8; 32],
    ) -> Result<Self, ServiceLifecycleError> {
        let vm_id = vm_id.into();
        let env = env.into();
        let bind_intent_ref = bind_intent_ref.into();
        let runner_intent_ref = runner_intent_ref.into();
        if vm_id.is_empty()
            || env.is_empty()
            || bind_intent_ref.is_empty()
            || runner_intent_ref.is_empty()
            || physical_key == [0; 32]
        {
            return Err(ServiceLifecycleError::InvalidState);
        }
        Ok(Self {
            vm_id,
            env,
            bind_intent_ref,
            runner_intent_ref,
            physical_key,
        })
    }

    /// Resolve Core-owned context before any host firewall or runner effect.
    #[cfg(test)]
    pub(crate) fn before_host_effects(
        vm_id: impl Into<String>,
        env: impl Into<String>,
        bind_intent_ref: impl Into<String>,
        runner_intent_ref: impl Into<String>,
        physical_identity: &[u8],
    ) -> Result<Self, ServiceLifecycleError> {
        Self::new(
            vm_id,
            env,
            bind_intent_ref,
            runner_intent_ref,
            UsbipCoreAdapter::physical_usb_backing_key(physical_identity).as_bytes(),
        )
    }
}

#[derive(Default)]
pub(crate) struct AuthorityLedger {
    next_token: AtomicU64,
    physical: BTreeMap<[u8; 32], (String, PhysicalAuthorityLease)>,
    relay: BTreeMap<String, (String, ServiceRelayLease)>,
    slots: BTreeMap<String, BindingSlotLease>,
    proxies: BTreeMap<String, BindingProxyLease>,
}

impl AuthorityLedger {
    fn token(&self, seed: u8) -> [u8; 16] {
        let sequence = self.next_token.fetch_add(1, Ordering::Relaxed);
        let mut token = [seed; 16];
        token[..8].copy_from_slice(&sequence.to_be_bytes());
        token
    }
}

pub(crate) fn new_authority_ledger() -> Arc<Mutex<AuthorityLedger>> {
    Arc::new(Mutex::new(AuthorityLedger::default()))
}

/// Daemon/broker-backed implementation of the Provider dispatcher.
pub struct DaemonUsbipDispatcher<'a> {
    state: &'a ServerState,
    context: UsbipBindingContext,
    ledger: Arc<Mutex<AuthorityLedger>>,
    attach_identity: Option<AttachProcessIdentity>,
    attach_slot: Option<BindingSlotLease>,
    attach_proxy: Option<BindingProxyLease>,
    owned_binding: Option<OwnedBusBinding>,
    physical_lease: Option<PhysicalAuthorityLease>,
    relay_lease: Option<ServiceRelayLease>,
}

impl<'a> DaemonUsbipDispatcher<'a> {
    /// Construct one dispatcher over the daemon's broker and authority ledger.
    pub(crate) fn new(
        state: &'a ServerState,
        context: UsbipBindingContext,
        ledger: Arc<Mutex<AuthorityLedger>>,
    ) -> Self {
        Self {
            state,
            context,
            ledger,
            attach_identity: None,
            attach_slot: None,
            attach_proxy: None,
            owned_binding: None,
            physical_lease: None,
            relay_lease: None,
        }
    }

    /// Wrap this dispatcher in the typed Service/Binding ports.
    pub(crate) fn into_port(self) -> ProductionPort<Self> {
        ProductionPort::new(self)
    }

    fn broker_role(&self) -> BrokerCallerRole {
        BrokerCallerRole::AdminUid {
            uid: self.state.daemon_uid,
        }
    }

    fn ack(&self, request: BrokerRequest) -> Result<(), ServiceLifecycleError> {
        match dispatch_broker_request_as(self.state, request, self.broker_role()) {
            Ok(BrokerResponse::Ack(response)) if response.accepted => Ok(()),
            Ok(BrokerResponse::Error(_)) | Ok(_) => Err(ServiceLifecycleError::Transient),
            Err(_) => Err(ServiceLifecycleError::Transient),
        }
    }

    fn binding_key(binding: &BindingIdentity) -> String {
        binding.as_resource_uid().to_canonical_string()
    }
}

impl<'a> UsbipBrokerDispatcher for DaemonUsbipDispatcher<'a> {
    fn reserve_physical(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<PhysicalAuthorityLease, ServiceLifecycleError> {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| ServiceLifecycleError::Transient)?;
        if let Some((owner, lease)) = ledger.physical.get(&self.context.physical_key) {
            if owner == &service_uid.to_canonical_string() {
                self.physical_lease = Some(lease.clone());
                return Ok(lease.clone());
            }
            return Err(ServiceLifecycleError::PhysicalAuthorityConflict);
        }
        let lease = PhysicalAuthorityLease::from_adapter(ledger.token(1));
        ledger.physical.insert(
            self.context.physical_key,
            (service_uid.to_canonical_string(), lease.clone()),
        );
        self.physical_lease = Some(lease.clone());
        Ok(lease)
    }

    fn reserve_relay(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<ServiceRelayLease, ServiceLifecycleError> {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| ServiceLifecycleError::Transient)?;
        if let Some((owner, lease)) = ledger.relay.get(&self.context.env) {
            if owner == &service_uid.to_canonical_string() {
                self.relay_lease = Some(lease.clone());
                return Ok(lease.clone());
            }
            return Err(ServiceLifecycleError::RelayAuthorityConflict);
        }
        let lease = ServiceRelayLease::from_adapter(ledger.token(2));
        ledger.relay.insert(
            self.context.env.clone(),
            (service_uid.to_canonical_string(), lease.clone()),
        );
        self.relay_lease = Some(lease.clone());
        Ok(lease)
    }

    fn bind_owned(
        &mut self,
        physical: &PhysicalAuthorityLease,
    ) -> Result<OwnedBusBinding, ServiceLifecycleError> {
        if self.physical_lease.as_ref() != Some(physical) {
            return Err(ServiceLifecycleError::PhysicalAuthorityConflict);
        }
        self.ack(BrokerRequest::UsbipBind(UsbipBindRequest {
            bundle_usbip_bind_intent_ref: BundleOpId::new(self.context.bind_intent_ref.clone()),
            tracing_span_id: None,
        }))?;
        let binding = OwnedBusBinding::from_adapter([3; 16]);
        self.owned_binding = Some(binding.clone());
        Ok(binding)
    }

    fn unbind_owned(&mut self, binding: &OwnedBusBinding) -> Result<(), ServiceLifecycleError> {
        if self.owned_binding.as_ref() != Some(binding) {
            return Err(ServiceLifecycleError::ForeignOwnership);
        }
        self.ack(BrokerRequest::UsbipUnbind(UsbipUnbindRequest {
            bundle_usbip_bind_intent_ref: BundleOpId::new(self.context.bind_intent_ref.clone()),
            preserve_durable_claim: false,
            tracing_span_id: None,
        }))?;
        self.owned_binding = None;
        Ok(())
    }

    fn release_relay(&mut self, relay: ServiceRelayLease) -> Result<(), ServiceLifecycleError> {
        if self.relay_lease.as_ref() != Some(&relay) {
            return Err(ServiceLifecycleError::ForeignOwnership);
        }
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| ServiceLifecycleError::Transient)?;
        if ledger
            .relay
            .get(&self.context.env)
            .is_some_and(|(_, lease)| lease == &relay)
        {
            ledger.relay.remove(&self.context.env);
        }
        self.relay_lease = None;
        Ok(())
    }

    fn release_physical(
        &mut self,
        physical: PhysicalAuthorityLease,
    ) -> Result<(), ServiceLifecycleError> {
        if self.physical_lease.as_ref() != Some(&physical) {
            return Err(ServiceLifecycleError::ForeignOwnership);
        }
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| ServiceLifecycleError::Transient)?;
        if ledger
            .physical
            .get(&self.context.physical_key)
            .is_some_and(|(_, lease)| lease == &physical)
        {
            ledger.physical.remove(&self.context.physical_key);
        }
        self.physical_lease = None;
        Ok(())
    }

    fn acquire_slot(
        &mut self,
        binding: &BindingIdentity,
    ) -> Result<BindingSlotLease, BindingLifecycleError> {
        let key = Self::binding_key(binding);
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| BindingLifecycleError::Transient)?;
        if let Some(slot) = ledger.slots.get(&key) {
            self.attach_slot = Some(slot.clone());
            return Ok(slot.clone());
        }
        let slot = BindingSlotLease::from_adapter(ledger.token(4));
        ledger.slots.insert(key, slot.clone());
        self.attach_slot = Some(slot.clone());
        Ok(slot)
    }

    fn start_proxy(
        &mut self,
        binding: &BindingIdentity,
        _slot: &BindingSlotLease,
    ) -> Result<BindingProxyLease, BindingLifecycleError> {
        let key = Self::binding_key(binding);
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| BindingLifecycleError::Transient)?;
        if let Some(proxy) = ledger.proxies.get(&key) {
            self.attach_proxy = Some(proxy.clone());
            return Ok(proxy.clone());
        }
        let proxy = BindingProxyLease::from_adapter(ledger.token(5));
        ledger.proxies.insert(key, proxy.clone());
        self.attach_proxy = Some(proxy.clone());
        Ok(proxy)
    }

    // The legacy attach seams below have no live consumer (U17 site 5): the v3
    // Binding realizes its attach through the declared
    // `Process/...guest-proxy` child rows, not from this port. They stay
    // fail-closed rather than pretending to realize a child.
    fn ensure_attach_process(
        &mut self,
        _binding: &BindingIdentity,
        _proxy: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError> {
        Err(BindingLifecycleError::Transient)
    }

    fn observe_attach_process(
        &mut self,
        _binding: &BindingIdentity,
        _identity: &AttachProcessIdentity,
    ) -> Result<AttachmentObservation, BindingLifecycleError> {
        Err(BindingLifecycleError::Transient)
    }

    fn delete_guest_endpoint(
        &mut self,
        _binding: &BindingIdentity,
        _proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        Err(BindingLifecycleError::Transient)
    }

    fn delete_attach_process(
        &mut self,
        _binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError> {
        if self.attach_identity.as_ref() != Some(identity) {
            return Err(BindingLifecycleError::ForeignIdentity);
        }
        self.attach_identity = None;
        Err(BindingLifecycleError::Transient)
    }

    fn close_proxy(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError> {
        if self.attach_proxy.as_ref() != Some(proxy) {
            return Err(BindingLifecycleError::ForeignIdentity);
        }
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| BindingLifecycleError::Transient)?;
        ledger.proxies.remove(&Self::binding_key(binding));
        self.attach_proxy = None;
        Ok(())
    }

    fn release_slot(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<(), BindingLifecycleError> {
        if self.attach_slot.as_ref() != Some(slot) {
            return Err(BindingLifecycleError::ForeignIdentity);
        }
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| BindingLifecycleError::Transient)?;
        let key = Self::binding_key(binding);
        if ledger.slots.get(&key).is_some_and(|owned| owned == slot) {
            ledger.slots.remove(&key);
        }
        self.attach_slot = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::UsbipBindingContext;

    fn context() -> UsbipBindingContext {
        UsbipBindingContext::before_host_effects(
            "corp-vm",
            "work",
            "bind-intent",
            "runner-intent",
            b"work:1-2",
        )
        .expect("valid USBIP context")
    }

    #[test]
    fn context_is_required_before_host_effects() {
        assert!(
            UsbipBindingContext::before_host_effects("", "work", "bind", "runner", b"work:1-2")
                .is_err()
        );
        assert!(
            UsbipBindingContext::before_host_effects("corp-vm", "", "bind", "runner", b"work:1-2")
                .is_err()
        );
        let first = context();
        let second = UsbipBindingContext::before_host_effects(
            "corp-vm",
            "work",
            "bind-intent",
            "runner-intent",
            b"work:1-2",
        )
        .expect("matching USBIP context");
        assert_eq!(first, second);
    }
}
