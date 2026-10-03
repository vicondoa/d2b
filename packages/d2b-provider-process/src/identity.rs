//! The Process-family typed spec and the row identity every provider ticket
//! is derived from.
//!
//! These types are the provider-facing half of a Process row: the typed
//! contract the driver dispatches on, and the durable identity plus
//! zone-authority inputs (KTD7) the signed ticket path consumes.

use d2b_contracts_resource::v3::process::{
    DesiredLifecycle, EphemeralProcessSpec, ExecutionSpec, ProcessClass, ProcessSpec,
    RestartPolicySpec,
};
use d2b_contracts_resource::v3::{
    AdoptionPolicy, ControllerGeneration, DurationMs, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneId, ZoneRevision,
};
use d2b_process_conformance::{
    GuestExecutionBinding, LaunchIdentity, ProcessPlanRefusal, ProcessSubject,
};

use crate::worker_launch::{DeviceWorkerLaunch, ServingWorkerLaunch};

/// The typed Process-family spec: one row is either the durable `Process`
/// contract or the one-shot `EphemeralProcess` contract. The two share the
/// execution fields, so the driver decodes once and dispatches on the row's
/// type name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessFamilySpec {
    /// The durable `Process` contract row.
    Process(ProcessSpec),
    /// The one-shot `EphemeralProcess` contract row.
    Ephemeral(EphemeralProcessSpec),
}

impl ProcessFamilySpec {
    /// Borrow the shared execution fields.
    pub fn execution(&self) -> &ExecutionSpec {
        match self {
            Self::Process(spec) => spec.execution(),
            Self::Ephemeral(spec) => spec.execution(),
        }
    }

    /// The declared process class (ephemeral rows are workers by contract).
    pub fn process_class(&self) -> ProcessClass {
        self.execution().process_class()
    }

    /// Whether the desired steady state is a live process. A one-shot
    /// `EphemeralProcess` is always Running (the old `DesiredProcess::
    /// is_running` ephemeral arm).
    pub fn wants_running(&self) -> bool {
        match self {
            Self::Process(spec) => spec.desired_lifecycle() == DesiredLifecycle::Running,
            Self::Ephemeral(_) => true,
        }
    }

    /// The adoption policy. A one-shot row has no policy field and always
    /// adopts on restart (the old ephemeral arm called
    /// `adopt_ephemeral_resource` unconditionally).
    pub fn adoption_policy(&self) -> AdoptionPolicy {
        match self {
            Self::Process(spec) => spec.adoption_policy(),
            Self::Ephemeral(_) => AdoptionPolicy::AdoptOnRestart,
        }
    }

    /// The restart policy, when the family member has one. A one-shot row
    /// never restarts (old `restart_delay` returned zero and every ephemeral
    /// restart decision was `false`).
    pub fn restart_policy(&self) -> Option<&RestartPolicySpec> {
        match self {
            Self::Process(spec) => Some(spec.restart_policy()),
            Self::Ephemeral(_) => None,
        }
    }

    /// The bounded graceful-drain timeout, when the family member has one.
    pub fn drain_timeout(&self) -> Option<&DurationMs> {
        match self {
            Self::Process(spec) => Some(spec.drain_timeout()),
            Self::Ephemeral(_) => None,
        }
    }
}

/// The identity inputs every provider ticket is derived from: the durable
/// adoption identity (zone, type, name, uid, generation) plus the
/// zone-authority ticket inputs from the bundle resolver /
/// `ZoneAuthorityIdentity` path (KTD7). Nothing is read from or written to the
/// spec store at runtime.
#[derive(Debug, Clone)]
pub struct ProcessResourceIdentity {
    /// Zone the row lives in; the ticket and the broker fences are
    /// zone-scoped.
    pub zone: ZoneId,
    /// Canonical reference of the row this identity belongs to.
    pub resource_ref: ResourceRef,
    /// Durable uid of the row.
    pub resource_uid: ResourceUid,
    /// Durable generation of the row.
    pub resource_generation: ResourceGeneration,
    /// Spec `processClass` (decoded from the same durable row). Only
    /// controller rows take the committed controller-provider identity, and
    /// the effects cannot re-read the spec at finalize time (KTD7).
    pub process_class: ProcessClass,
    /// Provider reference the row's spec selected.
    pub provider_ref: ResourceRef,
    /// The canonical launch identity (KTD7), resolved once from this row:
    /// semantic owner ref/UID, execution target, cross-target selector, VM
    /// scope, and the legacy runner role. The ticket, the broker fence, and
    /// this driver's adopt/probe path all consume this one value; none of
    /// them re-derives its fields.
    pub launch: LaunchIdentity,
    /// Zone authority uid (bundle resolver / `ZoneAuthorityIdentity` path).
    pub zone_uid: Option<ResourceUid>,
    /// Zone policy revision from the authority path.
    pub policy_revision: Option<u64>,
    /// Committed provider-assignment generation of the row, when the
    /// authority path retains one.
    pub provider_assignment_generation: Option<ResourceGeneration>,
    /// Controller generation the launch ticket binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// Committed uid of the `Provider` that owns the supervised controller
    /// route, when the row carries one.
    pub controller_provider_uid: Option<ResourceUid>,
    /// Committed generation of that controller `Provider`.
    pub controller_provider_generation: Option<ResourceGeneration>,
    /// Binding-declared Guest execution inputs, for a row that executes
    /// inside a Guest.
    pub guest_execution: Option<GuestExecutionBinding>,
    /// Binding-declared serving-worker launch inputs (VolumeBinding-owned
    /// virtiofsd workers only).
    pub worker_launch: Option<ServingWorkerLaunch>,
    /// Device-declared worker launch parameters (the four declared
    /// Device-owned worker rows only; `U17` gap closure).
    pub device_worker_launch: Option<DeviceWorkerLaunch>,
}

