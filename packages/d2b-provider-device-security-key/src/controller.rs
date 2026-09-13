//! Security-key Device controller facade.

use core::fmt;
use d2b_contracts_provider::v3::semantic_services::{
    SemanticFamily,
    child_resources::{
        BindingChildKind, BindingChildPlacement, BindingChildRequest, BindingChildSet,
        explicit_binding_children, explicit_binding_children_with_user,
    },
};
use d2b_contracts_resource::v3::{ExecutionDomain, ResourceRef, ResourceUid};

use crate::{
    MAX_SESSION_RING_SIZE, MIN_SESSION_RING_SIZE, PhysicalUsbBackingClaim, SecurityKeyAdmission,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyLease, SecurityKeyLeaseError,
    SecurityKeySessionId, SECURITY_KEY_BINDING_RESOURCE_TYPE, SECURITY_KEY_SERVICE_RESOURCE_TYPE,
};
const SECURITY_KEY_PROVIDER_REF: &str = "Provider/device-security-key";

const SECURITY_KEY_BINDING_CHILD_REQUESTS: [BindingChildRequest; 2] = [
    BindingChildRequest::process(
        BindingChildKind::Process,
        BindingChildPlacement::Guest,
        "guest-frontend",
        "Provider/system-systemd",
        "sk-frontend",
        ExecutionDomain::User,
        "service",
    ),
    BindingChildRequest::endpoint(
        BindingChildPlacement::Guest,
        "guest-endpoint",
        "guest-frontend",
    ),
];

const SECURITY_KEY_BINDING_CHILD_REQUESTS_WITH_USER: [BindingChildRequest; 2] = [
    BindingChildRequest::process_for_user(
        BindingChildKind::Process,
        BindingChildPlacement::Guest,
        "guest-frontend",
        "Provider/system-systemd",
        "sk-frontend",
        "service",
    ),
    BindingChildRequest::endpoint(
        BindingChildPlacement::Guest,
        "guest-endpoint",
        "guest-frontend",
    ),
];

/// Default descriptor repair interval.
pub const SECURITY_KEY_REPAIR_INTERVAL_SECS: u64 = 30;
/// Maximum descriptor repair interval.
pub const SECURITY_KEY_MAX_REPAIR_INTERVAL_SECS: u64 = 60;
/// Authority Service finalizer owned by this Provider.
pub const SECURITY_KEY_SERVICE_FINALIZER: &str =
    "device-security-key.d2bus.org/service-finalizer";
/// Consumer Binding finalizer owned by this Provider.
pub const SECURITY_KEY_BINDING_FINALIZER: &str =
    "device-security-key.d2bus.org/binding-finalizer";

/// Lifecycle phase retained by the resource-backed controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyPhase {
    /// No physical effect has been admitted.
    Pending,
    /// A session or child realization is active.
    Active,
    /// The last session completed and released authority.
    Completed,
    /// A stale or ambiguous fence requires fresh Core admission.
    Quarantined,
}

/// The cutover contract for SecurityKey Service and Binding owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityKeyRunnerContract {
    service_resource_type: &'static str,
    binding_resource_type: &'static str,
    repair_interval_secs: u64,
    watched_configuration_is_dependency: bool,
}

impl SecurityKeyRunnerContract {
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

/// Return the one shared-Runner registration for SecurityKey.
pub const fn security_key_runner_contract() -> SecurityKeyRunnerContract {
    SecurityKeyRunnerContract {
        service_resource_type: SECURITY_KEY_SERVICE_RESOURCE_TYPE,
        binding_resource_type: SECURITY_KEY_BINDING_RESOURCE_TYPE,
        repair_interval_secs: SECURITY_KEY_REPAIR_INTERVAL_SECS,
        watched_configuration_is_dependency: true,
    }
}

/// Controller-level failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyControllerError {
    /// Lease state rejected the requested operation.
    Lease(SecurityKeyLeaseError),
    /// Binding or Service references failed semantic admission.
    Admission,
    /// The configured session-ring capacity is outside the frozen bound.
    RingCapacity,
    /// An effect failed while advancing a terminal session transition.
    Effect(SecurityKeyEffectError),
}

