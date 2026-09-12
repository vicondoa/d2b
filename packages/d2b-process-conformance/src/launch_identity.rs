//! Canonical launch identity for one Process launch.
//!
//! A launch's identity is a composite of the row reference, the semantic
//! owner reference and UID, the exact execution target, the cross-target
//! selector, the broker VM scope, and the legacy runner role. Different
//! layers used to persist, derive, and compare different subsets of it, so an
//! incomplete identity was only discovered at the far end of the pipeline -
//! one fence field at a time.
//!
//! [`LaunchIdentity`] is that composite as one value. It is validated for
//! completeness at construction: a row that cannot name a launch fails once,
//! here, with the input that is missing or invalid named in
//! [`LaunchIdentityError`]. The launch ticket, the broker's identity fence,
//! and the driver's adopt/probe path all read this value instead of
//! re-deriving its fields.

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};

/// Maximum length of the VM scope or legacy role a launch identity carries.
const MAX_IDENTITY_NAME_BYTES: usize = 128;

/// The canonical, complete identity of one Process launch.
///
/// Construct it once per row through the row resolver (or the launching
/// layer's validated constructor) and pass the resolved value along: launch
/// ticket, broker fence, and adoption probe consume it unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    owner_ref: Option<ResourceRef>,
    owner_uid: Option<ResourceUid>,
    execution_ref: ResourceRef,
    target_ref: Option<ResourceRef>,
    vm: Option<String>,
    role: String,
    binding_worker: bool,
}

/// Named construction failure of a [`LaunchIdentity`].
///
/// Every variant names the exact input that was missing or invalid, so a
/// refused launch is diagnosable at the point of construction instead of at
/// the far end of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaunchIdentityError {
    /// The row's execution target is neither a Host nor a Guest reference.
    InvalidExecutionRef {
        /// The rejected execution reference.
        execution_ref: String,
    },
    /// A target selector that is not a Guest reference.
    InvalidTargetRef {
        /// The rejected target reference.
        target_ref: String,
    },
    /// A binding-owned serving worker that declares no attachment target.
    MissingTargetRef {
        /// The owner the missing target belongs to.
        owner_ref: String,
    },
    /// A durable owner UID with no owner reference to link it to.
    OwnerUidWithoutOwnerRef {
        /// The unlinked owner UID.
        owner_uid: String,
    },
    /// The derived VM scope is empty or carries forbidden bytes.
    InvalidVm {
        /// The rejected VM scope.
        vm: String,
    },
    /// The legacy runner role (the Process name) is empty or forbidden.
    InvalidRole {
        /// The rejected role.
        role: String,
    },
}

impl LaunchIdentityError {
    /// Stable diagnostic code for this construction failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidExecutionRef { .. } => "launch-identity-invalid-execution-ref",
            Self::InvalidTargetRef { .. } => "launch-identity-invalid-target-ref",
            Self::MissingTargetRef { .. } => "launch-identity-missing-target-ref",
            Self::OwnerUidWithoutOwnerRef { .. } => "launch-identity-owner-uid-without-owner-ref",
            Self::InvalidVm { .. } => "launch-identity-invalid-vm",
            Self::InvalidRole { .. } => "launch-identity-invalid-role",
        }
    }
}

impl std::fmt::Display for LaunchIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for LaunchIdentityError {}