impl ProcessResourceIdentity {
    /// The committed consumer identity one launch prepares its relationships
    /// against.
    ///
    /// This is the one place the driver derives a [`ProcessSubject`], and
    /// both Process lifetimes call it: a long-running `Process` and a
    /// run-to-completion `EphemeralProcess` reach the plan through the same
    /// derivation, and the only thing the returned subject distinguishes is
    /// the lifetime the row's own reference declares (AE20, AE28).
    ///
    /// The row reference, uid, and Zone are already committed on the identity
    /// the manager built, so nothing is re-read from the spec and no second
    /// copy of the row's identity exists (R2, R35). The row's committed
    /// generation is the revision preparation is pinned to: it is the counter
    /// every ownership, view, consumer, provider-assignment, or policy change
    /// advances, so a re-committed row cannot reuse an earlier plan (R35).
    ///
    /// # Errors
    ///
    /// Returns [`ProcessPlanRefusal`] when the row reference names no
    /// execution instance, or when the driver holds no Zone revision to pin
    /// the preparation to. Both are refusals rather than defaults: a launch
    /// prepared without a committed revision is a launch whose authority no
    /// later change can invalidate.
    pub fn subject(&self) -> Result<ProcessSubject, ProcessPlanRefusal> {
        ProcessSubject::new(
            self.resource_ref.clone(),
            self.resource_uid.clone(),
            self.zone.clone(),
            ZoneRevision::new(self.resource_generation.get()),
        )
    }
}

/// The authored `metadata.ownerRef` of one row, when it carries one.
///
/// A manager resolves an owner key only for owners that are its own rows, so
/// a child of an owner the plane does not hold - an unconverted type, or a
/// row ingested before its owner existed - falls back to the reference the
/// row was authored with. Nothing else is derived from the metadata.
pub fn decode_metadata_owner_ref(metadata: &[u8]) -> Option<ResourceRef> {
    let value: serde_json::Value = serde_json::from_slice(metadata).ok()?;
    value
        .get("ownerRef")
        .and_then(serde_json::Value::as_str)
        .and_then(|owner| ResourceRef::parse(owner).ok())
}

#[cfg(test)]
mod subject_tests {
    use super::*;
    use d2b_contracts_resource::v3::execution_policy_resource::ExecutionInstanceKind;

    fn identity(reference: &str) -> ProcessResourceIdentity {
        ProcessResourceIdentity {
            zone: ZoneId::parse("work").expect("a canonical Zone"),
            resource_ref: ResourceRef::parse(reference).expect("a canonical reference"),
            resource_uid: ResourceUid::parse("22222222-2222-4222-8222-222222222222")
                .expect("a canonical uid"),
            resource_generation: ResourceGeneration::new(4).expect("a nonzero generation"),
            process_class: ProcessClass::Worker,
            provider_ref: ResourceRef::parse("Provider/system-minijail")
                .expect("a canonical reference"),
            launch: LaunchIdentity::new(
                None,
                None,
                ResourceRef::parse("Host/host-system").expect("a canonical reference"),
                None,
                "worker",
                false,
            )
            .expect("a complete launch identity"),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("a nonzero generation"),
            controller_provider_uid: None,
            controller_provider_generation: None,
            guest_execution: None,
            worker_launch: None,
            device_worker_launch: None,
        }
    }

    /// AE28: both Process lifetimes reach the plan through one subject
    /// derivation, and the only thing the derivation records is the lifetime
    /// the row's own reference declares.
    #[test]
    fn both_lifetimes_reach_the_plan_through_one_subject() {
        let long_running = identity("Process/worker").subject().expect("a Process subject");
        let one_shot = identity("EphemeralProcess/flush").subject().expect("a one-shot subject");

        assert_eq!(long_running.kind(), ExecutionInstanceKind::LongRunning);
        assert_eq!(one_shot.kind(), ExecutionInstanceKind::OneShot);

        // The committed identity is the row's own, read from the manager's
        // identity rather than re-derived from the spec.
        assert_eq!(
            long_running.process_uid(),
            &identity("Process/worker").resource_uid
        );
        assert_eq!(
            long_running.resource_revision(),
            ZoneRevision::new(4)
        );
    }

    /// A row that is not an execution instance never reaches the plan path.
    #[test]
    fn a_row_that_is_not_an_execution_instance_is_refused() {
        assert!(identity("Volume/data").subject().is_err());
    }
}