impl fmt::Display for SecurityKeyControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Lease(error) => error.code(),
            Self::Admission => "security-key-controller-admission-failed",
            Self::RingCapacity => "security-key-session-ring-capacity-out-of-range",
            Self::Effect(error) => error.code(),
        })
    }
}

impl std::error::Error for SecurityKeyControllerError {}

/// Combined reconcile outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyReconcileOutcome {
    /// The lease and relay are active.
    Active,
    /// The terminal session completed and its authority was released.
    Completed,
}

/// Device-security-key controller state.
pub struct SecurityKeyController {
    lease: SecurityKeyLease,
    phase: SecurityKeyPhase,
}

impl SecurityKeyController {
    /// Construct a controller after admitting the configured session-ring
    /// capacity.
    pub fn new(
        holder: ResourceUid,
        backing: PhysicalUsbBackingClaim,
        ring_capacity: usize,
    ) -> Result<Self, SecurityKeyControllerError> {
        admit_ring_capacity(ring_capacity)?;
        Ok(Self {
            lease: SecurityKeyLease::new(holder, backing),
            phase: SecurityKeyPhase::Pending,
        })
    }

    /// Construct a controller from one exact Core Device admission.
    pub fn new_authorized(
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
        ring_capacity: usize,
    ) -> Result<Self, SecurityKeyControllerError> {
        admit_ring_capacity(ring_capacity)?;
        Ok(Self {
            lease: SecurityKeyLease::new_authorized(device_uid, admission)
                .map_err(SecurityKeyControllerError::Lease)?,
            phase: SecurityKeyPhase::Pending,
        })
    }

    /// Borrow the underlying lease state.
    pub const fn lease(&self) -> &SecurityKeyLease {
        &self.lease
    }

    /// Return the resource-backed lifecycle phase.
    pub const fn phase(&self) -> SecurityKeyPhase {
        self.phase
    }

    /// Quarantine the controller until Core supplies fresh matching evidence.
    pub fn quarantine(&mut self) {
        self.phase = SecurityKeyPhase::Quarantined;
    }

    /// Build the explicit Host relay and Guest frontend children for one
    /// authored security-key Binding.
    ///
    /// `target_ref` is the Guest execution target extracted from the Binding's
    /// target object. The caller must provide the authored Binding and its
    /// existing Service; a Service alone never creates consumer children.
    pub fn child_resources(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
    ) -> Result<BindingChildSet, SecurityKeyControllerError> {
        if target_ref.resource_type().as_str() != "Guest" {
            tracing::warn!(
                binding = %binding_ref.to_canonical_string(),
                reason = "target reference is not a Guest resource",
                "security-key binding children refused",
            );
            return Err(SecurityKeyControllerError::Admission);
        }
        explicit_binding_children(
            SemanticFamily::SecurityKey,
            binding_ref.clone(),
            service_ref.clone(),
            target_ref.clone(),
            ResourceRef::parse(SECURITY_KEY_PROVIDER_REF)
                .expect("security-key Provider reference is canonical"),
            &SECURITY_KEY_BINDING_CHILD_REQUESTS,
        )
        .map_err(|error| {
            tracing::warn!(
                binding = %binding_ref.to_canonical_string(),
                error = %error,
                "security-key binding child declaration failed",
            );
            SecurityKeyControllerError::Admission
        })
    }

    /// Build security-key children while binding the frontend to the
    /// authored workload User identity.
    pub fn child_resources_for_user(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
        user_ref: &ResourceRef,
    ) -> Result<BindingChildSet, SecurityKeyControllerError> {
        if target_ref.resource_type().as_str() != "Guest"
            || user_ref.resource_type().as_str() != "User"
        {
            tracing::warn!(
                binding = %binding_ref.to_canonical_string(),
                reason = "target is not a Guest or user reference is not a User",
                "security-key binding children refused",
            );
            return Err(SecurityKeyControllerError::Admission);
        }
        explicit_binding_children_with_user(
            SemanticFamily::SecurityKey,
            binding_ref.clone(),
            service_ref.clone(),
            target_ref.clone(),
            ResourceRef::parse(SECURITY_KEY_PROVIDER_REF)
                .expect("security-key Provider reference is canonical"),
            Some(user_ref.clone()),
            &SECURITY_KEY_BINDING_CHILD_REQUESTS_WITH_USER,
        )
        .map_err(|error| {
            tracing::warn!(
                binding = %binding_ref.to_canonical_string(),
                error = %error,
                "security-key binding child declaration failed",
            );
            SecurityKeyControllerError::Admission
        })
    }

