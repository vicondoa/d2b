//! Separate USB Service and USB Binding lifecycle ownership.
//!
//! A Service owns Host-global authority and the physical bus binding. A Binding
//! owns its Guest attachment, private proxy, and Service slot. The split keeps
//! a Binding finalizer from releasing a physical device that other Bindings
//! might still use.

use d2b_contracts_zone_session::v3::ResourceUid;

/// Opaque Host-global physical-backing reservation issued by the Core adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct PhysicalAuthorityLease([u8; 16]);

impl PhysicalAuthorityLease {
    /// Construct an authority lease at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl core::fmt::Debug for PhysicalAuthorityLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PhysicalAuthorityLease(<redacted>)")
    }
}

/// Opaque per-Network relay reservation issued by the Core adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct ServiceRelayLease([u8; 16]);

impl ServiceRelayLease {
    /// Construct a relay lease at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl core::fmt::Debug for ServiceRelayLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ServiceRelayLease(<redacted>)")
    }
}

/// Opaque proof that this Service, rather than a foreign owner, bound the bus.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnedBusBinding([u8; 16]);

impl OwnedBusBinding {
    /// Construct a binding proof at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl core::fmt::Debug for OwnedBusBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("OwnedBusBinding(<redacted>)")
    }
}

/// Closed USB Service lifecycle phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePhase {
    /// The Zone has not opted into this Service.
    WaitingForOptIn,
    /// Host-global authority is being reserved before effects.
    ReservingAuthority,
    /// The exact owned physical device is bound and may admit Bindings.
    Bound,
    /// Bindings must close before the Service may unbind the device.
    DrainingBindings,
    /// The owned binding is closed and authorities are being released.
    Releasing,
    /// All owned effects and authority have closed.
    Closed,
    /// A terminal safe-mutation error requires an operator-visible refusal.
    Blocked,
}

/// Closed Service lifecycle failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLifecycleError {
    /// The request originated from another Zone.
    WrongZone,
    /// The owning Zone did not opt into USBIP.
    ZoneNotOptedIn,
    /// The physical backing is already claimed by another owner.
    PhysicalAuthorityConflict,
    /// The Network relay is already claimed by another owner.
    RelayAuthorityConflict,
    /// A brokered effect can be retried with retained authority.
    Transient,
    /// A foreign binding or marker blocked a safe mutation.
    ForeignOwnership,
    /// The requested transition is not valid for the current state.
    InvalidState,
}

impl ServiceLifecycleError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongZone => "wrong-zone",
            Self::ZoneNotOptedIn => "zone-not-opted-in",
            Self::PhysicalAuthorityConflict => "physical-usb-backing-conflict",
            Self::RelayAuthorityConflict => "usbip-network-relay-authority-conflict",
            Self::Transient => "transient",
            Self::ForeignOwnership => "foreign-ownership",
            Self::InvalidState => "invalid-state",
        }
    }
}

impl core::fmt::Display for ServiceLifecycleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ServiceLifecycleError {}

/// Port through which a Service reaches the Core authority and broker adapter.
///
/// Implementations reserve through `HostGlobalAuthorityIndex` before invoking
/// typed broker operations and retain each lease until this lifecycle confirms
/// closure.
pub trait ServicePort {
    /// Reserve the shared physical USB backing before any effect starts.
    fn reserve_physical(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<PhysicalAuthorityLease, ServiceLifecycleError>;

    /// Reserve the per-Network relay authority before any effect starts.
    fn reserve_relay(
        &mut self,
        service_uid: &ResourceUid,
    ) -> Result<ServiceRelayLease, ServiceLifecycleError>;

    /// Bind the exact physical device held by the supplied authority lease.
    fn bind_owned(
        &mut self,
        physical: &PhysicalAuthorityLease,
    ) -> Result<OwnedBusBinding, ServiceLifecycleError>;

    /// Unbind only the exact device represented by the supplied ownership proof.
    fn unbind_owned(&mut self, binding: &OwnedBusBinding) -> Result<(), ServiceLifecycleError>;

    /// Release relay authority after the owned bus binding has closed.
    fn release_relay(&mut self, relay: ServiceRelayLease) -> Result<(), ServiceLifecycleError>;

    /// Release physical authority only after all owned effects have closed.
    fn release_physical(
        &mut self,
        physical: PhysicalAuthorityLease,
    ) -> Result<(), ServiceLifecycleError>;
}

/// Service lifecycle state, distinct from every Binding lifecycle.
pub struct ServiceLifecycle {
    zone_uid: ResourceUid,
    service_uid: ResourceUid,
    phase: ServicePhase,
    physical: Option<PhysicalAuthorityLease>,
    relay: Option<ServiceRelayLease>,
    binding: Option<OwnedBusBinding>,
}

impl ServiceLifecycle {
    /// Construct an inactive Service in its owning Zone.
    pub const fn new(zone_uid: ResourceUid, service_uid: ResourceUid) -> Self {
        Self {
            zone_uid,
            service_uid,
            phase: ServicePhase::WaitingForOptIn,
            physical: None,
            relay: None,
            binding: None,
        }
    }

