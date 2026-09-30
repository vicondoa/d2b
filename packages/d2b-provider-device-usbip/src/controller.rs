//! USB Service firewall and relay lifecycle controller.
//!
//! The semantic USB Service and Binding compose primitive relationships rather
//! than reaching a host grant. Each of them *requests* a typed binding -
//! [`usbip_relay_network_request`] for the relay's membership on its Network,
//! [`usbip_relay_endpoint_request`] for the relay Endpoint, and
//! [`usbip_guest_endpoint_request`] for the per-Guest Endpoint - and the
//! Service's physical backing is requested as a `DeviceBindingRequest` (see
//! [`crate::arbitration::usbip_service_device_request`]) and arbitrated by
//! the `Device` source.
//!
//! [`UsbipController::reconcile_claim`] and
//! [`UsbipController::finalize_claim`] are the converted path: every relay,
//! projection, and teardown step carries the one admitted
//! [`AdmittedDeviceClaim`] the source decided, a cross-Zone or stale claim is
//! refused before any effect is attempted, and the source reservation is given
//! up only after the projection is removed and the relay leg is stopped. The
//! pre-graph methods below them stay for the not-yet-cutover production path
//! and are queued for deletion in U34.

use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildSet;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingContractError, BindingRefusal, BindingSlot, DeviceEffectOperation,
    EndpointAttachmentKind, EndpointBindingRequest, NetworkBindingRequest, NetworkMembership,
    NetworkPresentation, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneId, execution_policy::BoundedToken,
};

use crate::arbitration::{AdmittedDeviceClaim, BoundDeviceLeg};
use crate::binding_child_resources;
use crate::firewall::{
    ClaimProjectionFence, FirewallConfirmationKind, FirewallDigest, FirewallGenerationFence,
    FirewallProjectionAction, FirewallProjectionIntent, FirewallToken, RelayAuthorityLease,
    UsbipClaimPort, UsbipEffectError, UsbipEffectPort,
};

/// The stable consumer slot the relay's Network membership occupies.
pub const USBIP_RELAY_NETWORK_SLOT: &str = "relay-network";

/// The bounded purpose the relay Endpoint is admitted for.
pub const USBIP_RELAY_ENDPOINT_PURPOSE: &str = "usb-relay";

/// The effect operation classes a USBIP realization drives.
///
/// The relay is a helper of the Service's device claim, so it spawns the
/// per-Network relay and applies its projection. The list is a subset the leg
/// must cover; it never widens what the source admitted.
pub const USBIP_RELAY_OPERATIONS: [DeviceEffectOperation; 2] = [
    DeviceEffectOperation::SpawnRunner,
    DeviceEffectOperation::ApplyNftablesProjection,
];

/// Build the canonical `NetworkBindingRequest` the per-Network relay is.
///
/// The relay consumes the Network, not the Service: a semantic USB Service
/// does not hold a fabric, and this request is what the Network source admits
/// before any listener exists.
///
/// # Errors
///
/// Returns [`BindingContractError`] when the source is not a `Network`.
pub fn usbip_relay_network_request(
    network_ref: &ResourceRef,
) -> Result<NetworkBindingRequest, BindingContractError> {
    NetworkBindingRequest::new(
        network_ref.clone(),
        service_controller_ref(),
        BindingSlot::parse(USBIP_RELAY_NETWORK_SLOT).map_err(|_| BindingContractError::InvalidField)?,
        NetworkMembership::new(Vec::new(), false)?,
        NetworkPresentation::shared_fabric(),
    )
}

/// Build the canonical `EndpointBindingRequest` the relay Endpoint is.
///
/// The relay accepts connections for the Service, so the relationship is a
/// listen attachment delivered to the Service's own controller. A Guest
/// cannot widen this by naming the endpoint: a different consumer, a
/// different slot, or a different attachment kind is a different request.
///
/// # Errors
///
/// Returns [`BindingContractError`] when the source is not an `Endpoint`.
pub fn usbip_relay_endpoint_request(
    endpoint_ref: &ResourceRef,
) -> Result<EndpointBindingRequest, BindingContractError> {
    EndpointBindingRequest::new(
        endpoint_ref.clone(),
        service_controller_ref(),
        BindingSlot::parse(USBIP_RELAY_NETWORK_SLOT).map_err(|_| BindingContractError::InvalidField)?,
        EndpointAttachmentKind::Listen,
        relay_purpose()?,
    )
}

/// Build the canonical `EndpointBindingRequest` one Binding's Guest Endpoint
/// is.
///
/// `consumer` is the Binding's own declared guest-proxy `Process`: the
/// per-Guest attachment is delivered to the helper that realizes the Binding,
/// never to the Guest as a whole and never to the host directory that happens
/// to contain the socket (R23).
///
/// # Errors
///
/// Returns [`BindingContractError`] when the source is not an `Endpoint` or
/// the consumer is not one this binding kind admits.
pub fn usbip_guest_endpoint_request(
    endpoint_ref: &ResourceRef,
    consumer: &ResourceRef,
) -> Result<EndpointBindingRequest, BindingContractError> {
    EndpointBindingRequest::new(
        endpoint_ref.clone(),
        consumer.clone(),
        BindingSlot::parse(USBIP_GUEST_ENDPOINT_SLOT).map_err(|_| BindingContractError::InvalidField)?,
        EndpointAttachmentKind::Connect,
        relay_purpose()?,
    )
}

