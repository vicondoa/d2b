//! Canonical Process ResourceSpec and LaunchTicket construction.

pub use d2b_contracts_resource::v3::ProcessSpec;
use d2b_contracts_resource::v3::{
    DesiredLifecycle, DeviceAccess, DeviceUsageSpec, EnvironmentClass, ExecutionSpec,
    HealthCheckClass, HealthCheckSpec, MountAccess, MountSpec, NamespaceClass, NetworkUsageSpec,
    ProcessClass, ReadinessClass, ReadinessSpec, RestartClass, RestartPolicySpec, SandboxSpec,
    TelemetrySpec,
};
use d2b_contracts_resource::v3::{
    ResourceRef,
    execution_policy::{BoundedToken, BudgetSpec, CountBudget, DurationMs, ExecutionDomain},
};
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Process template id.
pub const PROCESS_TEMPLATE: &str = "qemu-media-runner";

/// Attachment kind delivered through Core's private LaunchTicket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttachmentKind {
    /// KVM device fd.
    Kvm,
    /// Network tap fd.
    Tap,
    /// Media Volume fd.
    Media,
    /// Wayland display fd.
    Display,
    /// QMP Endpoint connection.
    Qmp,
    /// Serial Endpoint connection.
    Serial,
}


use crate::controller::attachments::{AdmittedAttachments, LaunchAttachments};

/// Construct the canonical qemu-media worker Process base spec.
pub fn build_process_spec(
    execution_ref: ResourceRef,
    runtime_volume_ref: ResourceRef,
    device_ref: Option<ResourceRef>,
    network_refs: impl IntoIterator<Item = ResourceRef>,
) -> Result<ProcessSpec, ProcessSpecError> {
    if execution_ref.resource_type().as_str() != "Host"
        || runtime_volume_ref.resource_type().as_str() != "Volume"
        || device_ref
            .as_ref()
            .is_some_and(|reference| reference.resource_type().as_str() != "Device")
    {
        return Err(ProcessSpecError::InvalidReference);
    }
    let network_refs: Vec<_> = network_refs.into_iter().collect();
    if network_refs.len() > 1
        || network_refs
            .iter()
            .any(|reference| reference.resource_type().as_str() != "Network")
    {
        return Err(ProcessSpecError::InvalidReference);
    }

    let runtime_mount = MountSpec::new(
        runtime_volume_ref,
        BoundedToken::parse("runner").map_err(|_| ProcessSpecError::InvalidShape)?,
        "/run/qemu",
        MountAccess::ReadWrite,
        true,
    )
    .map_err(|_| ProcessSpecError::InvalidShape)?;
    let sandbox = SandboxSpec::new(
        vec![NamespaceClass::Pid, NamespaceClass::Mount],
        Vec::new(),
        BoundedToken::parse(PROCESS_TEMPLATE).map_err(|_| ProcessSpecError::InvalidShape)?,
        true,
        false,
        EnvironmentClass::Minimal,
        true,
        Some("0022".to_owned()),
        200,
        None,
    )
    .map_err(|_| ProcessSpecError::InvalidShape)?;
    let budget = BudgetSpec::new(
        None,
        None,
        Some(CountBudget { limit: Some(512) }),
        Some(CountBudget { limit: Some(1024) }),
        None,
        None,
        None,
    )
    .map_err(|_| ProcessSpecError::InvalidShape)?;
    let network_usage = network_refs
        .into_iter()
        .next()
        .map(|network_ref| NetworkUsageSpec::new(Some(network_ref), Vec::new(), true))
        .transpose()
        .map_err(|_| ProcessSpecError::InvalidShape)?;
    let device_usage = device_ref
        .map(|device_ref| {
            DeviceUsageSpec::new(device_ref, DeviceAccess::Shared, "kvm-acceleration")
        })
        .transpose()
        .map_err(|_| ProcessSpecError::InvalidShape)?
        .into_iter()
        .collect();
    let execution = ExecutionSpec::new(
        execution_ref,
        Some(ExecutionDomain::System),
        None,
        ProcessClass::Worker,
        BoundedToken::parse(PROCESS_TEMPLATE).map_err(|_| ProcessSpecError::InvalidShape)?,
        None,
        Vec::new(),
        vec![runtime_mount],
        sandbox,
        budget,
        network_usage,
        device_usage,
        TelemetrySpec::default(),
    )
    .map_err(|_| ProcessSpecError::InvalidShape)?;
    let process = ProcessSpec::new(
        execution,
        DesiredLifecycle::Running,
        RestartPolicySpec::new(
            RestartClass::Never,
            duration("1s", 0, 60_000)?,
            duration("60s", 1_000, 3_600_000)?,
            2_000,
            None,
            duration("300s", 0, 86_400_000)?,
        )
        .map_err(|_| ProcessSpecError::InvalidShape)?,
        ReadinessSpec::new(
            duration("0s", 0, 300_000)?,
            duration("30s", 1_000, 300_000)?,
            1,
            1,
            ReadinessClass::ProviderDefined,
        )
        .map_err(|_| ProcessSpecError::InvalidShape)?,
        HealthCheckSpec::new(
            true,
            duration("10s", 1_000, 3_600_000)?,
            duration("5s", 1_000, 60_000)?,
            3,
            HealthCheckClass::ProviderDefined,
        )
        .map_err(|_| ProcessSpecError::InvalidShape)?,
        d2b_contracts_resource::v3::AdoptionPolicy::AdoptOnRestart,
        duration("30s", 0, 3_600_000)?,
    )
    .map_err(|_| ProcessSpecError::InvalidShape)?;
    validate_process_spec(&process)?;
    Ok(process)
}