    /// Return the closed Service lifecycle phase.
    pub const fn phase(&self) -> ServicePhase {
        self.phase
    }

    /// Whether the Service has an owned bus binding and may admit Bindings.
    pub const fn ready_for_bindings(&self) -> bool {
        matches!(self.phase, ServicePhase::Bound)
    }

    /// Reserve both Host-global authorities before binding the owned device.
    pub fn activate<P: ServicePort>(
        &mut self,
        zone_opted_in: bool,
        request_zone: ResourceUid,
        port: &mut P,
    ) -> Result<(), ServiceLifecycleError> {
        if request_zone != self.zone_uid {
            return Err(ServiceLifecycleError::WrongZone);
        }
        if !zone_opted_in {
            return Err(ServiceLifecycleError::ZoneNotOptedIn);
        }
        if self.ready_for_bindings() {
            return Ok(());
        }
        if matches!(self.phase, ServicePhase::Closed) {
            return Err(ServiceLifecycleError::InvalidState);
        }

        self.phase = ServicePhase::ReservingAuthority;
        if self.physical.is_none() {
            self.physical = Some(port.reserve_physical(&self.service_uid)?);
        }
        if self.relay.is_none() {
            self.relay = Some(port.reserve_relay(&self.service_uid)?);
        }
        if self.binding.is_none() {
            let physical = self
                .physical
                .as_ref()
                .ok_or(ServiceLifecycleError::InvalidState)?;
            self.binding = Some(port.bind_owned(physical)?);
        }
        self.phase = ServicePhase::Bound;
        Ok(())
    }

    /// Close the owned binding after every Binding has closed its process.
    ///
    /// Retained state is deliberately left intact after an effect failure so a
    /// supervisor can retry without admitting a competing owner.
    fn finalize_after_bindings_drain<P: ServicePort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), ServiceLifecycleError> {
        if self.phase == ServicePhase::Closed {
            return Ok(());
        }
        self.phase = ServicePhase::Releasing;
        if let Some(binding) = self.binding.as_ref() {
            port.unbind_owned(binding)?;
            self.binding = None;
        }
        if let Some(relay) = self.relay.take()
            && let Err(error) = port.release_relay(relay.clone())
        {
            self.relay = Some(relay);
            return Err(error);
        }
        if let Some(physical) = self.physical.take()
            && let Err(error) = port.release_physical(physical.clone())
        {
            self.physical = Some(physical);
            return Err(error);
        }
        self.phase = ServicePhase::Closed;
        Ok(())
    }
}

impl core::fmt::Debug for ServiceLifecycle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ServiceLifecycle")
            .field("phase", &self.phase)
            .field("has_physical_authority", &self.physical.is_some())
            .field("has_relay_authority", &self.relay.is_some())
            .field("has_owned_bus_binding", &self.binding.is_some())
            .finish()
    }
}

/// Verified pidfd/start-time identity for a broker-spawned attach runner.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachProcessIdentity {
    pid: u32,
    start_time: u64,
}

impl AttachProcessIdentity {
    /// Construct a verified process identity at the broker adapter boundary.
    pub const fn from_adapter(pid: u32, start_time: u64) -> Self {
        Self { pid, start_time }
    }
}

impl core::fmt::Debug for AttachProcessIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttachProcessIdentity(<redacted>)")
    }
}

/// Opaque Binding identity resolved from the trusted Resource graph.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingIdentity(ResourceUid);

impl BindingIdentity {
    /// Construct a Binding identity at the trusted controller boundary.
    pub const fn from_controller(value: ResourceUid) -> Self {
        Self(value)
    }

    /// Borrow the opaque resource identity for a daemon adapter.
    pub const fn as_resource_uid(&self) -> &ResourceUid {
        &self.0
    }
}