/// The stable consumer slot a Binding's Guest Endpoint occupies.
const USBIP_GUEST_ENDPOINT_SLOT: &str = "guest-endpoint";

/// The one `Process` that consumes a USB Service's Device, Network, and relay
/// Endpoint relationships.
fn service_controller_ref() -> ResourceRef {
    ResourceRef::parse(crate::driver::USBIP_SERVICE_CONTROLLER_REF)
        .expect("the USBIP Service controller reference is canonical")
}

/// The bounded purpose both USBIP Endpoint relationships are admitted for.
fn relay_purpose() -> Result<BoundedToken, BindingContractError> {
    BoundedToken::parse(USBIP_RELAY_ENDPOINT_PURPOSE).map_err(|_| BindingContractError::InvalidField)
}

/// Default descriptor repair interval.
pub const USBIP_REPAIR_INTERVAL_SECS: u64 = 30;
/// Maximum descriptor repair interval.
pub const USBIP_MAX_REPAIR_INTERVAL_SECS: u64 = 60;
/// Service finalizer owned by the USBIP Provider.
pub const USBIP_SERVICE_FINALIZER: &str = "device-usbip.d2bus.org/service-finalizer";
/// Binding finalizer owned by the USBIP Provider.
pub const USBIP_BINDING_FINALIZER: &str = "device-usbip.d2bus.org/binding-finalizer";

/// The cutover contract for the USB Service and Binding owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipRunnerContract {
    service_resource_type: &'static str,
    binding_resource_type: &'static str,
    repair_interval_secs: u64,
    watched_configuration_is_dependency: bool,
}

impl UsbipRunnerContract {
    /// Return the provider-neutral Service ResourceType.
    pub const fn service_resource_type(self) -> &'static str {
        self.service_resource_type
    }

    /// Return the provider-neutral Binding ResourceType.
    pub const fn binding_resource_type(self) -> &'static str {
        self.binding_resource_type
    }

    /// Return the bounded repair interval.
    pub const fn repair_interval_secs(self) -> u64 {
        self.repair_interval_secs
    }

    /// Whether watched configuration is treated as a dependency.
    pub const fn watched_configuration_is_dependency(self) -> bool {
        self.watched_configuration_is_dependency
    }
}

/// Return the one shared-Runner registration for USBIP.
pub const fn usbip_runner_contract() -> UsbipRunnerContract {
    UsbipRunnerContract {
        service_resource_type: crate::USB_SERVICE_RESOURCE_TYPE,
        binding_resource_type: crate::USB_BINDING_RESOURCE_TYPE,
        repair_interval_secs: USBIP_REPAIR_INTERVAL_SECS,
        watched_configuration_is_dependency: true,
    }
}

/// Closed USB Binding lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipBindingPhase {
    /// Child resources are being admitted.
    Pending,
    /// Child resources are ready for attachment observation.
    Ready,
    /// A child resource or attachment is temporarily unavailable.
    Degraded,
    /// Child resources are draining.
    Deleted,
}

/// Exact Core admission for one USB Binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipBindingAdmission {
    zone_uid: ResourceUid,
    binding_uid: ResourceUid,
    service_uid: ResourceUid,
    guest_uid: ResourceUid,
    service_generation: ResourceGeneration,
    assignment_epoch: u64,
}

impl UsbipBindingAdmission {
    /// Construct an admission bound to one Zone, Service, Guest, and
    /// assignment generation.
    pub fn new(
        zone_uid: ResourceUid,
        binding_uid: ResourceUid,
        service_uid: ResourceUid,
        guest_uid: ResourceUid,
        service_generation: ResourceGeneration,
        assignment_epoch: u64,
    ) -> Result<Self, UsbipBindingControllerError> {
        if assignment_epoch == 0 {
            return Err(UsbipBindingControllerError::InvalidAdmission);
        }
        Ok(Self {
            zone_uid,
            binding_uid,
            service_uid,
            guest_uid,
            service_generation,
            assignment_epoch,
        })
    }

    /// Borrow the admitted Zone identity.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the Binding UID.
    pub const fn binding_uid(&self) -> &ResourceUid {
        &self.binding_uid
    }

    /// Borrow the Service UID.
    pub const fn service_uid(&self) -> &ResourceUid {
        &self.service_uid
    }

    /// Borrow the Guest UID.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Return the admitted Service generation.
    pub const fn service_generation(&self) -> ResourceGeneration {
        self.service_generation
    }

    /// Return the exact assignment epoch.
    pub const fn assignment_epoch(&self) -> u64 {
        self.assignment_epoch
    }
}

/// USB Binding reconcile output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipBindingReconcileResult {
    /// Binding lifecycle phase.
    pub phase: UsbipBindingPhase,
    /// UID-free Process and Endpoint intents.
    pub children: BindingChildSet,
}

/// Controller-level errors for USB Binding child admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipBindingControllerError {
    /// Binding, Service, target, or Provider references were not admitted.
    Admission,
    /// The Core assignment admission was malformed.
    InvalidAdmission,
    /// A newer assignment must be read before this Binding can continue.
    StaleAssignment,
    /// Reconciliation was requested after finalization.
    Finalized,
}

impl core::fmt::Display for UsbipBindingControllerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Admission => "usbip-binding-controller-admission-failed",
            Self::InvalidAdmission => "usbip-binding-controller-admission-invalid",
            Self::StaleAssignment => "usbip-binding-controller-assignment-stale",
            Self::Finalized => "usbip-binding-controller-finalized",
        })
    }
}

impl std::error::Error for UsbipBindingControllerError {}