/// Validate a qemu-media Process against the canonical v3 Process contract.
pub fn validate_process_spec(process: &ProcessSpec) -> Result<(), ProcessSpecError> {
    let encoded = serde_json::to_vec(process).map_err(|_| ProcessSpecError::InvalidShape)?;
    let decoded: ProcessSpec =
        serde_json::from_slice(&encoded).map_err(|_| ProcessSpecError::InvalidShape)?;
    if decoded != *process {
        return Err(ProcessSpecError::InvalidShape);
    }
    let execution = process.execution();
    if execution.execution_ref().resource_type().as_str() != "Host"
        || execution.process_class() != ProcessClass::Worker
        || execution.template().as_str() != PROCESS_TEMPLATE
        || execution.sandbox().namespace_classes() != [NamespaceClass::Pid, NamespaceClass::Mount]
        || !execution.sandbox().capability_classes().is_empty()
        || !execution.sandbox().no_new_privileges()
        || execution.sandbox().start_root()
        || !execution.sandbox().read_only_root()
        || execution.sandbox().seccomp_class().as_str() != PROCESS_TEMPLATE
        || execution.device_usage().len() > 1
        || process.restart_policy().class() != RestartClass::Never
        || process.desired_lifecycle() != DesiredLifecycle::Running
    {
        return Err(ProcessSpecError::InvalidShape);
    }
    if execution.mounts().len() != 1
        || execution.mounts()[0].mount_path() != "/run/qemu"
        || execution.mounts()[0].view().as_str() != "runner"
    {
        return Err(ProcessSpecError::InvalidShape);
    }
    Ok(())
}

fn duration(value: &str, min_millis: u64, max_millis: u64) -> Result<DurationMs, ProcessSpecError> {
    DurationMs::parse(value, min_millis, max_millis).map_err(|_| ProcessSpecError::InvalidShape)
}


/// Opaque Core LaunchTicket.
///
/// The ticket is a private carrier, not an authority surface. Its
/// [`LaunchAttachments::Admitted`] form is the converted model: every
/// descriptor already carries the relationship, reservation, and right that
/// authorized it, so a descriptor list that named a source the Guest never
/// bound cannot be built through it (R15, R16, R34).
///
/// The [`LaunchAttachments::Declared`] form is the pre-graph staging shape the
/// unchanged daemon composition still constructs. U34 deletes it together
/// with the snapshot fields that feed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTicket {
    /// Process resource template.
    pub process: ProcessSpec,
    /// Authorized attachment slots.
    pub attachments: LaunchAttachments,
}

impl LaunchTicket {
    /// Construct a ticket from the descriptors the graph admitted.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessSpecError`] when the process spec fails validation or
    /// when two descriptors claim the same private slot label.
    pub fn admitted(
        process: ProcessSpec,
        attachments: AdmittedAttachments,
    ) -> Result<Self, ProcessSpecError> {
        validate_process_spec(&process)?;
        let ticket = Self {
            process,
            attachments: LaunchAttachments::Admitted(attachments),
        };
        ticket.validate()?;
        Ok(ticket)
    }

    /// Construct a ticket from the pre-graph declared refs.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessSpecError`] when the process spec fails validation or
    /// when a declared ref does not match its slot's ResourceType.
    pub fn declared(
        process: ProcessSpec,
        media_refs: impl IntoIterator<Item = d2b_contracts_resource::v3::ResourceRef>,
        display_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    ) -> Result<Self, ProcessSpecError> {
        validate_process_spec(&process)?;
        let declared = crate::controller::attachments::DeclaredAttachments {
            media_refs: media_refs.into_iter().collect(),
            display_ref,
        };
        let slots = declared.project(&process)?;
        let ticket = Self {
            process,
            attachments: LaunchAttachments::Declared(slots),
        };
        ticket.validate()?;
        Ok(ticket)
    }

    /// Return the private slot labels, in launch order.
    pub fn labels(&self) -> Vec<&str> {
        self.attachments.labels()
    }

    /// Validate the process spec and the private slot labels.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessSpecError::InvalidShape`] when the process spec fails
    /// validation and [`ProcessSpecError::DuplicateAttachmentSlot`] when two
    /// descriptors claim the same label.
    pub fn validate(&self) -> Result<(), ProcessSpecError> {
        validate_process_spec(&self.process)?;
        let labels: BTreeSet<&str> = match &self.attachments {
            LaunchAttachments::Admitted(attachments) => {
                attachments
                    .validate_labels()
                    .map_err(|_| ProcessSpecError::DuplicateAttachmentSlot)?
            }
            LaunchAttachments::Declared(slots) => {
                let mut labels = BTreeSet::new();
                for slot in slots {
                    if !labels.insert(slot.slot.as_str()) {
                        return Err(ProcessSpecError::DuplicateAttachmentSlot);
                    }
                }
                labels
            }
        };
        debug_assert!(!labels.is_empty());
        Ok(())
    }
}

/// Process spec failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSpecError {
    /// A reference has the wrong ResourceType.
    InvalidReference,
    /// The canonical Process shape was changed.
    InvalidShape,
    /// Two attachment slots have the same label.
    DuplicateAttachmentSlot,
}
