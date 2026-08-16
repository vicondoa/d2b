//! Opaque Process ResourceSpec and LaunchTicket construction.

use d2b_contracts::v3::ResourceRef;
use serde::{Deserialize, Serialize};

/// Process spec provider.
pub const PROCESS_PROVIDER_REF: &str = "Provider/system-minijail";
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

/// One opaque LaunchTicket slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentSlot {
    /// Slot label.
    pub slot: String,
    /// Slot kind.
    pub kind: AttachmentKind,
    /// Authorizing ResourceRef.
    pub source_ref: ResourceRef,
}

/// Canonical qemu-media Process ResourceSpec projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessSpec {
    /// Process Provider.
    pub provider_ref: ResourceRef,
    /// Process execution reference.
    pub execution_ref: ResourceRef,
    /// Worker class.
    pub process_class: String,
    /// Signed template id.
    pub template: String,
    /// Runtime Volume ref.
    pub runtime_volume_ref: ResourceRef,
    /// KVM Device ref.
    pub device_ref: Option<ResourceRef>,
    /// Network refs.
    pub network_refs: Vec<ResourceRef>,
    /// Sandbox classes.
    pub namespace_classes: Vec<String>,
    /// Sandbox capabilities.
    pub capability_classes: Vec<String>,
    /// Sandbox seccomp class.
    pub seccomp_class: String,
    /// Whether no-new-privileges is required.
    pub no_new_privileges: bool,
    /// Whether the root filesystem is read-only.
    pub read_only_root: bool,
    /// Process restart policy.
    pub restart_policy: String,
    /// Desired lifecycle.
    pub desired_lifecycle: String,
}

impl ProcessSpec {
    /// Construct the canonical worker Process spec.
    pub fn new(
        guest_ref: ResourceRef,
        execution_ref: ResourceRef,
        runtime_volume_ref: ResourceRef,
        device_ref: Option<ResourceRef>,
        network_refs: impl IntoIterator<Item = ResourceRef>,
    ) -> Result<Self, ProcessSpecError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || execution_ref.resource_type().as_str() != "Host"
            || runtime_volume_ref.resource_type().as_str() != "Volume"
            || device_ref
                .as_ref()
                .is_some_and(|reference| reference.resource_type().as_str() != "Device")
        {
            return Err(ProcessSpecError::InvalidReference);
        }
        let network_refs: Vec<_> = network_refs.into_iter().collect();
        if network_refs
            .iter()
            .any(|reference| reference.resource_type().as_str() != "Network")
        {
            return Err(ProcessSpecError::InvalidReference);
        }
        Ok(Self {
            provider_ref: ResourceRef::parse(PROCESS_PROVIDER_REF)
                .expect("frozen Process Provider ref"),
            execution_ref,
            process_class: "worker".to_owned(),
            template: PROCESS_TEMPLATE.to_owned(),
            runtime_volume_ref,
            device_ref,
            network_refs,
            namespace_classes: vec!["pid".to_owned(), "mount".to_owned()],
            capability_classes: Vec::new(),
            seccomp_class: "qemu-media-runner".to_owned(),
            no_new_privileges: true,
            read_only_root: true,
            restart_policy: "never".to_owned(),
            desired_lifecycle: "running".to_owned(),
        })
    }

    /// Validate the no-ambient-authority Process shape.
    pub fn validate(&self) -> Result<(), ProcessSpecError> {
        if self.process_class != "worker"
            || self.template != PROCESS_TEMPLATE
            || !self.capability_classes.is_empty()
            || !self.no_new_privileges
            || !self.read_only_root
            || self.restart_policy != "never"
        {
            return Err(ProcessSpecError::InvalidShape);
        }
        Ok(())
    }
}

/// Opaque Core LaunchTicket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchTicket {
    /// Process resource template.
    pub process: ProcessSpec,
    /// Authorized attachment slots.
    pub attachments: Vec<AttachmentSlot>,
}

impl LaunchTicket {
    /// Construct a ticket from already-authorized refs.
    pub fn new(
        process: ProcessSpec,
        media_refs: impl IntoIterator<Item = ResourceRef>,
        display_ref: Option<ResourceRef>,
    ) -> Result<Self, ProcessSpecError> {
        process.validate()?;
        let mut attachments = Vec::new();
        if let Some(device_ref) = &process.device_ref {
            attachments.push(AttachmentSlot {
                slot: "kvm".to_owned(),
                kind: AttachmentKind::Kvm,
                source_ref: device_ref.clone(),
            });
        }
        for (index, reference) in process.network_refs.iter().enumerate() {
            attachments.push(AttachmentSlot {
                slot: format!("tap-{index}"),
                kind: AttachmentKind::Tap,
                source_ref: reference.clone(),
            });
        }
        for (index, reference) in media_refs.into_iter().enumerate() {
            if reference.resource_type().as_str() != "Volume" {
                return Err(ProcessSpecError::InvalidReference);
            }
            attachments.push(AttachmentSlot {
                slot: format!("media-{index}"),
                kind: AttachmentKind::Media,
                source_ref: reference,
            });
        }
        if let Some(reference) = display_ref {
            if reference.resource_type().as_str() != "Endpoint" {
                return Err(ProcessSpecError::InvalidReference);
            }
            attachments.push(AttachmentSlot {
                slot: "display".to_owned(),
                kind: AttachmentKind::Display,
                source_ref: reference,
            });
        }
        Ok(Self {
            process,
            attachments,
        })
    }

    /// Validate unique slot labels and typed sources.
    pub fn validate(&self) -> Result<(), ProcessSpecError> {
        self.process.validate()?;
        let mut slots = std::collections::BTreeSet::new();
        for attachment in &self.attachments {
            if !slots.insert(&attachment.slot) {
                return Err(ProcessSpecError::DuplicateAttachmentSlot);
            }
        }
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