/// Provider-owned USB Binding controller.
///
/// This controller declares and observes child resources. Host bind,
/// attachment launch, adoption, signalling, and reap stay behind the generic
/// resource runtime and the typed lifecycle port.
pub struct UsbipBindingController {
    binding_ref: ResourceRef,
    service_ref: ResourceRef,
    target_ref: ResourceRef,
    children: BindingChildSet,
    phase: UsbipBindingPhase,
    admission: Option<UsbipBindingAdmission>,
}

impl core::fmt::Debug for UsbipBindingController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("UsbipBindingController")
            .field("phase", &self.phase)
            .field("binding_ref", &self.binding_ref)
            .field("service_ref", &self.service_ref)
            .field("target_ref", &self.target_ref)
            .field("children", &self.children)
            .field("has_admission", &self.admission.is_some())
            .finish()
    }
}

impl UsbipBindingController {
    /// Construct a Binding controller from explicit authored references.
    ///
    /// # Errors
    ///
    /// Returns [`UsbipBindingControllerError::Admission`] when the binding,
    /// service, or target reference has the wrong resource type or the
    /// binding child declaration fails.
    pub fn new(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
    ) -> Result<Self, UsbipBindingControllerError> {
        if binding_ref.resource_type().as_str() != crate::USB_BINDING_RESOURCE_TYPE
            || service_ref.resource_type().as_str() != crate::USB_SERVICE_RESOURCE_TYPE
            || target_ref.resource_type().as_str() != "Guest"
        {
            tracing::warn!(
                binding = %binding_ref.to_canonical_string(),
                service = %service_ref.to_canonical_string(),
                reason = "binding, service, or target reference has the wrong resource type",
                "usbip binding controller construction refused",
            );
            return Err(UsbipBindingControllerError::Admission);
        }
        let children = binding_child_resources(binding_ref, service_ref, target_ref)
            .map_err(|error| {
                tracing::warn!(
                    binding = %binding_ref.to_canonical_string(),
                    service = %service_ref.to_canonical_string(),
                    error = %error,
                    "usbip binding child declaration failed",
                );
                UsbipBindingControllerError::Admission
            })?;
        Ok(Self {
            binding_ref: binding_ref.clone(),
            service_ref: service_ref.clone(),
            target_ref: target_ref.clone(),
            children,
            phase: UsbipBindingPhase::Pending,
            admission: None,
        })
    }

    /// Construct a Binding controller from exact Core assignment evidence.
    pub fn new_admitted(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
        admission: UsbipBindingAdmission,
    ) -> Result<Self, UsbipBindingControllerError> {
        let mut controller = Self::new(binding_ref, service_ref, target_ref)?;
        controller.validate_admission(&admission)?;
        controller.admission = Some(admission);
        Ok(controller)
    }

    /// Return the current Binding lifecycle phase.
    pub const fn phase(&self) -> UsbipBindingPhase {
        self.phase
    }

    /// Borrow the current child intents.
    pub const fn children(&self) -> &BindingChildSet {
        &self.children
    }

    /// Borrow the exact Core assignment admission, when one was supplied.
    pub const fn admission(&self) -> Option<&UsbipBindingAdmission> {
        self.admission.as_ref()
    }

    /// Whether a ResourceRef is one of this Binding's Process/Endpoint
    /// children. Volume ownership is never admitted by this controller.
    pub fn owns_child(&self, resource_ref: &ResourceRef) -> bool {
        matches!(
            resource_ref.resource_type().as_str(),
            "Process" | "Endpoint"
        ) && self.children.resource_refs().any(|current| current == resource_ref)
    }

    /// Build the canonical `EndpointBindingRequest` this Binding's declared
    /// guest Endpoint is.
    ///
    /// The consumer is the Binding's own declared `guest-proxy` child, so the
    /// per-Guest attachment is admitted for the helper that realizes the
    /// Binding. Naming a different consumer - including the Guest itself or
    /// the Service - is a different request and the Endpoint source has to
    /// admit it on its own evidence.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError`] when the declared guest-proxy child is
    /// missing, the source is not an `Endpoint`, or the consumer kind is not
    /// one this binding kind admits.
    pub fn endpoint_request(
        &self,
        endpoint_ref: &ResourceRef,
    ) -> Result<EndpointBindingRequest, BindingContractError> {
        let proxy = self
            .children
            .child("guest-proxy")
            .ok_or(BindingContractError::InvalidField)?
            .resource_ref();
        usbip_guest_endpoint_request(endpoint_ref, proxy)
    }

    /// Observe Core-managed child readiness without spawning a feature
    /// process.
    pub fn observe_children(
        &mut self,
        ready: bool,
    ) -> Result<UsbipBindingReconcileResult, UsbipBindingControllerError> {
        if self.phase == UsbipBindingPhase::Deleted {
            tracing::debug!(
                binding = %self.binding_ref.to_canonical_string(),
                reason = "binding already finalized",
                "usbip binding child observation refused",
            );
            return Err(UsbipBindingControllerError::Finalized);
        }
        self.phase = if ready {
            UsbipBindingPhase::Ready
        } else {
            UsbipBindingPhase::Degraded
        };
        Ok(UsbipBindingReconcileResult {
            phase: self.phase,
            children: self.children.clone(),
        })
    }

