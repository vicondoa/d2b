//! The Process-controller-authenticated LaunchTicket.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{
    ActivationRunnerInput, ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneRevision,
};
use sha2::{Digest, Sha256};

use crate::error::ProcessConformanceError;
use crate::identity::{ConfigurationDigest, IdentityBinding, ProcessIdentityDigest};
use crate::sandbox::SandboxPlan;

/// Maximum launch deadline, matching the frozen resource-API request
/// lifetime ceiling of 900000 ms.
pub const MAX_LAUNCH_DEADLINE_MS: u32 = 900_000;
/// Maximum inherited descriptors represented by one private launch table.
pub const MAX_INHERITED_FDS: u16 = 256;

/// Compute the Core-owned commitment for a Process execution placement.
///
/// The bundle content identity is supplied only by the verified Core bundle
/// resolver. The resulting digest binds that identity to the exact execution
/// reference, optional cross-target selector, domain, user, template, and
/// selected Provider before the ticket crosses into an effect adapter.
pub fn execution_commitment(
    bundle_content_identity: &str,
    execution_ref: &ResourceRef,
    target_ref: Option<&ResourceRef>,
    domain: ExecutionDomain,
    user_ref: Option<&ResourceRef>,
    template: &BoundedToken,
    selected_provider: &BoundedToken,
) -> ConfigurationDigest {
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-execution-commitment-v1");
    let execution_ref = execution_ref.to_canonical_string();
    let target_ref = target_ref.map(ResourceRef::to_canonical_string);
    let user_ref = user_ref.map(ResourceRef::to_canonical_string);
    for value in [
        bundle_content_identity,
        execution_ref.as_str(),
        target_ref.as_deref().unwrap_or(""),
        match domain {
            ExecutionDomain::System => "system",
            ExecutionDomain::User => "user",
        },
        user_ref.as_deref().unwrap_or(""),
        template.as_str(),
        selected_provider.as_str(),
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    ConfigurationDigest::from_bytes(digest.finalize().into())
}

/// Compute the private host-runtime scope for one Process incarnation.
///
/// The scope is deliberately separate from the Zone-local [`ResourceRef`].
/// It binds the immutable Zone and optional Guest identities to the exact
/// Process incarnation and trusted role so host cgroups and runtime objects
/// cannot collide when names are reused across Zones or generations.
pub fn runtime_scope_commitment(
    zone_uid: &ResourceUid,
    guest_uid: Option<&ResourceUid>,
    process_ref: &ResourceRef,
    process_uid: &ResourceUid,
    role: &str,
    generation: u64,
) -> ConfigurationDigest {
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-runtime-scope-v1");
    let process_ref = process_ref.to_canonical_string();
    for value in [
        zone_uid.as_str(),
        guest_uid.map(ResourceUid::as_str).unwrap_or(""),
        process_ref.as_str(),
        process_uid.as_str(),
        role,
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    digest.update(generation.to_le_bytes());
    ConfigurationDigest::from_bytes(digest.finalize().into())
}

/// The compiled configuration digests bound into one ticket.
///
/// Every member is a digest of a plan the Provider never sees. The
/// `fd_table` member digests the exact inherited FD table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledDigests {
    /// Digest of the compiled sandbox plan.
    pub sandbox: ConfigurationDigest,
    /// Digest of the compiled budget.
    pub budget: ConfigurationDigest,
    /// Digest of the compiled mount set.
    pub mounts: ConfigurationDigest,
    /// Digest of the compiled device access set.
    pub devices: ConfigurationDigest,
    /// Digest of the compiled network usage.
    pub network: ConfigurationDigest,
    /// Digest of the compiled endpoint set.
    pub endpoints: ConfigurationDigest,
    /// Digest of the exact inherited FD table.
    pub fd_table: ConfigurationDigest,
}

impl CompiledDigests {
    /// Validate that every compiled input is bound to a real plan.
    ///
    /// A zero digest is not an identity. Rejecting it at ticket creation
    /// keeps a missing compiler output from becoming an apparently valid
    /// launch that can later be adopted.
    pub fn validate(&self) -> Result<(), ProcessConformanceError> {
        if [
            self.sandbox,
            self.budget,
            self.mounts,
            self.devices,
            self.network,
            self.endpoints,
            self.fd_table,
        ]
        .into_iter()
        .any(ConfigurationDigest::is_zero)
        {
            Err(ProcessConformanceError::InvalidTicket)
        } else {
            Ok(())
        }
    }
}

/// Operation, deadline, and cancellation binding of one launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBinding {
    operation_uid: ResourceUid,
    deadline_ms: u32,
    cancellation: CancellationBinding,
}

impl OperationBinding {
    /// Bind a launch to its operation and deadline.
    pub fn new(
        operation_uid: ResourceUid,
        deadline_ms: u32,
    ) -> Result<Self, ProcessConformanceError> {
        if deadline_ms == 0 || deadline_ms > MAX_LAUNCH_DEADLINE_MS {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self {
            operation_uid,
            deadline_ms,
            cancellation: CancellationBinding::Active,
        })
    }

