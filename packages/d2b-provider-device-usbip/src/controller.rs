//! USB Service firewall and relay lifecycle controller.

use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildSet;
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceUid};

use crate::binding_child_resources;
use crate::firewall::{
    FirewallConfirmationKind, FirewallDigest, FirewallGenerationFence, FirewallProjectionAction,
    FirewallProjectionIntent, FirewallToken, RelayAuthorityLease, UsbipEffectError,
    UsbipEffectPort,
};

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
    /// Reconciliation was requested after finalization.
    Finalized,
}

impl core::fmt::Display for UsbipBindingControllerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Admission => "usbip-binding-controller-admission-failed",
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
    children: BindingChildSet,
    phase: UsbipBindingPhase,
}

impl core::fmt::Debug for UsbipBindingController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("UsbipBindingController")
            .field("phase", &self.phase)
            .field("children", &self.children)
            .finish()
    }
}

impl UsbipBindingController {
    /// Construct a Binding controller from explicit authored references.
    pub fn new(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
    ) -> Result<Self, UsbipBindingControllerError> {
        let children = binding_child_resources(binding_ref, service_ref, target_ref)
            .map_err(|_| UsbipBindingControllerError::Admission)?;
        Ok(Self {
            children,
            phase: UsbipBindingPhase::Pending,
        })
    }

    /// Return the current Binding lifecycle phase.
    pub const fn phase(&self) -> UsbipBindingPhase {
        self.phase
    }

    /// Borrow the current child intents.
    pub const fn children(&self) -> &BindingChildSet {
        &self.children
    }

    /// Observe Core-managed child readiness without spawning a feature
    /// process.
    pub fn observe_children(
        &mut self,
        ready: bool,
    ) -> Result<UsbipBindingReconcileResult, UsbipBindingControllerError> {
        if self.phase == UsbipBindingPhase::Deleted {
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

    /// Mark the Binding deleted after Endpoint, then Process children drain.
    pub fn finalize(&mut self) {
        self.phase = UsbipBindingPhase::Deleted;
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
}

impl core::fmt::Debug for FirewallLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallLease(<redacted>)")
    }
}

/// USB Service controller state for one physical backing and Network relay.
pub struct UsbipController {
    service: ScopedResourceUid,
    service_generation: ResourceGeneration,
    device_uid: ResourceUid,
    network: Option<NetworkDependency>,
    phase: UsbipServicePhase,
    relay: Option<RelayAuthorityLease>,
    firewall: Option<FirewallLease>,
    last_error: Option<UsbipEffectError>,
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
            .ok_or(UsbipControllerError::InvalidState)?;
        let firewall = self
            .firewall
            .as_mut()
            .ok_or(UsbipControllerError::InvalidState)?;
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
        self.last_error = None;
        self.phase = UsbipServicePhase::WaitingForNetwork;
        Ok(())
    }

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
        Ok(())
    }

    fn effect_failed<T>(&mut self, error: UsbipEffectError) -> Result<T, UsbipControllerError> {
        self.last_error = Some(error);
        self.phase = match error {
            UsbipEffectError::Transient | UsbipEffectError::FirewallGenerationMismatch => {
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