    /// Observe children after rechecking the exact assignment fence.
    pub fn observe_children_with_admission(
        &mut self,
        admission: UsbipBindingAdmission,
        ready: bool,
    ) -> Result<UsbipBindingReconcileResult, UsbipBindingControllerError> {
        self.validate_admission(&admission)?;
        if self.admission.is_none() {
            self.admission = Some(admission);
        }
        self.observe_children(ready)
    }

    /// Mark the Binding deleted after Endpoint, then Process children drain.
    pub fn finalize(&mut self) {
        self.phase = UsbipBindingPhase::Deleted;
    }

    /// Mark the Binding deleted after validating its current assignment.
    pub fn finalize_with_admission(
        &mut self,
        admission: UsbipBindingAdmission,
    ) -> Result<(), UsbipBindingControllerError> {
        self.validate_admission(&admission)?;
        self.finalize();
        Ok(())
    }

    fn validate_admission(
        &self,
        admission: &UsbipBindingAdmission,
    ) -> Result<(), UsbipBindingControllerError> {
        if admission.assignment_epoch() == 0 {
            tracing::warn!(
                binding = %self.binding_ref.to_canonical_string(),
                reason = "assignment epoch is zero",
                "usbip binding admission rejected",
            );
            return Err(UsbipBindingControllerError::InvalidAdmission);
        }
        if let Some(current) = self.admission.as_ref() {
            if current.zone_uid() != admission.zone_uid()
                || current.binding_uid() != admission.binding_uid()
                || current.service_uid() != admission.service_uid()
                || current.guest_uid() != admission.guest_uid()
                || current.service_generation() != admission.service_generation()
            {
                tracing::warn!(
                    binding = %self.binding_ref.to_canonical_string(),
                    reason = "admission zone, binding, service, guest, or generation differs",
                    "usbip binding admission rejected: fence mismatch",
                );
                return Err(UsbipBindingControllerError::Admission);
            }
            if current.assignment_epoch() != admission.assignment_epoch() {
                tracing::warn!(
                    binding = %self.binding_ref.to_canonical_string(),
                    reason = "assignment epoch moved backwards",
                    "usbip binding admission rejected: stale assignment",
                );
                return Err(UsbipBindingControllerError::StaleAssignment);
            }
        }
        Ok(())
    }
}

/// Zone-scoped opaque resource identity.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedResourceUid {
    zone_uid: ResourceUid,
    resource_uid: ResourceUid,
}

impl ScopedResourceUid {
    /// Bind an opaque resource identity to its exact Zone.
    pub const fn new(zone_uid: ResourceUid, resource_uid: ResourceUid) -> Self {
        Self {
            zone_uid,
            resource_uid,
        }
    }

    /// Borrow the Zone identity for equality checks only.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the opaque resource identity for the Core adapter.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }
}

impl core::fmt::Debug for ScopedResourceUid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ScopedResourceUid(<redacted>)")
    }
}

/// Network dependency surface visible to the Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkDependency {
    identity: ScopedResourceUid,
    generation: ResourceGeneration,
    ready: bool,
    assignment_epoch: Option<u64>,
}

impl NetworkDependency {
    /// Construct the bounded identity/readiness/generation projection.
    pub const fn new(
        identity: ScopedResourceUid,
        generation: ResourceGeneration,
        ready: bool,
    ) -> Self {
        Self {
            identity,
            generation,
            ready,
            assignment_epoch: None,
        }
    }

    /// Borrow the scoped Network identity.
    pub const fn identity(&self) -> &ScopedResourceUid {
        &self.identity
    }

    /// Return the observed Network generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Whether the Network is Ready for the relay dependency.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    /// Bind the dependency to an exact Core assignment epoch.
    pub fn with_assignment_epoch(
        mut self,
        assignment_epoch: u64,
    ) -> Result<Self, UsbipEffectError> {
        if assignment_epoch == 0 {
            return Err(UsbipEffectError::StaleAssignment);
        }
        self.assignment_epoch = Some(assignment_epoch);
        Ok(self)
    }

    /// Return the assignment epoch, when Core supplied one.
    pub const fn assignment_epoch(&self) -> Option<u64> {
        self.assignment_epoch
    }
}

impl core::fmt::Debug for NetworkDependency {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NetworkDependency")
            .field("identity", &self.identity)
            .field("generation", &self.generation)
            .field("ready", &self.ready)
            .finish()
    }
}

/// Closed USB Service firewall lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipServicePhase {
    /// Waiting for a Ready Network dependency.
    WaitingForNetwork,
    /// Acquiring relay authority or applying the projection.
    Applying,
    /// Relay and firewall projection are confirmed Ready.
    Ready,
    /// Observation found ownership-scoped drift.
    Drifted,
    /// Projection removal is in progress while authority stays retained.
    Releasing,
    /// A terminal safe-mutation failure blocked progress.
    Blocked,
}

/// Closed controller operation label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipOperation {
    /// Acquire or share relay authority.
    AcquireRelay,
    /// Apply one projection.
    ApplyFirewall,
    /// Observe one projection.
    ObserveFirewall,
    /// Remove one projection.
    RemoveFirewall,
    /// Release relay authority.
    ReleaseRelay,
}

impl UsbipOperation {
    const fn label(self) -> &'static str {
        match self {
            Self::AcquireRelay => "acquire-relay",
            Self::ApplyFirewall => "apply-firewall",
            Self::ObserveFirewall => "observe-firewall",
            Self::RemoveFirewall => "remove-firewall",
            Self::ReleaseRelay => "release-relay",
        }
    }
}

/// Closed controller outcome label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipOutcome {
    /// Operation converged.
    Success,
    /// Operation is safe to retry.
    Retry,
    /// Operation was blocked fail closed.
    Blocked,
}