    /// Bind a launch that has already been cancelled.
    pub fn cancelled(
        operation_uid: ResourceUid,
        deadline_ms: u32,
    ) -> Result<Self, ProcessConformanceError> {
        let mut binding = Self::new(operation_uid, deadline_ms)?;
        binding.cancellation = CancellationBinding::Cancelled;
        Ok(binding)
    }

    /// Borrow the operation identity.
    pub const fn operation_uid(&self) -> &ResourceUid {
        &self.operation_uid
    }

    /// Return the launch deadline in milliseconds.
    pub const fn deadline_ms(&self) -> u32 {
        self.deadline_ms
    }

    /// Return the cancellation state captured at admission.
    pub const fn cancellation(&self) -> CancellationBinding {
        self.cancellation
    }
}

/// Cancellation state captured by a launch ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CancellationBinding {
    /// The operation may proceed.
    Active,
    /// The operation was cancelled before the effect boundary.
    Cancelled,
}

/// The expected readiness observation for a launched process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReadinessExpectation {
    /// No provider-defined readiness check is required.
    None,
    /// The provider must report readiness within this bounded interval.
    Condition {
        /// Maximum wait for the provider readiness observation.
        timeout_ms: u32,
    },
}

impl ReadinessExpectation {
    /// Construct a bounded condition expectation.
    pub fn condition(timeout_ms: u32) -> Result<Self, ProcessConformanceError> {
        if timeout_ms == 0 || timeout_ms > MAX_LAUNCH_DEADLINE_MS {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self::Condition { timeout_ms })
    }

    /// Validate an expectation constructed from a decoded or otherwise
    /// untrusted value.
    pub const fn validate(self) -> Result<(), ProcessConformanceError> {
        match self {
            Self::None => Ok(()),
            Self::Condition { timeout_ms }
                if timeout_ms > 0 && timeout_ms <= MAX_LAUNCH_DEADLINE_MS =>
            {
                Ok(())
            }
            Self::Condition { .. } => Err(ProcessConformanceError::InvalidTicket),
        }
    }
}

/// The private inherited-fd table binding carried by a launch ticket.
///
/// Only the count and a digest cross the Provider seam.  Individual fd
/// numbers and their object identities remain in the core effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InheritedFdTable {
    digest: ConfigurationDigest,
    count: u16,
}

/// The target-side proof carried by a static controller launch.
///
/// The target session and readiness commitments are opaque evidence produced
/// by Core. They do not grant a ResourceClient and cannot be combined with an
/// assignment binding on the same ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControllerLaunchBinding {
    provider_generation: ResourceGeneration,
    target_session_generation: ReconnectGeneration,
    signed_descriptor_digest: ConfigurationDigest,
    target_readiness_digest: ConfigurationDigest,
}

/// The Core-issued assignment proof carried by an assigned controller or
/// Process child.
///
/// The ResourceClient itself remains in Core. Only its opaque binding digest
/// crosses the Process Provider seam, together with the exact Provider,
/// session, and assignment generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControllerAssignmentBinding {
    provider_generation: ResourceGeneration,
    session_generation: ReconnectGeneration,
    assignment_epoch: u64,
    resource_client_binding: ConfigurationDigest,
}

/// Authenticated target binding required for Guest Process execution.
///
/// The Guest daemon derives this commitment from its enrolled Guest identity
/// and current ComponentSession. It carries no raw boot id or transport
/// handle, but fences every launch against Guest replacement, reconnect,
/// controller, and assignment generations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestExecutionBinding {
    target_uid: ResourceUid,
    boot_identity_digest: ConfigurationDigest,
    session_generation: ReconnectGeneration,
    assignment_epoch: u64,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
}

impl GuestExecutionBinding {
    /// Construct an exact Guest target binding.
    pub fn new(
        target_uid: ResourceUid,
        boot_identity_digest: ConfigurationDigest,
        session_generation: ReconnectGeneration,
        assignment_epoch: u64,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
    ) -> Result<Self, ProcessConformanceError> {
        if boot_identity_digest.is_zero() || assignment_epoch == 0 {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self {
            target_uid,
            boot_identity_digest,
            session_generation,
            assignment_epoch,
            provider_generation,
            controller_generation,
        })
    }

    /// Borrow the exact Guest Resource UID.
    pub const fn target_uid(&self) -> &ResourceUid {
        &self.target_uid
    }

    /// Return the kernel boot-identity commitment.
    pub const fn boot_identity_digest(&self) -> ConfigurationDigest {
        self.boot_identity_digest
    }

    /// Return the authenticated parent ComponentSession generation.
    pub const fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// Return the Core assignment epoch.
    pub const fn assignment_epoch(&self) -> u64 {
        self.assignment_epoch
    }

    /// Return the installed Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }
}

impl InheritedFdTable {
    /// Build a bounded private table binding.
    pub fn new(digest: ConfigurationDigest, count: u16) -> Result<Self, ProcessConformanceError> {
        if digest.is_zero() || count > MAX_INHERITED_FDS {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self { digest, count })
    }

    /// Borrow the table digest.
    pub const fn digest(&self) -> ConfigurationDigest {
        self.digest
    }

    /// Return the number of inherited descriptors.
    pub const fn count(&self) -> u16 {
        self.count
    }
}

