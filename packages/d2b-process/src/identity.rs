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
    ZoneId,
};
use d2b_process_conformance::{GuestExecutionBinding, LaunchIdentity};

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