    /// Start a session through the authority-before-open sequence.
    pub fn acquire<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        self.lease
            .acquire(session, device_uid, port)
            .map_err(|error| {
                tracing::warn!(
                    device = %self.lease.holder().to_canonical_string(),
                    error = %error,
                    "security-key session acquire failed",
                );
                if matches!(
                    error,
                    SecurityKeyLeaseError::AuthorizationDenied
                        | SecurityKeyLeaseError::Effect(
                            SecurityKeyEffectError::AuthorizationDenied
                        )
                ) {
                    self.phase = SecurityKeyPhase::Quarantined;
                }
                SecurityKeyControllerError::Lease(error)
            })?;
        self.phase = SecurityKeyPhase::Active;
        Ok(SecurityKeyReconcileOutcome::Active)
    }

    /// Acquire a session after exact Device and holder revalidation.
    pub fn acquire_authorized<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        holder: &ResourceRef,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        self.lease
            .acquire_authorized(session, device_uid, holder, port)
            .map_err(|error| {
                tracing::warn!(
                    device = %self.lease.holder().to_canonical_string(),
                    holder = %holder.to_canonical_string(),
                    error = %error,
                    "security-key authorized session acquire failed",
                );
                if matches!(
                    error,
                    SecurityKeyLeaseError::AuthorizationDenied
                        | SecurityKeyLeaseError::Effect(
                            SecurityKeyEffectError::AuthorizationDenied
                        )
                ) {
                    self.phase = SecurityKeyPhase::Quarantined;
                }
                SecurityKeyControllerError::Lease(error)
            })?;
        self.phase = SecurityKeyPhase::Active;
        Ok(SecurityKeyReconcileOutcome::Active)
    }

    /// Rebind the controller to fresh Core admission evidence after a
    /// completed session.
    pub fn rebind_authorized(
        &mut self,
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
    ) -> Result<(), SecurityKeyControllerError> {
        let device = device_uid.to_canonical_string();
        self.lease
            .rebind_authorized(device_uid, admission)
            .map_err(|error| {
                tracing::warn!(
                    device = %device,
                    error = %error,
                    "security-key admission rebind refused",
                );
                SecurityKeyControllerError::Lease(error)
            })
    }

    /// Complete and record the active session.
    pub fn complete<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        if self.lease.session().is_none() {
            tracing::warn!(
                device = %self.lease.holder().to_canonical_string(),
                reason = "complete requested with no active session",
                "security-key session complete refused",
            );
            return Err(SecurityKeyControllerError::Lease(
                SecurityKeyLeaseError::InvalidTransition,
            ));
        }
        self.lease
            .complete(port)
            .map_err(|error| {
                tracing::warn!(
                    device = %self.lease.holder().to_canonical_string(),
                    error = %error,
                    "security-key session complete failed",
                );
                SecurityKeyControllerError::Lease(error)
            })?;
        self.phase = SecurityKeyPhase::Completed;
        Ok(SecurityKeyReconcileOutcome::Completed)
    }
}

impl fmt::Debug for SecurityKeyController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyController")
            .field("lease", &self.lease)
            .field("phase", &self.phase)
            .finish()
    }
}

/// Admit only the frozen bounded session-ring capacity.
fn admit_ring_capacity(ring_capacity: usize) -> Result<(), SecurityKeyControllerError> {
    if (MIN_SESSION_RING_SIZE..=MAX_SESSION_RING_SIZE).contains(&ring_capacity) {
        Ok(())
    } else {
        Err(SecurityKeyControllerError::RingCapacity)
    }
}
