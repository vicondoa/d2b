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

use crate::effect_port::{
    DeviceId, InventoryEffectError, InventoryObservation, ObservationPolicyId,
    SecurityKeyInventoryEffectPort,
};
use crate::{
    PhysicalUsbBackingClaim, SecurityKeyAdmission, SecurityKeyEffectError, SecurityKeyEffectPort,
    SecurityKeyLease, SecurityKeyLeaseError, SecurityKeySessionId, SessionRecord, SessionResult,
    SessionRing,
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

/// Controller-level failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyControllerError {
    /// Lease state rejected the requested operation.
    Lease(SecurityKeyLeaseError),
    /// Binding or Service references failed semantic admission.
    Admission,
    /// Session ring could not be created.
    RingCapacity,
    /// An effect failed while recording a terminal session.
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
    /// The terminal session was recorded and authority released.
    Completed,
}

/// Reconcile output including the child resources owned by a Binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityKeyReconcileResultWithChildren {
    /// Lease/session outcome.
    pub outcome: SecurityKeyReconcileOutcome,
    /// UID-free Process and Endpoint intents.
    pub children: BindingChildSet,
}

/// Device-security-key controller state.
pub struct SecurityKeyController {
    lease: SecurityKeyLease,
    ring: SessionRing,
}

impl SecurityKeyController {
    /// Construct a controller with a bounded session ring.
    pub fn new(
        holder: ResourceUid,
        backing: PhysicalUsbBackingClaim,
        ring_capacity: usize,
    ) -> Result<Self, SecurityKeyControllerError> {
        Ok(Self {
            lease: SecurityKeyLease::new(holder, backing),
            ring: SessionRing::new(ring_capacity)
                .map_err(|_| SecurityKeyControllerError::RingCapacity)?,
        })
    }

    /// Construct a controller from one exact Core Device admission.
    pub fn new_authorized(
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
        ring_capacity: usize,
    ) -> Result<Self, SecurityKeyControllerError> {
        Ok(Self {
            lease: SecurityKeyLease::new_authorized(device_uid, admission)
                .map_err(SecurityKeyControllerError::Lease)?,
            ring: SessionRing::new(ring_capacity)
                .map_err(|_| SecurityKeyControllerError::RingCapacity)?,
        })
    }

    /// Borrow the underlying lease state.
    pub const fn lease(&self) -> &SecurityKeyLease {
        &self.lease
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
        .map_err(|_| SecurityKeyControllerError::Admission)
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
        .map_err(|_| SecurityKeyControllerError::Admission)
    }

    /// Return the session outcome together with the explicit Binding children.
    pub fn reconcile_with_children(
        &mut self,
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
        outcome: SecurityKeyReconcileOutcome,
    ) -> Result<SecurityKeyReconcileResultWithChildren, SecurityKeyControllerError> {
        let children = Self::child_resources(binding_ref, service_ref, target_ref)?;
        Ok(SecurityKeyReconcileResultWithChildren { outcome, children })
    }

    /// Return the reconcile output with an explicit workload User identity.
    pub fn reconcile_with_children_for_user(
        &mut self,
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
        user_ref: &ResourceRef,
        outcome: SecurityKeyReconcileOutcome,
    ) -> Result<SecurityKeyReconcileResultWithChildren, SecurityKeyControllerError> {
        let children =
            Self::child_resources_for_user(binding_ref, service_ref, target_ref, user_ref)?;
        Ok(SecurityKeyReconcileResultWithChildren { outcome, children })
    }

    /// Observe the exact physical Device through Core's injected port.
    pub async fn observe_inventory<P: SecurityKeyInventoryEffectPort>(
        &self,
        device_id: &DeviceId,
        policy_id: &ObservationPolicyId,
        port: &P,
    ) -> Result<InventoryObservation, InventoryEffectError> {
        port.observe_inventory(device_id, policy_id).await
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
            .map_err(SecurityKeyControllerError::Lease)?;
        self.ring
            .push(SessionRecord::new(session, SessionResult::InProgress));
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
            .map_err(SecurityKeyControllerError::Lease)?;
        self.ring
            .push(SessionRecord::new(session, SessionResult::InProgress));
        Ok(SecurityKeyReconcileOutcome::Active)
    }

    /// Rebind the controller to fresh Core admission evidence after a
    /// completed session.
    pub fn rebind_authorized(
        &mut self,
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
    ) -> Result<(), SecurityKeyControllerError> {
        self.lease
            .rebind_authorized(device_uid, admission)
            .map_err(SecurityKeyControllerError::Lease)
    }

    /// Complete and record the active session.
    pub fn complete<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<SecurityKeyReconcileOutcome, SecurityKeyControllerError> {
        let session = self
            .lease
            .session()
            .copied()
            .ok_or(SecurityKeyControllerError::Lease(
                SecurityKeyLeaseError::InvalidTransition,
            ))?;
        self.lease
            .complete(port)
            .map_err(SecurityKeyControllerError::Lease)?;
        self.ring
            .push(SessionRecord::new(session, SessionResult::Success));
        Ok(SecurityKeyReconcileOutcome::Completed)
    }
}

impl fmt::Debug for SecurityKeyController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyController")
            .field("lease", &self.lease)
            .field("ring", &self.ring)
            .finish()
    }
}