impl UsbipOutcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retry => "retry",
            Self::Blocked => "blocked",
        }
    }
}

/// The admitted relationship one USB Service realizes, as its own row declares it.
///
/// The controller compares the claim against exactly these facts - the Zone the
/// Service row lives in, the backing `Device` it names, the store incarnation
/// it is fenced against, and the helper whose bounded leg may realize it - so
/// the reconcile entry point takes one admission value rather than a list of
/// independently supplied arguments.
#[derive(Debug, Clone, Copy)]
pub struct UsbipServiceClaim<'a> {
    zone: &'a ZoneId,
    store: &'a StoreIncarnation,
    device_ref: &'a ResourceRef,
    claim: &'a AdmittedDeviceClaim,
    helper: &'a ResourceRef,
    helper_uid: &'a ResourceUid,
}

impl<'a> UsbipServiceClaim<'a> {
    /// Bind one admitted claim to the facts the Service row declares.
    pub const fn new(
        zone: &'a ZoneId,
        store: &'a StoreIncarnation,
        device_ref: &'a ResourceRef,
        claim: &'a AdmittedDeviceClaim,
        helper: &'a ResourceRef,
        helper_uid: &'a ResourceUid,
    ) -> Self {
        Self {
            zone,
            store,
            device_ref,
            claim,
            helper,
            helper_uid,
        }
    }

    /// The admitted `Device` claim.
    pub const fn claim(&self) -> &'a AdmittedDeviceClaim {
        self.claim
    }

    /// The helper whose bounded leg realizes the claim.
    pub const fn helper(&self) -> &'a ResourceRef {
        self.helper
    }

    /// The Zone the Service row lives in.
    pub const fn zone(&self) -> &'a ZoneId {
        self.zone
    }

    /// The store incarnation the admission is fenced against.
    pub const fn store(&self) -> &'a StoreIncarnation {
        self.store
    }
}

/// Bounded metric labels whose keys and values come from closed sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipMetricLabels {
    /// Fixed Provider label.
    pub provider: &'static str,
    /// Fixed semantic component label.
    pub component: &'static str,
    /// Closed operation label.
    pub operation: &'static str,
    /// Closed outcome label.
    pub outcome: &'static str,
    /// Closed error label or `none`.
    pub error: &'static str,
}

impl UsbipMetricLabels {
    /// Project controller state without any resource, Zone, device, caller, or
    /// supplied identity value.
    pub const fn new(
        operation: UsbipOperation,
        outcome: UsbipOutcome,
        error: Option<UsbipEffectError>,
    ) -> Self {
        Self {
            provider: "device-usbip",
            component: "service-controller",
            operation: operation.label(),
            outcome: outcome.label(),
            error: match error {
                Some(error) => error.code(),
                None => "none",
            },
        }
    }
}

struct FirewallLease {
    token: FirewallToken,
    digest: FirewallDigest,
    fence: FirewallGenerationFence,
    store: Option<StoreIncarnation>,
}

impl FirewallLease {
    /// The store incarnation this retained projection was written under.
    ///
    /// `None` for the pre-graph projection path, which was fenced on resource
    /// generations alone; the converted path always has one.
    const fn claim_store(&self) -> Option<&StoreIncarnation> {
        self.store.as_ref()
    }
}

impl core::fmt::Debug for FirewallLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallLease(<redacted>)")
    }
}

/// USB Service controller state for one physical backing and Network relay.
///
/// The converted path additionally retains the one admitted
/// [`AdmittedDeviceClaim`] the `Device` source arbitrated, the helper its relay
/// leg is bound to, and the store incarnation that admission was fenced
/// against. None of them is an authority of this Provider's own: they are the
/// evidence every effect below is measured against.
pub struct UsbipController {
    service: ScopedResourceUid,
    service_generation: ResourceGeneration,
    device_uid: ResourceUid,
    network: Option<NetworkDependency>,
    phase: UsbipServicePhase,
    relay: Option<RelayAuthorityLease>,
    firewall: Option<FirewallLease>,
    last_error: Option<UsbipEffectError>,
    network_assignment_epoch: Option<u64>,
    claim: Option<AdmittedDeviceClaim>,
    relay_helper: Option<ResourceRef>,
    store: Option<StoreIncarnation>,
    source_released: bool,
}

impl UsbipController {
    /// Construct one authority-Service controller with no acquired effect state.
    pub const fn new(
        service: ScopedResourceUid,
        service_generation: ResourceGeneration,
        device_uid: ResourceUid,
    ) -> Self {
        Self {
            service,
            service_generation,
            device_uid,
            network: None,
            phase: UsbipServicePhase::WaitingForNetwork,
            relay: None,
            firewall: None,
            last_error: None,
            network_assignment_epoch: None,
            claim: None,
            relay_helper: None,
            store: None,
            source_released: false,
        }
    }

    /// Return the closed lifecycle phase.
    pub const fn phase(&self) -> UsbipServicePhase {
        self.phase
    }

    /// Return the last closed error class.
    pub const fn last_error(&self) -> Option<UsbipEffectError> {
        self.last_error
    }

    /// Whether relay authority is currently retained.
    pub const fn relay_authority_retained(&self) -> bool {
        self.relay.is_some()
    }

    /// Whether firewall token/status is currently retained.
    pub const fn firewall_status_retained(&self) -> bool {
        self.firewall.is_some()
    }