impl LaunchIdentity {
    /// Resolve one launch identity from its row inputs.
    ///
    /// The VM scope is derived here, once:
    /// - a Guest execution target names its own VM,
    /// - a Host execution target names the selected Guest target, or the
    ///   owning Guest when the row declares no target,
    /// - otherwise the execution target names the VM implicitly.
    ///
    /// `binding_worker` marks the host-exec/guest-target split of a
    /// binding-owned serving worker; its launch VM is the execution host even
    /// though the ticket's target points at the attachment's Guest.
    pub fn new(
        owner_ref: Option<ResourceRef>,
        owner_uid: Option<ResourceUid>,
        execution_ref: ResourceRef,
        target_ref: Option<ResourceRef>,
        process_name: &str,
        binding_worker: bool,
    ) -> Result<Self, LaunchIdentityError> {
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(LaunchIdentityError::InvalidExecutionRef {
                execution_ref: execution_ref.to_canonical_string(),
            });
        }
        if let Some(target) = &target_ref
            && target.resource_type().as_str() != "Guest"
        {
            return Err(LaunchIdentityError::InvalidTargetRef {
                target_ref: target.to_canonical_string(),
            });
        }
        if owner_ref.is_none()
            && let Some(owner_uid) = &owner_uid
        {
            return Err(LaunchIdentityError::OwnerUidWithoutOwnerRef {
                owner_uid: owner_uid.as_str().to_owned(),
            });
        }
        if owner_ref
            .as_ref()
            .is_some_and(|owner| owner.resource_type().as_str() == "VolumeBinding")
            && target_ref.is_none()
        {
            return Err(LaunchIdentityError::MissingTargetRef {
                owner_ref: owner_ref
                    .as_ref()
                    .expect("binding owner is present")
                    .to_canonical_string(),
            });
        }
        if !valid_identity_name(process_name) {
            return Err(LaunchIdentityError::InvalidRole {
                role: process_name.to_owned(),
            });
        }
        let vm = match execution_ref.resource_type().as_str() {
            "Guest" => Some(execution_ref.name().as_str().to_owned()),
            _ => target_ref
                .as_ref()
                .map(|target| target.name().as_str().to_owned())
                .or_else(|| {
                    owner_ref
                        .as_ref()
                        .filter(|owner| owner.resource_type().as_str() == "Guest")
                        .map(|owner| owner.name().as_str().to_owned())
                }),
        };
        if let Some(vm) = &vm
            && !valid_identity_name(vm)
        {
            return Err(LaunchIdentityError::InvalidVm { vm: vm.clone() });
        }
        Ok(Self {
            owner_ref,
            owner_uid,
            execution_ref,
            target_ref,
            vm,
            role: process_name.to_owned(),
            binding_worker,
        })
    }

    /// Replace the derived VM scope with the one a signed bundle node names.
    ///
    /// The legacy process-DAG ticket path carries nodes whose VM name is the
    /// DAG's, which the execution target alone cannot name.
    pub fn with_vm(mut self, vm: &str) -> Result<Self, LaunchIdentityError> {
        if !valid_identity_name(vm) {
            return Err(LaunchIdentityError::InvalidVm { vm: vm.to_owned() });
        }
        self.vm = Some(vm.to_owned());
        Ok(self)
    }

    /// Attach the exact semantic owner.
    ///
    /// The binding-owned serving-worker split follows the owner kind: a
    /// `VolumeBinding`-owned launch is the virtiofsd worker whose attachment
    /// Guest is only its target.
    pub fn with_owner(mut self, owner_ref: ResourceRef) -> Result<Self, LaunchIdentityError> {
        self.binding_worker = owner_ref.resource_type().as_str() == "VolumeBinding";
        self.owner_ref = Some(owner_ref);
        self.rederive_vm()
    }

    /// Attach the durable owner UID linkage.
    pub fn with_owner_uid(mut self, owner_uid: ResourceUid) -> Result<Self, LaunchIdentityError> {
        if self.owner_ref.is_none() {
            return Err(LaunchIdentityError::OwnerUidWithoutOwnerRef {
                owner_uid: owner_uid.as_str().to_owned(),
            });
        }
        self.owner_uid = Some(owner_uid);
        Ok(self)
    }

    /// Attach a cross-target Guest selector.
    pub fn with_target_ref(
        mut self,
        target_ref: ResourceRef,
    ) -> Result<Self, LaunchIdentityError> {
        if target_ref.resource_type().as_str() != "Guest" {
            return Err(LaunchIdentityError::InvalidTargetRef {
                target_ref: target_ref.to_canonical_string(),
            });
        }
        self.target_ref = Some(target_ref);
        self.rederive_vm()
    }

    fn rederive_vm(mut self) -> Result<Self, LaunchIdentityError> {
        if self.execution_ref.resource_type().as_str() == "Guest" {
            self.vm = Some(self.execution_ref.name().as_str().to_owned());
            return Ok(self);
        }
        self.vm = self
            .target_ref
            .as_ref()
            .map(|target| target.name().as_str().to_owned())
            .or_else(|| {
                self.owner_ref
                    .as_ref()
                    .filter(|owner| owner.resource_type().as_str() == "Guest")
                    .map(|owner| owner.name().as_str().to_owned())
            });
        if let Some(vm) = &self.vm
            && !valid_identity_name(vm)
        {
            return Err(LaunchIdentityError::InvalidVm { vm: vm.clone() });
        }
        Ok(self)
    }

    /// The exact semantic owner, when the row has one.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// The durable owner UID linkage, when the row carries one.
    pub const fn owner_uid(&self) -> Option<&ResourceUid> {
        self.owner_uid.as_ref()
    }

    /// The exact execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// The cross-target Guest selector, when the row declares one.
    pub const fn target_ref(&self) -> Option<&ResourceRef> {
        self.target_ref.as_ref()
    }

    /// The VM scope the row's signed intent resolves under, when the row (or
    /// its owner) names one. A plain host row resolves without a VM
    /// restriction and returns `None`.
    pub fn vm(&self) -> Option<&str> {
        self.vm.as_deref()
    }

    /// The VM scope the launch is bound to: the row-resolved VM, or the
    /// execution target's own name when the row names none.
    pub fn vm_or_execution(&self) -> &str {
        self.vm
            .as_deref()
            .unwrap_or(self.execution_ref.name().as_str())
    }

    /// The broker VM scope of this launch. A binding-owned serving worker
    /// executes on the Host its signed template binds; the attachment's Guest
    /// is only the ticket's target ref, so its launch VM is the execution
    /// host.
    pub fn launch_vm(&self) -> &str {
        if self.binding_worker {
            self.execution_ref.name().as_str()
        } else {
            self.vm_or_execution()
        }
    }

    /// The legacy runner role the row requests (its Process name).
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Whether this is a binding-owned serving worker.
    pub const fn is_binding_worker(&self) -> bool {
        self.binding_worker
    }
}

