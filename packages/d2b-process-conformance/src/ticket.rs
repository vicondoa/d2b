//! The Process-controller-authenticated LaunchTicket.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts::v3::{ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid};

use crate::error::ProcessConformanceError;
use crate::identity::{ConfigurationDigest, IdentityBinding};

/// Maximum launch deadline, matching the frozen resource-API request
/// lifetime ceiling of 900000 ms.
pub const MAX_LAUNCH_DEADLINE_MS: u32 = 900_000;

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

/// Operation, deadline, and cancellation binding of one launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBinding {
    operation_uid: ResourceUid,
    deadline_ms: u32,
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
        })
    }

    /// Borrow the operation identity.
    pub const fn operation_uid(&self) -> &ResourceUid {
        &self.operation_uid
    }

    /// Return the launch deadline in milliseconds.
    pub const fn deadline_ms(&self) -> u32 {
        self.deadline_ms
    }
}

/// The ticket a Process controller hands to the fixed process effect
/// adapter.
///
/// It binds the resource, the owning Provider component and template, the
/// placement, the selected Process Provider, the compiled configuration
/// digests, the operation, and the identity bindings the launch is expected
/// to establish. Nothing in it names an executable, a host path, a numeric
/// UID or GID, a cgroup path, a broker operation, or an environment value.
#[derive(Clone, PartialEq, Eq)]
pub struct LaunchTicket {
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
        if expected_identity.is_empty() {
            return Err(ProcessConformanceError::InvalidTicket);
        }
        Ok(Self {
            process_ref,
            process_uid,
            resource_generation,
            controller_generation,
            owner_provider,
            component,
            template,
            execution_ref,
            domain,
            user_ref,
            selected_provider,
            digests,
            operation,
            expected_identity,
        })
    }

    /// Borrow the Process or EphemeralProcess reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the resource UID.
    pub const fn process_uid(&self) -> &ResourceUid {
        &self.process_uid
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
    fn the_deadline_is_bounded_and_the_ticket_debug_is_redacted() {
        let uid = fixtures::operation_uid();
        assert!(OperationBinding::new(uid.clone(), 0).is_err());
        assert!(OperationBinding::new(uid.clone(), MAX_LAUNCH_DEADLINE_MS + 1).is_err());
        assert!(OperationBinding::new(uid, MAX_LAUNCH_DEADLINE_MS).is_ok());
        let ticket = fixtures::ticket_builder().build().unwrap();
        assert_eq!(format!("{ticket:?}"), "LaunchTicket(<redacted>)");
    }
}