impl core::fmt::Debug for BindingIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BindingIdentity(<redacted>)")
    }
}

/// Opaque Service admission slot for one exact Binding.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingSlotLease([u8; 16]);

impl BindingSlotLease {
    /// Construct a slot lease at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl core::fmt::Debug for BindingSlotLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BindingSlotLease(<redacted>)")
    }
}

/// Opaque private proxy and Endpoint ownership for one exact Binding.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingProxyLease([u8; 16]);

impl BindingProxyLease {
    /// Construct a proxy lease at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl core::fmt::Debug for BindingProxyLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BindingProxyLease(<redacted>)")
    }
}

/// Restart observation of a previously stored attach-runner identity.
#[derive(Clone, PartialEq, Eq)]
pub enum AttachmentObservation {
    /// A live runner matches the stored pidfd/start-time identity.
    Matching {
        /// The exact Service admission slot restored with the runner.
        slot: BindingSlotLease,
        /// The exact private proxy restored with the runner.
        proxy: BindingProxyLease,
    },
    /// No runner exists and normal reconciliation may create one.
    Missing,
    /// A pid or bus identity was reused or could not be proven.
    StaleIdentity,
}

impl core::fmt::Debug for AttachmentObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Matching { .. } => formatter.write_str("AttachmentObservation::Matching"),
            Self::Missing => formatter.write_str("AttachmentObservation::Missing"),
            Self::StaleIdentity => formatter.write_str("AttachmentObservation::StaleIdentity"),
        }
    }
}

/// Closed Binding lifecycle phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingPhase {
    /// The Binding is not attached.
    WaitingForService,
    /// A slot, proxy, or runner is being acquired.
    Attaching,
    /// The Binding owns a Guest attachment through its private proxy.
    Attached,
    /// Restart observation was ambiguous; no destructive mutation is allowed.
    Quarantined,
    /// Guest, runner, proxy, and slot cleanup is underway.
    Releasing,
    /// Every Binding-owned effect is closed.
    Closed,
    /// A terminal safe-mutation failure blocked progress.
    Blocked,
}

/// Closed Binding lifecycle failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingLifecycleError {
    /// Binding and Service are not in the same Zone.
    WrongZone,
    /// The Service has not finished its physical bind lifecycle.
    ServiceNotReady,
    /// A slot or private proxy could not be admitted.
    AdmissionDenied,
    /// A broker-spawned attach runner could be retried.
    Transient,
    /// A foreign or ambiguous identity blocked safe mutation.
    ForeignIdentity,
    /// Restart recovery has not resolved an ambiguous prior identity.
    Quarantined,
}

impl BindingLifecycleError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongZone => "wrong-zone",
            Self::ServiceNotReady => "service-not-ready",
            Self::AdmissionDenied => "binding-admission-denied",
            Self::Transient => "transient",
            Self::ForeignIdentity => "foreign-identity",
            Self::Quarantined => "quarantined",
        }
    }
}

impl core::fmt::Display for BindingLifecycleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for BindingLifecycleError {}

/// Port through which a Binding reaches its supervisor and brokered runner.
pub trait BindingPort {
    /// Reserve the Service admission slot for this exact Binding.
    fn acquire_slot(
        &mut self,
        binding: &BindingIdentity,
    ) -> Result<BindingSlotLease, BindingLifecycleError>;