    /// Borrow the admitted `Device` claim this Service realizes, once one has
    /// been accepted.
    pub const fn claim(&self) -> Option<&AdmittedDeviceClaim> {
        self.claim.as_ref()
    }

    /// Whether the source reservation has been handed back.
    ///
    /// Release is the last step of teardown, so this only becomes true after
    /// the projection is gone and the relay leg is stopped.
    pub const fn source_released(&self) -> bool {
        self.source_released
    }

    /// Reconcile the Ready Network dependency, relay authority, and exact
    /// ownership-scoped firewall projection.
    pub fn reconcile<P: UsbipEffectPort>(
        &mut self,
        network: NetworkDependency,
        port: &mut P,
    ) -> Result<(), UsbipControllerError> {
        self.validate_network(&network)?;
        self.phase = UsbipServicePhase::Applying;
        self.network = Some(network.clone());
        self.network_assignment_epoch = network.assignment_epoch();
        if self.relay.is_none() {
            match port.acquire_relay(network.identity().resource_uid()) {
                Ok(lease) => self.relay = Some(lease),
                Err(error) => return self.effect_failed(error),
            }
        }
        let fence = FirewallGenerationFence::new(network.generation(), self.service_generation);
        let intent = FirewallProjectionIntent::new(
            self.device_uid.clone(),
            network.identity().resource_uid().clone(),
            FirewallProjectionAction::Apply,
            fence.clone(),
        );
        match port.mutate_firewall(&intent, None) {
            Ok(confirmation) => {
                let Some((token, digest)) = confirmation.into_applied() else {
                    return self.effect_failed(UsbipEffectError::EffectRejected);
                };
                self.firewall = Some(FirewallLease {
                    token,
                    digest,
                    fence,
                    store: None,
                });
                self.last_error = None;
                self.phase = UsbipServicePhase::Ready;
                Ok(())
            }
            Err(error) => self.effect_failed(error),
        }
    }