fn valid_identity_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_NAME_BYTES
        && !value.bytes().any(|byte| byte == 0 || byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::ResourceRef;

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("valid reference")
    }

    #[test]
    fn host_execution_derives_the_launch_vm_from_the_target_or_owner() {
        let identity = LaunchIdentity::new(
            Some(reference("Guest/acceptance-guest")),
            None,
            reference("Host/host-system"),
            Some(reference("Guest/acceptance-guest")),
            "acceptance-guest-vmm",
            false,
        )
        .expect("complete identity");
        assert_eq!(identity.vm(), Some("acceptance-guest"));
        assert_eq!(identity.launch_vm(), "acceptance-guest");

        let binding_worker = LaunchIdentity::new(
            Some(reference("VolumeBinding/data")),
            None,
            reference("Host/host-system"),
            Some(reference("Guest/acceptance-guest")),
            "vol-vfd-deadbeef",
            true,
        )
        .expect("complete identity");
        assert_eq!(binding_worker.vm(), Some("acceptance-guest"));
        assert_eq!(binding_worker.launch_vm(), "host-system");

        let host_worker = LaunchIdentity::new(
            None,
            None,
            reference("Host/host-system"),
            None,
            "worker",
            false,
        )
        .expect("complete identity");
        assert_eq!(host_worker.vm(), None);
        assert_eq!(host_worker.launch_vm(), "host-system");
    }

    #[test]
    fn incomplete_rows_fail_with_the_missing_input_named() {
        let binding_without_target = LaunchIdentity::new(
            Some(reference("VolumeBinding/data")),
            None,
            reference("Host/host-system"),
            None,
            "vol-vfd-deadbeef",
            true,
        )
        .expect_err("binding worker without a target is incomplete");
        assert_eq!(
            binding_without_target,
            LaunchIdentityError::MissingTargetRef {
                owner_ref: "VolumeBinding/data".to_owned(),
            }
        );

        let invalid_execution = LaunchIdentity::new(
            None,
            None,
            reference("Provider/system-core"),
            None,
            "worker",
            false,
        )
        .expect_err("non-Host/Guest execution is invalid");
        assert_eq!(
            invalid_execution.code(),
            "launch-identity-invalid-execution-ref"
        );

        let invalid_target = LaunchIdentity::new(
            Some(reference("VolumeBinding/data")),
            None,
            reference("Host/host-system"),
            Some(reference("Host/host-system")),
            "vol-vfd-deadbeef",
            true,
        )
        .expect_err("target selectors are Guest references");
        assert_eq!(invalid_target.code(), "launch-identity-invalid-target-ref");

        assert_eq!(
            LaunchIdentity::new(
                None,
                Some(
                    ResourceUid::parse("323e4567-e89b-42d3-a456-426614174001")
                        .expect("owner uid")
                ),
                reference("Host/host-system"),
                None,
                "worker",
                false,
            )
            .expect_err("an owner uid needs an owner ref")
            .code(),
            "launch-identity-owner-uid-without-owner-ref"
        );
    }

    #[test]
    fn guest_execution_names_its_own_vm() {
        let identity = LaunchIdentity::new(
            Some(reference("Guest/acceptance-guest")),
            None,
            reference("Guest/acceptance-guest"),
            None,
            "agent",
            false,
        )
        .expect("complete identity");
        assert_eq!(identity.vm(), Some("acceptance-guest"));
        assert_eq!(identity.launch_vm(), "acceptance-guest");
    }

    #[test]
    fn owner_and_target_updates_rederive_the_vm() {
        let identity = LaunchIdentity::new(
            None,
            None,
            reference("Host/host-system"),
            None,
            "worker",
            false,
        )
        .expect("complete identity");
        assert_eq!(identity.vm(), None);

        let target = identity
            .with_target_ref(reference("Guest/acceptance-guest"))
            .expect("guest target");
        assert_eq!(target.vm(), Some("acceptance-guest"));
        assert_eq!(target.launch_vm(), "acceptance-guest");

        let owned = LaunchIdentity::new(
            None,
            None,
            reference("Host/host-system"),
            None,
            "worker",
            false,
        )
        .expect("complete identity")
        .with_owner(reference("Guest/acceptance-guest"))
        .expect("guest owner");
        assert_eq!(owned.vm(), Some("acceptance-guest"));
    }

    #[test]
    fn a_bundle_node_may_name_its_own_vm() {
        let identity = LaunchIdentity::new(
            None,
            None,
            reference("Host/host-system"),
            None,
            "ch-runner",
            false,
        )
        .expect("complete identity")
        .with_vm("corp-vm")
        .expect("bundle node vm");
        assert_eq!(identity.vm(), Some("corp-vm"));
        assert_eq!(identity.launch_vm(), "corp-vm");
    }
}