/// The ticket a Process controller hands to the fixed process effect
/// adapter.
///
/// It binds the resource and committed revision, the owning Provider
/// component and template, the placement, the selected Process Provider, the
/// compiled configuration digests, the operation, and the identity bindings
/// the launch is expected to establish. Static controller launches carry
/// target-session, signed-descriptor, and target-readiness commitments;
/// assignment-bound launches carry only an opaque ResourceClient commitment
/// plus the exact Provider/session/epoch fence. Nothing in it names an
/// executable, a host path, a numeric UID or GID, a cgroup path, a broker
/// operation, or an environment value.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchTicket {
    process_ref: ResourceRef,
    process_uid: ResourceUid,
    zone_uid: Option<ResourceUid>,
    owner_ref: Option<ResourceRef>,
    owner_uid: Option<ResourceUid>,
    runtime_scope: Option<ConfigurationDigest>,
    resource_revision: Option<ZoneRevision>,
    resource_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    owner_provider: BoundedToken,
    component: BoundedToken,
    template: BoundedToken,
    execution_ref: ResourceRef,
    target_ref: Option<ResourceRef>,
    domain: ExecutionDomain,
    user_ref: Option<ResourceRef>,
    selected_provider: BoundedToken,
    provider_ref: ResourceRef,
    digests: CompiledDigests,
    operation: OperationBinding,
    expected_identity: BTreeSet<IdentityBinding>,
    expected_identity_digest: Option<ProcessIdentityDigest>,
    readiness: ReadinessExpectation,
    activation_input: Option<ActivationRunnerInput>,
    inherited_fd_table: InheritedFdTable,
    sandbox: Option<SandboxPlan>,
    guest_execution: Option<GuestExecutionBinding>,
    controller_launch: Option<ControllerLaunchBinding>,
    assignment: Option<ControllerAssignmentBinding>,
    execution_commitment: Option<ConfigurationDigest>,
}