    /// Observe only this Service's USBIP ownership projection.
    pub fn observe<P: UsbipEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), UsbipControllerError> {
        let network = self
            .network
            .as_ref()
            .ok_or(UsbipControllerError::InvalidState)
            .inspect_err(|_| {
                tracing::debug!(
                    device = %self.device_uid.to_canonical_string(),
                    reason = "observe called before reconcile established network and firewall",
                    "usbip service observation refused",
                );
            })?;
        let firewall = self
            .firewall
            .as_mut()
            .ok_or(UsbipControllerError::InvalidState)
            .inspect_err(|_| {
                tracing::debug!(
                    device = %self.device_uid.to_canonical_string(),
                    reason = "observe called before reconcile established network and firewall",
                    "usbip service observation refused",
                );
            })?;
        let intent = FirewallProjectionIntent::new(
            self.device_uid.clone(),
            network.identity().resource_uid().clone(),
            FirewallProjectionAction::Apply,
            firewall.fence.clone(),
        );
        match port.observe_firewall(&intent, &firewall.token) {
            Ok(observation) if observation.matches_expected() => {
                firewall.digest = observation.digest().clone();
                self.phase = UsbipServicePhase::Ready;
                self.last_error = None;
                Ok(())
            }
            Ok(_) => {
                self.phase = UsbipServicePhase::Drifted;
                tracing::warn!(
                    device = %self.device_uid.to_canonical_string(),
                    reason = "ownership-scoped firewall observation differs from desired state",
                    "usbip service firewall drifted",
                );
                Err(UsbipControllerError::FirewallDrift)
            }
            Err(error) => self.effect_failed(error),
        }
    }

    /// Remove the exact projection, then release relay authority only after a
    /// confirmed removal or ownership-validated absence.
    pub fn finalize<P: UsbipEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), UsbipControllerError> {
        self.phase = UsbipServicePhase::Releasing;
        if let Some(firewall) = self.firewall.as_ref() {
            let network = self
                .network
                .as_ref()
                .ok_or(UsbipControllerError::InvalidState)?;
            let intent = FirewallProjectionIntent::new(
                self.device_uid.clone(),
                network.identity().resource_uid().clone(),
                FirewallProjectionAction::Remove,
                firewall.fence.clone(),
            );
            match port.mutate_firewall(&intent, Some(&firewall.token)) {
                Ok(confirmation)
                    if matches!(
                        confirmation.kind(),
                        FirewallConfirmationKind::Removed
                            | FirewallConfirmationKind::ValidatedAbsent
                    ) =>
                {
                    self.firewall = None;
                }
                Ok(_) => return self.effect_failed(UsbipEffectError::EffectRejected),
                Err(error) => return self.effect_failed(error),
            }
        }
        if let Some(relay) = self.relay.take()
            && let Err(error) = port.release_relay(relay.clone())
        {
            self.relay = Some(relay);
            return self.effect_failed(error);
        }
        self.network = None;
        self.network_assignment_epoch = None;
        self.last_error = None;
        self.phase = UsbipServicePhase::WaitingForNetwork;
        Ok(())
    }

    /// Reconcile the relay and projection as bounded realizations of one
    /// admitted `Device` claim.
    ///
    /// The order of checks is the point: `claim` has to be the relationship
    /// this Service row declares, in the Zone the row lives in; `leg` has to be
    /// an attenuated realization of that same relationship and reservation,
    /// fenced against the same store incarnation, reaching the same capability
    /// and physical authority; and the Network dependency has to be Ready at a
    /// non-regressing assignment - all before the port is called even once. A
    /// cross-Zone claim, a stale store, a revoking relationship, or a leg that
    /// claims the device itself therefore leaves no relay, no listener, and no
    /// firewall rule behind (R8, R35).
    ///
    /// # Errors
    ///
    /// Returns [`UsbipControllerError::Effect`] with
    /// [`UsbipEffectError::ClaimRefused`] when the claim or the leg is refused,
    /// the closed network refusals when the dependency is not usable, and the
    /// port's own error when an effect fails after authority was admitted.
    pub fn reconcile_claim<L, P>(
        &mut self,
        admitted: &UsbipServiceClaim<'_>,
        leg: &L,
        network: NetworkDependency,
        port: &mut P,
    ) -> Result<(), UsbipControllerError>
    where
        L: BoundDeviceLeg + ?Sized,
        P: UsbipClaimPort,
    {
        let UsbipServiceClaim {
            zone,
            store,
            device_ref,
            claim,
            helper,
            helper_uid,
        } = *admitted;
        claim
            .verify_service_claim(zone, device_ref)
            .map_err(|refusal| self.claim_refusal_error(refusal))?;
        claim
            .verify_helper_leg(helper, helper_uid, &USBIP_RELAY_OPERATIONS, leg)
            .map_err(|refusal| self.claim_refusal_error(refusal))?;
        if self
            .store
            .as_ref()
            .is_some_and(|current| current != store)
        {
            return Err(self.stale_store_error());
        }
        let fence = self.claim_fence(claim, &network, store)?;
        self.validate_network(&network)?;
        self.store = Some(store.clone());
        // The claim is retained before the first effect so a failure part way
        // through still has a relationship to drain.
        self.claim = Some(claim.clone());
        self.relay_helper = Some(helper.clone());
        self.phase = UsbipServicePhase::Applying;
        self.network = Some(network.clone());
        self.network_assignment_epoch = network.assignment_epoch();
        if self.relay.is_none() {
            match port.start_relay_leg(
                claim,
                helper,
                network.identity().resource_uid(),
                &fence,
            ) {
                Ok(lease) => self.relay = Some(lease),
                Err(error) => return self.effect_failed(error),
            }
        }
        match port.mutate_claim_firewall(
            claim,
            network.identity().resource_uid(),
            FirewallProjectionAction::Apply,
            &fence,
            None,
        ) {
            Ok(confirmation) => {
                let Some((token, digest)) = confirmation.into_applied() else {
                    return self.effect_failed(UsbipEffectError::EffectRejected);
                };
                self.firewall = Some(FirewallLease {
                    token,
                    digest,
                    fence: self.generation_fence(&network),
                    store: Some(store.clone()),
                });
                self.last_error = None;
                self.phase = UsbipServicePhase::Ready;
                Ok(())
            }
            Err(error) => self.effect_failed(error),
        }
    }

    /// Observe only this claim's own projection.
    ///
    /// # Errors
    ///
    /// Returns [`UsbipControllerError::InvalidState`] when no admitted claim
    /// has been reconciled, [`UsbipControllerError::FirewallDrift`] when the
    /// observation differs from the desired projection, and the port's error
    /// when the observation itself fails.
    pub fn observe_claim<P: UsbipClaimPort>(
        &mut self,
        store: &StoreIncarnation,
        port: &mut P,
    ) -> Result<(), UsbipControllerError> {
        let claim = self.claim.as_ref().ok_or(UsbipControllerError::InvalidState)?;
        let network = self.network.as_ref().ok_or(UsbipControllerError::InvalidState)?;
        let firewall = self.firewall.as_ref().ok_or(UsbipControllerError::InvalidState)?;
        let fence = self.claim_fence(claim, network, store)?;
        match port.observe_claim_firewall(
            claim,
            network.identity().resource_uid(),
            &fence,
            &firewall.token,
        ) {
            Ok(observation) if observation.matches_expected() => {
                self.phase = UsbipServicePhase::Ready;
                self.last_error = None;
                Ok(())
            }
            Ok(_) => {
                self.phase = UsbipServicePhase::Drifted;
                tracing::warn!(
                    device = %self.device_uid.to_canonical_string(),
                    reason = "ownership-scoped firewall observation differs from desired state",
                    "usbip service firewall drifted",
                );
                Err(UsbipControllerError::FirewallDrift)
            }
            Err(error) => self.effect_failed(error),
        }
    }

    /// Remove the projection, stop the relay leg, then hand the relationship
    /// back.
    ///
    /// The order is the contract: the projection is removed while the
    /// reservation is still held, the relay that could still reach the backing
    /// is stopped next, and only then is the source's claim released. A failure
    /// at any step leaves the retained state intact for a retry, so a
    /// partially torn-down Service never releases an authority its own effects
    /// are still using.
    ///
    /// # Errors
    ///
    /// Returns [`UsbipControllerError::InvalidState`] when a claim is held
    /// without a recorded network, and the port's error when a teardown step
    /// does not confirm.
    pub fn finalize_claim<P: UsbipClaimPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), UsbipControllerError> {
        let Some(claim) = self.claim.clone() else {
            return Err(UsbipControllerError::InvalidState);
        };
        self.phase = UsbipServicePhase::Releasing;
        if let Some(firewall) = self.firewall.as_ref() {
            let network = self
                .network
                .as_ref()
                .ok_or(UsbipControllerError::InvalidState)?;
            let store = firewall
                .claim_store()
                .ok_or(UsbipControllerError::InvalidState)?;
            let fence = self.claim_fence(&claim, network, store)?;
            match port.mutate_claim_firewall(
                &claim,
                network.identity().resource_uid(),
                FirewallProjectionAction::Remove,
                &fence,
                Some(&firewall.token),
            ) {
                Ok(confirmation)
                    if matches!(
                        confirmation.kind(),
                        FirewallConfirmationKind::Removed
                            | FirewallConfirmationKind::ValidatedAbsent
                    ) =>
                {
                    self.firewall = None;
                }
                Ok(_) => return self.effect_failed(UsbipEffectError::EffectRejected),
                Err(error) => return self.effect_failed(error),
            }
        }
        if let Some(helper) = self.relay_helper.clone()
            && let Err(error) = port.stop_relay_leg(&claim, &helper)
        {
            return self.effect_failed(error);
        }
        self.relay = None;
        self.relay_helper = None;
        if let Err(error) = port.release_claim(&claim) {
            return self.effect_failed(error);
        }
        self.claim = None;
        self.store = None;
        self.network = None;
        self.network_assignment_epoch = None;
        self.source_released = true;
        self.last_error = None;
        self.phase = UsbipServicePhase::WaitingForNetwork;
        Ok(())
    }

    /// Record one claim refusal and report it as this controller's failure.
    ///
    /// The refusal names the enforcing stage and the reason (R42); no resource
    /// identity, path, or device detail is logged from it.
    fn claim_refusal_error(&mut self, refusal: BindingRefusal) -> UsbipControllerError {
        let error = UsbipEffectError::ClaimRefused(refusal.stage(), refusal.reason());
        self.effect_failed::<()>(error)
            .expect_err("a refusal always fails the controller it is recorded on")
    }

    /// The refusal this controller reports when the retained store moved.
    fn stale_store_error(&mut self) -> UsbipControllerError {
        self.claim_refusal_error(BindingRefusal::new(
            AdmissionStage::Reserve,
            RefusalReason::StaleAuthority,
        ))
    }

    /// The generation fence the pre-graph projection path uses.
    fn generation_fence(&self, network: &NetworkDependency) -> FirewallGenerationFence {
        FirewallGenerationFence::new(network.generation(), self.service_generation)
    }

    /// The claim-scoped fence one projection mutation is measured against.
    fn claim_fence(
        &self,
        claim: &AdmittedDeviceClaim,
        network: &NetworkDependency,
        store: &StoreIncarnation,
    ) -> Result<ClaimProjectionFence, UsbipControllerError> {
        ClaimProjectionFence::new(claim, network.generation(), self.service_generation, store)
            .map_err(|refusal| {
                UsbipControllerError::Effect(UsbipEffectError::ClaimRefused(
                    refusal.stage(),
                    refusal.reason(),
                ))
            })
    }

    /// The legacy per-Network/per-device projection path's own checks.
    fn validate_network(
        &mut self,
        network: &NetworkDependency,
    ) -> Result<(), UsbipControllerError> {
        if self.service.zone_uid() != network.identity().zone_uid() {
            return self.effect_failed(UsbipEffectError::WrongZone);
        }
        if !network.ready() {
            return self.effect_failed(UsbipEffectError::NetworkNotReady);
        }
        if network.assignment_epoch() == Some(0)
            || self
                .network_assignment_epoch
                .zip(network.assignment_epoch())
                .is_some_and(|(current, next)| next < current)
        {
            return self.effect_failed(UsbipEffectError::StaleAssignment);
        }
        Ok(())
    }

    fn effect_failed<T>(&mut self, error: UsbipEffectError) -> Result<T, UsbipControllerError> {
        tracing::warn!(
            device = %self.device_uid.to_canonical_string(),
            service = %self.service.resource_uid().to_canonical_string(),
            phase = ?self.phase,
            error = %error,
            "usbip service effect failed",
        );
        self.last_error = Some(error);
        self.phase = match error {
            UsbipEffectError::Transient
            | UsbipEffectError::FirewallGenerationMismatch
            | UsbipEffectError::StaleAssignment => {
                if self.firewall.is_some() {
                    UsbipServicePhase::Releasing
                } else {
                    UsbipServicePhase::Applying
                }
            }
            UsbipEffectError::WrongZone
            | UsbipEffectError::RelayAuthorityConflict
            | UsbipEffectError::FirewallForeignConflict
            | UsbipEffectError::EffectRejected
            | UsbipEffectError::ClaimRefused(_, _)
            | UsbipEffectError::UnknownProjectionAction => UsbipServicePhase::Blocked,
            UsbipEffectError::NetworkNotReady => UsbipServicePhase::WaitingForNetwork,
        };
        Err(UsbipControllerError::Effect(error))
    }
}

impl core::fmt::Debug for UsbipController {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UsbipController")
            .field("phase", &self.phase)
            .field("has_network", &self.network.is_some())
            .field("has_relay", &self.relay.is_some())
            .field("has_firewall", &self.firewall.is_some())
            .field("last_error", &self.last_error)
            .finish()
    }
}

/// Closed controller failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipControllerError {
    /// The controller state did not admit the requested transition.
    InvalidState,
    /// Ownership-scoped observation differs from desired state.
    FirewallDrift,
    /// An injected semantic effect failed.
    Effect(UsbipEffectError),
}

impl UsbipControllerError {
    /// Return the stable identity-free code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidState => "invalid-state",
            Self::FirewallDrift => "firewall-drift",
            Self::Effect(error) => error.code(),
        }
    }
}

impl core::fmt::Display for UsbipControllerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for UsbipControllerError {}