    /// Start the Binding-private proxy and Endpoint.
    fn start_proxy(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<BindingProxyLease, BindingLifecycleError>;

    /// Spawn the one-shot attach runner through d2bd's `SpawnRunner` path.
    fn spawn_attach_runner(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<AttachProcessIdentity, BindingLifecycleError>;

    /// Verify a persisted attach runner by pidfd and start-time identity.
    fn observe_attach_runner(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<AttachmentObservation, BindingLifecycleError>;

    /// Detach the Guest through the Binding-private Endpoint.
    fn detach_guest(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError>;

    /// Close the exact broker-spawned attach runner after Guest detach.
    fn close_attach_runner(
        &mut self,
        binding: &BindingIdentity,
        identity: &AttachProcessIdentity,
    ) -> Result<(), BindingLifecycleError>;

    /// Remove the Binding-private proxy and Endpoint.
    fn close_proxy(
        &mut self,
        binding: &BindingIdentity,
        proxy: &BindingProxyLease,
    ) -> Result<(), BindingLifecycleError>;

    /// Release the Service admission slot after proxy closure.
    fn release_slot(
        &mut self,
        binding: &BindingIdentity,
        slot: &BindingSlotLease,
    ) -> Result<(), BindingLifecycleError>;
}

/// Binding lifecycle state, which never owns Service unbind or authority release.
pub struct BindingLifecycle {
    service_zone_uid: ResourceUid,
    binding_zone_uid: ResourceUid,
    identity: BindingIdentity,
    phase: BindingPhase,
    slot: Option<BindingSlotLease>,
    proxy: Option<BindingProxyLease>,
    attach: Option<AttachProcessIdentity>,
}

impl BindingLifecycle {
    /// Construct an inactive Binding for a Service and Guest Zone.
    pub const fn new(
        service_zone_uid: ResourceUid,
        binding_zone_uid: ResourceUid,
        identity: BindingIdentity,
    ) -> Self {
        Self {
            service_zone_uid,
            binding_zone_uid,
            identity,
            phase: BindingPhase::WaitingForService,
            slot: None,
            proxy: None,
            attach: None,
        }
    }

    /// Return the closed Binding lifecycle phase.
    pub const fn phase(&self) -> BindingPhase {
        self.phase
    }

    /// Whether every Binding-owned effect has been confirmed closed.
    pub const fn is_closed(&self) -> bool {
        matches!(self.phase, BindingPhase::Closed)
    }

    fn is_same_zone(&self) -> bool {
        self.service_zone_uid == self.binding_zone_uid
    }

    /// Acquire the Service slot, then proxy, then brokered attach runner.
    fn activate<P: BindingPort>(
        &mut self,
        service: &ServiceLifecycle,
        port: &mut P,
    ) -> Result<(), BindingLifecycleError> {
        if !self.is_same_zone() {
            return Err(BindingLifecycleError::WrongZone);
        }
        if self.phase == BindingPhase::Quarantined {
            return Err(BindingLifecycleError::Quarantined);
        }
        if !service.ready_for_bindings() {
            return Err(BindingLifecycleError::ServiceNotReady);
        }
        if matches!(self.phase, BindingPhase::Attached) {
            return Ok(());
        }
        self.phase = BindingPhase::Attaching;
        if self.slot.is_none() {
            self.slot = Some(port.acquire_slot(&self.identity)?);
        }
        if self.proxy.is_none() {
            let slot = self
                .slot
                .as_ref()
                .ok_or(BindingLifecycleError::AdmissionDenied)?;
            self.proxy = Some(port.start_proxy(&self.identity, slot)?);
        }
        if self.attach.is_none() {
            let proxy = self
                .proxy
                .as_ref()
                .ok_or(BindingLifecycleError::AdmissionDenied)?;
            self.attach = Some(port.spawn_attach_runner(&self.identity, proxy)?);
        }
        self.phase = BindingPhase::Attached;
        Ok(())
    }

    /// Adopt only a matching attach process; quarantine stale identity without
    /// starting, closing, or unbinding any host effect.
    fn adopt<P: BindingPort>(
        &mut self,
        identity: AttachProcessIdentity,
        port: &mut P,
    ) -> Result<(), BindingLifecycleError> {
        if !self.is_same_zone() {
            return Err(BindingLifecycleError::WrongZone);
        }
        match port.observe_attach_runner(&self.identity, &identity)? {
            AttachmentObservation::Matching { slot, proxy } => {
                self.slot = Some(slot);
                self.proxy = Some(proxy);
                self.attach = Some(identity);
                self.phase = BindingPhase::Attached;
            }
            AttachmentObservation::Missing => {
                self.attach = None;
                self.slot = None;
                self.proxy = None;
                self.phase = BindingPhase::WaitingForService;
            }
            AttachmentObservation::StaleIdentity => {
                self.attach = None;
                self.phase = BindingPhase::Quarantined;
            }
        }
        Ok(())
    }

    /// Detach the Guest, close the owned runner and proxy, then release the
    /// slot. It intentionally has no access to Service authority or unbind.
    fn finalize<P: BindingPort>(&mut self, port: &mut P) -> Result<(), BindingLifecycleError> {
        if self.is_closed() {
            return Ok(());
        }
        if self.phase == BindingPhase::Quarantined {
            return Err(BindingLifecycleError::Quarantined);
        }
        self.phase = BindingPhase::Releasing;
        if let Some(identity) = self.attach.as_ref() {
            let proxy = self
                .proxy
                .as_ref()
                .ok_or(BindingLifecycleError::AdmissionDenied)?;
            port.detach_guest(&self.identity, proxy)?;
            port.close_attach_runner(&self.identity, identity)?;
            self.attach = None;
        }
        if let Some(proxy) = self.proxy.as_ref() {
            port.close_proxy(&self.identity, proxy)?;
            self.proxy = None;
        }
        if let Some(slot) = self.slot.as_ref() {
            port.release_slot(&self.identity, slot)?;
            self.slot = None;
        }
        self.phase = BindingPhase::Closed;
        Ok(())
    }
}

impl core::fmt::Debug for BindingLifecycle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingLifecycle")
            .field("phase", &self.phase)
            .field("has_slot", &self.slot.is_some())
            .field("has_proxy", &self.proxy.is_some())
            .field("has_attach_runner", &self.attach.is_some())
            .finish()
    }
}

/// Supervisor that coordinates separate Service and Binding finalizers.
///
/// It is the only public path that can finalize a Service, so a caller cannot
/// release Host-global authority until every Binding has detached its Guest,
/// closed its brokered runner, removed its private proxy, and released its
/// Service slot.
pub struct UsbipSupervisor {
    service: ServiceLifecycle,
    bindings: Vec<BindingLifecycle>,
}

impl UsbipSupervisor {
    /// Construct a supervisor for one Service lifecycle.
    pub const fn new(service: ServiceLifecycle) -> Self {
        Self {
            service,
            bindings: Vec::new(),
        }
    }

    /// Borrow the supervised Service lifecycle.
    pub const fn service(&self) -> &ServiceLifecycle {
        &self.service
    }

    /// Add one Binding that is already validated to reference this Service.
    pub fn add_binding(&mut self, binding: BindingLifecycle) -> Result<(), BindingLifecycleError> {
        if binding.service_zone_uid != self.service.zone_uid || !binding.is_same_zone() {
            return Err(BindingLifecycleError::WrongZone);
        }
        self.bindings.push(binding);
        Ok(())
    }

    /// Activate the Binding at `index` after its Service becomes ready.
    pub fn activate_binding<P: BindingPort>(
        &mut self,
        index: usize,
        port: &mut P,
    ) -> Result<(), BindingLifecycleError> {
        let binding = self
            .bindings
            .get_mut(index)
            .ok_or(BindingLifecycleError::AdmissionDenied)?;
        binding.activate(&self.service, port)
    }

    /// Restore Binding-owned effects after a daemon restart without starting
    /// replacement work until the persisted runner identity is verified.
    pub fn adopt_binding<P: BindingPort>(
        &mut self,
        index: usize,
        identity: AttachProcessIdentity,
        port: &mut P,
    ) -> Result<(), BindingLifecycleError> {
        let binding = self
            .bindings
            .get_mut(index)
            .ok_or(BindingLifecycleError::AdmissionDenied)?;
        binding.adopt(identity, port)
    }

    /// Finalize one Binding without unbinding the Service or releasing
    /// Host-global authority needed by its remaining Bindings.
    pub fn finalize_binding<P: BindingPort>(
        &mut self,
        index: usize,
        port: &mut P,
    ) -> Result<(), BindingLifecycleError> {
        let binding = self
            .bindings
            .get_mut(index)
            .ok_or(BindingLifecycleError::AdmissionDenied)?;
        binding.finalize(port)
    }

    /// Drain every Binding before unbinding the owned device and releasing
    /// relay then physical Host-global authority.
    pub fn finalize<P: ServicePort + BindingPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SupervisorFinalizeError> {
        self.service.phase = ServicePhase::DrainingBindings;
        for binding in &mut self.bindings {
            binding
                .finalize(port)
                .map_err(SupervisorFinalizeError::Binding)?;
        }
        self.service
            .finalize_after_bindings_drain(port)
            .map_err(SupervisorFinalizeError::Service)
    }
}

impl core::fmt::Debug for UsbipSupervisor {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("UsbipSupervisor")
            .field("service", &self.service)
            .field("binding_count", &self.bindings.len())
            .finish()
    }
}

/// Closed errors emitted while the supervisor drains a lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorFinalizeError {
    /// A Binding effect has not safely closed.
    Binding(BindingLifecycleError),
    /// A Service effect or authority release has not safely closed.
    Service(ServiceLifecycleError),
}

impl SupervisorFinalizeError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Binding(error) => error.code(),
            Self::Service(error) => error.code(),
        }
    }
}

impl core::fmt::Display for SupervisorFinalizeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SupervisorFinalizeError {}