impl LaunchTicket {
    /// Construct a ticket after checking every conformance bound.
    ///
    /// `process_ref` must name `Process` or `EphemeralProcess`,
    /// `execution_ref` must name `Host` or `Guest`, `user_ref` must name
    /// `User` and is mandatory for the user domain, and the expected
    /// identity set must not be empty because every Process Provider has a
    /// locally verified identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_ref: ResourceRef,
        process_uid: ResourceUid,
        resource_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        owner_provider: BoundedToken,
        component: BoundedToken,
        template: BoundedToken,
        execution_ref: ResourceRef,
        domain: ExecutionDomain,
        user_ref: Option<ResourceRef>,
        selected_provider: BoundedToken,
        digests: CompiledDigests,
        operation: OperationBinding,
        expected_identity: BTreeSet<IdentityBinding>,
    ) -> Result<Self, ProcessConformanceError> {
        if !matches!(
            process_ref.resource_type().as_str(),
            "Process" | "EphemeralProcess"
        ) {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if let Some(user_ref) = &user_ref
            && user_ref.resource_type().as_str() != "User"
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if domain == ExecutionDomain::User && user_ref.is_none() {
            return Err(ProcessConformanceError::UserRefRequired);
        }
        if domain == ExecutionDomain::System && user_ref.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if expected_identity.is_empty() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        digests.validate()?;
        let provider_ref = ResourceRef::parse(&format!("Provider/{}", selected_provider.as_str()))
            .map_err(|_| ProcessConformanceError::InvalidTicket)?;
        let inherited_fd_table = InheritedFdTable::new(digests.fd_table, 0)?;
        Ok(Self {
            process_ref,
            process_uid,
            zone_uid: None,
            owner_ref: None,
            owner_uid: None,
            runtime_scope: None,
            resource_revision: None,
            resource_generation,
            controller_generation,
            owner_provider,
            component,
            template,
            execution_ref,
            target_ref: None,
            domain,
            user_ref,
            selected_provider,
            provider_ref,
            digests,
            operation,
            expected_identity,
            expected_identity_digest: None,
            readiness: ReadinessExpectation::None,
            activation_input: None,
            inherited_fd_table,
            sandbox: None,
            guest_execution: None,
            controller_launch: None,
            assignment: None,
            execution_commitment: None,
        })
    }

    /// Attach the Core-produced execution placement commitment.
    pub fn with_execution_commitment(
        mut self,
        commitment: ConfigurationDigest,
    ) -> Result<Self, ProcessConformanceError> {
        if commitment.is_zero() || self.execution_commitment.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.execution_commitment = Some(commitment);
        Ok(self)
    }

    /// Bind the private Zone and Process runtime identity.
    ///
    /// The scope is an effect-owner commitment, not a public ResourceRef.
    /// It must be supplied together with the immutable Zone UID so the broker
    /// can independently derive and verify host-global runtime placement.
    pub fn with_runtime_identity(
        mut self,
        zone_uid: ResourceUid,
        owner_ref: Option<ResourceRef>,
        runtime_scope: ConfigurationDigest,
    ) -> Result<Self, ProcessConformanceError> {
        if runtime_scope.is_zero()
            || self.zone_uid.is_some()
            || self.runtime_scope.is_some()
            || self
                .owner_ref
                .as_ref()
                .is_some_and(|current| owner_ref.as_ref() != Some(current))
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.zone_uid = Some(zone_uid);
        self.owner_ref = owner_ref;
        self.runtime_scope = Some(runtime_scope);
        Ok(self)
    }

    /// Bind the immutable UID of the semantic owner.
    pub fn with_owner_uid(
        mut self,
        owner_uid: ResourceUid,
    ) -> Result<Self, ProcessConformanceError> {
        if self.owner_ref.is_none() || self.owner_uid.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.owner_uid = Some(owner_uid);
        Ok(self)
    }

    /// Bind the exact semantic owner without adding a host-runtime scope.
    ///
    /// Static controller launches use this when their target deployment
    /// supplies the owner proof but the target Zone UID is held by the
    /// controller deployment layer.
    pub fn with_owner_ref(
        mut self,
        owner_ref: ResourceRef,
    ) -> Result<Self, ProcessConformanceError> {
        if self.owner_ref.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.owner_ref = Some(owner_ref);
        Ok(self)
    }

    /// Bind the committed resource revision to this launch.
    pub fn with_resource_revision(
        mut self,
        resource_revision: ZoneRevision,
    ) -> Result<Self, ProcessConformanceError> {
        if resource_revision.get() == 0 || self.resource_revision.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.resource_revision = Some(resource_revision);
        Ok(self)
    }

    /// Bind a target selector for a cross-target Process launch.
    ///
    /// Host-scoped execution refs such as `Host/host-system` may serve
    /// multiple VMs. The selector keeps the signed bundle lookup specific to
    /// the Guest resource without changing the execution authority.
    pub fn with_target_ref(
        mut self,
        target_ref: ResourceRef,
    ) -> Result<Self, ProcessConformanceError> {
        if target_ref.resource_type().as_str() != "Guest" || self.target_ref.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.target_ref = Some(target_ref);
        Ok(self)
    }

    /// Attach the Core-produced proof for a target-local controller launch.
    ///
    /// A fresh controller receives only its Provider generation, target
    /// session, signed component descriptor, and target readiness
    /// commitments. It does not receive a ResourceClient or assignment
    /// authority until a separate controller session is authenticated and
    /// admitted.
    pub fn with_controller_launch_binding(
        mut self,
        provider_generation: ResourceGeneration,
        target_session_generation: ReconnectGeneration,
        signed_descriptor_digest: ConfigurationDigest,
        target_readiness_digest: ConfigurationDigest,
    ) -> Result<Self, ProcessConformanceError> {
        if signed_descriptor_digest.is_zero()
            || target_readiness_digest.is_zero()
            || self.assignment.is_some()
            || self.controller_launch.is_some()
            || self.process_ref.resource_type().as_str() != "Process"
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.controller_launch = Some(ControllerLaunchBinding {
            provider_generation,
            target_session_generation,
            signed_descriptor_digest,
            target_readiness_digest,
        });
        Ok(self)
    }

    /// Attach the separate Core-issued assignment and ResourceClient proof.
    ///
    /// This operation is intentionally incompatible with a static controller
    /// launch binding. A controller must authenticate and become Ready before
    /// it can receive this assignment-scoped authority.
    pub fn with_assignment_binding(
        mut self,
        provider_generation: ResourceGeneration,
        session_generation: ReconnectGeneration,
        assignment_epoch: u64,
        resource_client_binding: ConfigurationDigest,
    ) -> Result<Self, ProcessConformanceError> {
        if assignment_epoch == 0
            || resource_client_binding.is_zero()
            || self.controller_launch.is_some()
            || self.assignment.is_some()
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.assignment = Some(ControllerAssignmentBinding {
            provider_generation,
            session_generation,
            assignment_epoch,
            resource_client_binding,
        });
        Ok(self)
    }

    /// Validate this ticket before handing it to an effect adapter.
    pub fn validate(&self) -> Result<(), ProcessConformanceError> {
        if !matches!(
            self.process_ref.resource_type().as_str(),
            "Process" | "EphemeralProcess"
        ) || !matches!(
            self.execution_ref.resource_type().as_str(),
            "Host" | "Guest"
        ) || (self.domain == ExecutionDomain::User && self.user_ref.is_none())
            || (self.domain == ExecutionDomain::System && self.user_ref.is_some())
            || self
                .target_ref
                .as_ref()
                .is_some_and(|target| target.resource_type().as_str() != "Guest")
            || self.expected_identity.is_empty()
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if self.activation_input.is_some()
            && (self.process_ref.resource_type().as_str() != "EphemeralProcess"
                || self.template.as_str() != "activation-nixos-runner"
                || !matches!(
                    self.execution_ref.resource_type().as_str(),
                    "Host" | "Guest"
                )
                || self
                    .activation_input
                    .as_ref()
                    .is_some_and(|input| input.target_generation == 0))
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if self
            .resource_revision
            .is_some_and(|revision| revision.get() == 0)
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if self.zone_uid.is_some() != self.runtime_scope.is_some()
            || self.runtime_scope.is_some_and(ConfigurationDigest::is_zero)
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if self.execution_ref.resource_type().as_str() == "Guest"
            && self.controller_launch.is_none()
            && self.guest_execution.is_none()
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        if self.execution_ref.resource_type().as_str() == "Host" && self.guest_execution.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.digests.validate()?;
        self.readiness.validate()?;
        if let Some(binding) = &self.guest_execution {
            if binding.controller_generation != self.controller_generation {
                return Err(ProcessConformanceError::InvalidTicket);
            }
            if let Some(assignment) = &self.assignment
                && (assignment.provider_generation != binding.provider_generation
                    || assignment.session_generation != binding.session_generation
                    || assignment.assignment_epoch != binding.assignment_epoch)
            {
                return Err(ProcessConformanceError::InvalidTicket);
            }
        }
        if let Some(binding) = self.controller_launch {
            if binding.provider_generation.get() == 0
                || binding.target_session_generation.get() == 0
                || binding.signed_descriptor_digest.is_zero()
                || binding.target_readiness_digest.is_zero()
                || self.process_ref.resource_type().as_str() != "Process"
            {
                return Err(ProcessConformanceError::InvalidTicket);
            }
        }
        if let Some(binding) = self.assignment {
            if binding.provider_generation.get() == 0
                || binding.session_generation.get() == 0
                || binding.assignment_epoch == 0
                || binding.resource_client_binding.is_zero()
            {
                return Err(ProcessConformanceError::InvalidTicket);
            }
        }
        Ok(())
    }

    /// Validate that this ticket is a static target-local controller launch.
    pub fn validate_controller_launch(&self) -> Result<(), ProcessConformanceError> {
        self.validate()?;
        if self.controller_launch.is_some()
            && self.assignment.is_none()
            && self.resource_revision.is_some()
        {
            Ok(())
        } else {
            Err(ProcessConformanceError::InvalidTicket)
        }
    }

    /// Validate that this ticket carries an assignment-scoped capability.
    pub fn validate_assignment(&self) -> Result<(), ProcessConformanceError> {
        self.validate()?;
        if self.assignment.is_some()
            && self.controller_launch.is_none()
            && self.resource_revision.is_some()
        {
            Ok(())
        } else {
            Err(ProcessConformanceError::InvalidTicket)
        }
    }

    /// Attach a bounded readiness expectation.
    #[must_use]
    pub fn with_readiness(mut self, readiness: ReadinessExpectation) -> Self {
        self.readiness = readiness;
        self
    }

    /// Attach a readiness expectation while rejecting malformed decoded data.
    pub fn try_with_readiness(
        mut self,
        readiness: ReadinessExpectation,
    ) -> Result<Self, ProcessConformanceError> {
        readiness.validate()?;
        self.readiness = readiness;
        Ok(self)
    }

    /// Bind the complete typed sandbox plan to the launch.
    #[must_use]
    pub fn with_sandbox_plan(mut self, sandbox: SandboxPlan) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    /// Bind the closed activation-runner stdin input.
    pub fn with_activation_input(
        mut self,
        input: ActivationRunnerInput,
    ) -> Result<Self, ProcessConformanceError> {
        if self.process_ref.resource_type().as_str() != "EphemeralProcess"
            || self.template.as_str() != "activation-nixos-runner"
            || !matches!(
                self.execution_ref.resource_type().as_str(),
                "Host" | "Guest"
            )
            || input.target_generation == 0
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.activation_input = Some(input);
        Ok(self)
    }

    /// Bind a generic Guest Process to its authenticated target/session.
    pub fn with_guest_execution_binding(
        mut self,
        binding: GuestExecutionBinding,
    ) -> Result<Self, ProcessConformanceError> {
        if self.execution_ref.resource_type().as_str() != "Guest" || self.guest_execution.is_some()
        {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.guest_execution = Some(binding);
        Ok(self)
    }

    /// Borrow the authenticated Guest execution binding, when present.
    pub const fn guest_execution_binding(&self) -> Option<&GuestExecutionBinding> {
        self.guest_execution.as_ref()
    }

    /// Borrow the closed activation-runner stdin input, when present.
    pub const fn activation_input(&self) -> Option<&ActivationRunnerInput> {
        self.activation_input.as_ref()
    }

    /// Borrow the typed sandbox plan, when this launch is a generic Process.
    pub const fn sandbox_plan(&self) -> Option<&SandboxPlan> {
        self.sandbox.as_ref()
    }

    /// Bind the ticket to an already recorded process identity digest.
    ///
    /// The digest is optional for a first launch. Adoption and terminal
    /// relay callers should set it once the effect adapter has established
    /// the process identity, so a result for another process cannot be
    /// accepted under the same resource operation.
    pub fn with_expected_identity_digest(
        mut self,
        identity: ProcessIdentityDigest,
    ) -> Result<Self, ProcessConformanceError> {
        if identity.is_zero() || self.expected_identity_digest.is_some() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        self.expected_identity_digest = Some(identity);
        Ok(self)
    }

    /// Replace the inherited descriptor-table count after core has compiled
    /// the table.
    pub fn with_inherited_fd_count(mut self, count: u16) -> Result<Self, ProcessConformanceError> {
        self.inherited_fd_table = InheritedFdTable::new(self.digests.fd_table, count)?;
        Ok(self)
    }

    /// Borrow the Process or EphemeralProcess reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the resource UID.
    pub const fn process_uid(&self) -> &ResourceUid {
        &self.process_uid
    }

    /// Borrow the immutable Zone UID bound to this launch.
    pub const fn zone_uid(&self) -> Option<&ResourceUid> {
        self.zone_uid.as_ref()
    }

    /// Borrow the exact semantic owner, when one was committed.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// Borrow the immutable semantic-owner UID, when bound.
    pub const fn owner_uid(&self) -> Option<&ResourceUid> {
        self.owner_uid.as_ref()
    }

    /// Borrow the private host-runtime scope commitment.
    pub const fn runtime_scope(&self) -> Option<ConfigurationDigest> {
        self.runtime_scope
    }

    /// Return the committed resource revision, when one was bound.
    pub const fn resource_revision(&self) -> Option<ZoneRevision> {
        self.resource_revision
    }

    /// Return the resource generation this launch is for.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Return the controller generation that issued the ticket.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Borrow the owning semantic Provider name.
    pub const fn owner_provider(&self) -> &BoundedToken {
        &self.owner_provider
    }

    /// Borrow the owning Provider component name.
    pub const fn component(&self) -> &BoundedToken {
        &self.component
    }

    /// Borrow the plain component template ID.
    pub const fn template(&self) -> &BoundedToken {
        &self.template
    }

    /// Borrow the Host or Guest this process runs on.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the optional cross-target selector.
    pub const fn target_ref(&self) -> Option<&ResourceRef> {
        self.target_ref.as_ref()
    }

    /// Borrow the Core-produced execution placement commitment.
    pub const fn execution_commitment(&self) -> Option<ConfigurationDigest> {
        self.execution_commitment
    }

    /// Return the resolved execution domain.
    pub const fn domain(&self) -> ExecutionDomain {
        self.domain
    }

    /// Borrow the exact user identity for a user-domain launch.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }

    /// Borrow the selected Process Provider name.
    pub const fn selected_provider(&self) -> &BoundedToken {
        &self.selected_provider
    }

    /// Borrow the selected Provider ResourceRef.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the compiled configuration digests.
    pub const fn digests(&self) -> &CompiledDigests {
        &self.digests
    }

    /// Borrow the operation and deadline binding.
    pub const fn operation(&self) -> &OperationBinding {
        &self.operation
    }

    /// Borrow the identity bindings this launch is expected to establish.
    pub const fn expected_identity(&self) -> &BTreeSet<IdentityBinding> {
        &self.expected_identity
    }

    /// Borrow the optional expected process identity digest.
    pub const fn expected_identity_digest(&self) -> Option<&ProcessIdentityDigest> {
        self.expected_identity_digest.as_ref()
    }

    /// Validate a returned process identity against this ticket's optional
    /// adoption/terminal identity seal.
    pub fn validate_process_identity(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<(), ProcessConformanceError> {
        if self
            .expected_identity_digest
            .is_some_and(|expected| &expected != identity)
        {
            Err(ProcessConformanceError::TerminalEvidenceMismatch)
        } else {
            Ok(())
        }
    }

    /// Return the readiness expectation.
    pub const fn readiness(&self) -> ReadinessExpectation {
        self.readiness
    }

    /// Borrow the inherited descriptor-table binding.
    pub const fn inherited_fd_table(&self) -> &InheritedFdTable {
        &self.inherited_fd_table
    }

    /// Whether this ticket carries a static controller launch proof.
    pub const fn has_controller_launch_binding(&self) -> bool {
        self.controller_launch.is_some()
    }

    /// Return the target session generation bound to a static controller
    /// launch.
    pub const fn target_session_generation(&self) -> Option<ReconnectGeneration> {
        match self.controller_launch {
            Some(binding) => Some(binding.target_session_generation),
            None => None,
        }
    }

    /// Return the signed component descriptor commitment.
    pub const fn signed_descriptor_digest(&self) -> Option<ConfigurationDigest> {
        match self.controller_launch {
            Some(binding) => Some(binding.signed_descriptor_digest),
            None => None,
        }
    }

    /// Return the target readiness commitment.
    pub const fn target_readiness_digest(&self) -> Option<ConfigurationDigest> {
        match self.controller_launch {
            Some(binding) => Some(binding.target_readiness_digest),
            None => None,
        }
    }

    /// Whether this ticket carries a Core-issued assignment proof.
    pub const fn has_assignment_binding(&self) -> bool {
        self.assignment.is_some()
    }

    /// Return the Provider generation bound to this launch or assignment.
    pub const fn provider_generation(&self) -> Option<ResourceGeneration> {
        match self.assignment {
            Some(binding) => Some(binding.provider_generation),
            None => match self.controller_launch {
                Some(binding) => Some(binding.provider_generation),
                None => None,
            },
        }
    }

    /// Return the authenticated session generation bound to the assignment.
    pub const fn session_generation(&self) -> Option<ReconnectGeneration> {
        match self.assignment {
            Some(binding) => Some(binding.session_generation),
            None => None,
        }
    }

    /// Return the assignment epoch bound to the assignment.
    pub const fn assignment_epoch(&self) -> Option<u64> {
        match self.assignment {
            Some(binding) => Some(binding.assignment_epoch),
            None => None,
        }
    }

    /// Borrow the opaque ResourceClient binding commitment.
    pub const fn resource_client_binding(&self) -> Option<ConfigurationDigest> {
        match self.assignment {
            Some(binding) => Some(binding.resource_client_binding),
            None => None,
        }
    }
}

impl fmt::Debug for LaunchTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LaunchTicket(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    #[test]
    fn a_user_domain_ticket_without_an_exact_user_ref_is_rejected() {
        let error = fixtures::ticket_builder()
            .domain(ExecutionDomain::User)
            .user_ref(None)
            .build()
            .unwrap_err();
        assert_eq!(error, ProcessConformanceError::UserRefRequired);
    }

    #[test]
    fn folded_references_are_type_checked() {
        assert_eq!(
            fixtures::ticket_builder()
                .execution_ref(ResourceRef::parse("Provider/system-core").unwrap())
                .build()
                .unwrap_err(),
            ProcessConformanceError::InvalidTicket
        );
        assert_eq!(
            fixtures::ticket_builder()
                .process_ref(ResourceRef::parse("Volume/state").unwrap())
                .build()
                .unwrap_err(),
            ProcessConformanceError::InvalidTicket
        );
        assert_eq!(
            fixtures::ticket_builder()
                .domain(ExecutionDomain::User)
                .user_ref(Some(ResourceRef::parse("Host/host-system").unwrap()))
                .build()
                .unwrap_err(),
            ProcessConformanceError::InvalidTicket
        );
        assert!(
            fixtures::ticket_builder()
                .execution_ref(ResourceRef::parse("Guest/dev-vm").unwrap())
                .build()
                .is_ok()
        );
    }

    #[test]
    fn guest_process_tickets_require_target_execution_binding() {
        let ticket = fixtures::ticket_builder()
            .execution_ref(ResourceRef::parse("Guest/dev-vm").unwrap())
            .without_guest_execution_binding()
            .build()
            .unwrap();
        assert_eq!(
            ticket.validate(),
            Err(ProcessConformanceError::InvalidTicket)
        );
    }

    #[test]
    fn guest_execution_binding_matches_assignment_identity() {
        let session = d2b_contracts_resource::v3::identity::ReconnectGeneration::new(3).unwrap();
        let binding = GuestExecutionBinding::new(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ConfigurationDigest::from_bytes([9; 32]),
            session,
            7,
            ResourceGeneration::new(2).unwrap(),
            ControllerGeneration::new(1).unwrap(),
        )
        .unwrap();
        let ticket = fixtures::ticket_builder()
            .execution_ref(ResourceRef::parse("Guest/dev-vm").unwrap())
            .without_guest_execution_binding()
            .build()
            .unwrap()
            .with_guest_execution_binding(binding)
            .unwrap()
            .with_resource_revision(ZoneRevision::new(1))
            .unwrap()
            .with_assignment_binding(
                ResourceGeneration::new(2).unwrap(),
                session,
                7,
                ConfigurationDigest::from_bytes([10; 32]),
            )
            .unwrap();
        assert!(ticket.validate_assignment().is_ok());
    }

    #[test]
    fn cross_target_process_reference_is_guest_bound_and_single_use() {
        let guest = ResourceRef::parse("Guest/dev-vm").unwrap();
        let bound = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_target_ref(guest.clone())
            .unwrap();

        assert_eq!(bound.target_ref(), Some(&guest));
        assert!(bound.validate().is_ok());
        assert_eq!(
            bound.clone().with_target_ref(guest),
            Err(ProcessConformanceError::InvalidTicket)
        );
        assert_eq!(
            fixtures::ticket_builder()
                .build()
                .unwrap()
                .with_target_ref(ResourceRef::parse("Host/host-system").unwrap()),
            Err(ProcessConformanceError::InvalidTicket)
        );
    }

    #[test]
    fn the_deadline_is_bounded_and_the_ticket_debug_is_redacted() {
        let uid = fixtures::operation_uid();
        assert!(OperationBinding::new(uid.clone(), 0).is_err());
        assert!(OperationBinding::new(uid.clone(), MAX_LAUNCH_DEADLINE_MS + 1).is_err());
        assert!(OperationBinding::new(uid, MAX_LAUNCH_DEADLINE_MS).is_ok());
        let ticket = fixtures::ticket_builder().build().unwrap();
        assert_eq!(format!("{ticket:?}"), "LaunchTicket(<redacted>)");
    }

    #[test]
    fn controller_launch_proof_cannot_carry_assignment_authority() {
        assert_eq!(
            fixtures::ticket_builder()
                .build()
                .unwrap()
                .with_resource_revision(ZoneRevision::new(1))
                .unwrap()
                .validate_controller_launch(),
            Err(ProcessConformanceError::InvalidTicket)
        );
        let ticket = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_resource_revision(ZoneRevision::new(1))
            .unwrap()
            .with_controller_launch_binding(
                ResourceGeneration::new(2).unwrap(),
                d2b_contracts_resource::v3::identity::ReconnectGeneration::new(2).unwrap(),
                ConfigurationDigest::from_bytes([8; 32]),
                ConfigurationDigest::from_bytes([9; 32]),
            )
            .unwrap();
        assert!(ticket.has_controller_launch_binding());
        assert!(!ticket.has_assignment_binding());
        assert!(ticket.resource_client_binding().is_none());
        assert_eq!(format!("{ticket:?}"), "LaunchTicket(<redacted>)");
        assert_eq!(
            ticket.with_assignment_binding(
                ResourceGeneration::new(2).unwrap(),
                d2b_contracts_resource::v3::identity::ReconnectGeneration::new(2).unwrap(),
                1,
                ConfigurationDigest::from_bytes([10; 32]),
            ),
            Err(ProcessConformanceError::InvalidTicket)
        );
    }

    #[test]
    fn runtime_scope_commitment_is_incarnation_and_zone_bound() {
        let zone_a = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("zone uid");
        let zone_b = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let guest = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").expect("guest uid");
        let resource =
            ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").expect("resource uid");
        let other_resource =
            ResourceUid::parse("523e4567-e89b-42d3-a456-426614174004").expect("resource uid");
        let process_ref = ResourceRef::parse("Process/cloud-hypervisor").expect("process ref");
        let first = runtime_scope_commitment(
            &zone_a,
            Some(&guest),
            &process_ref,
            &resource,
            "cloud-hypervisor",
            1,
        );

        assert_eq!(
            first,
            runtime_scope_commitment(
                &zone_a,
                Some(&guest),
                &process_ref,
                &resource,
                "cloud-hypervisor",
                1,
            )
        );
        assert_ne!(
            first,
            runtime_scope_commitment(
                &zone_b,
                Some(&guest),
                &process_ref,
                &resource,
                "cloud-hypervisor",
                1,
            )
        );
        assert_ne!(
            first,
            runtime_scope_commitment(
                &zone_a,
                Some(&guest),
                &process_ref,
                &resource,
                "cloud-hypervisor",
                2,
            )
        );
        assert_ne!(
            first,
            runtime_scope_commitment(
                &zone_a,
                Some(&guest),
                &process_ref,
                &other_resource,
                "cloud-hypervisor",
                1,
            )
        );
    }

    #[test]
    fn assignment_binding_requires_nonzero_session_epoch_and_client_commitment() {
        let ticket = fixtures::ticket_builder().build().unwrap();
        let session = d2b_contracts_resource::v3::identity::ReconnectGeneration::new(3).unwrap();
        let bound = ticket
            .with_resource_revision(ZoneRevision::new(1))
            .unwrap()
            .with_assignment_binding(
                ResourceGeneration::new(2).unwrap(),
                session,
                7,
                ConfigurationDigest::from_bytes([11; 32]),
            )
            .unwrap();
        assert!(bound.has_assignment_binding());
        assert_eq!(bound.provider_generation().unwrap().get(), 2);
        assert_eq!(bound.session_generation().unwrap().get(), 3);
        assert_eq!(bound.assignment_epoch(), Some(7));
        assert_eq!(
            bound.resource_client_binding(),
            Some(ConfigurationDigest::from_bytes([11; 32]))
        );
    }

    #[test]
    fn malformed_readiness_is_rejected_by_ticket_validation() {
        let ticket = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_readiness(ReadinessExpectation::Condition { timeout_ms: 0 });
        assert_eq!(
            ticket.validate(),
            Err(ProcessConformanceError::InvalidTicket)
        );
    }

    #[test]
    fn an_expected_process_identity_seal_rejects_a_reused_identity() {
        let ticket = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_expected_identity_digest(ProcessIdentityDigest::from_bytes([1; 32]))
            .unwrap();
        assert!(
            ticket
                .validate_process_identity(&ProcessIdentityDigest::from_bytes([1; 32]))
                .is_ok()
        );
        assert_eq!(
            ticket.validate_process_identity(&ProcessIdentityDigest::from_bytes([2; 32])),
            Err(ProcessConformanceError::TerminalEvidenceMismatch)
        );
    }

    #[test]
    fn runtime_identity_binding_is_private_and_validated() {
        let zone_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("zone uid");
        let owner_ref = ResourceRef::parse("Guest/workload").expect("owner ref");
        let process_ref = ResourceRef::parse("Process/cloud-hypervisor").expect("process ref");
        let process_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("process uid");
        let scope = runtime_scope_commitment(
            &zone_uid,
            Some(&process_uid),
            &process_ref,
            &process_uid,
            "cloud-hypervisor",
            3,
        );
        let ticket = fixtures::ticket_builder()
            .build()
            .unwrap()
            .with_runtime_identity(zone_uid.clone(), Some(owner_ref.clone()), scope)
            .unwrap();

        assert_eq!(ticket.zone_uid(), Some(&zone_uid));
        assert_eq!(ticket.owner_ref(), Some(&owner_ref));
        assert_eq!(ticket.runtime_scope(), Some(scope));
        assert!(ticket.validate().is_ok());
        assert_eq!(format!("{ticket:?}"), "LaunchTicket(<redacted>)");
    }
}
