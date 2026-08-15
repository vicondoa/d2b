//! Dependency-gated VMM bootstrap graph.

use std::fmt;

use d2b_contracts::v3::ResourceRef;

/// Readiness of one dependency family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyReadiness {
    /// All required effects are ready.
    Ready,
    /// At least one dependency is still pending.
    Pending,
}

/// Opaque VMM attachment reference.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttachmentRef(String);

impl AttachmentRef {
    /// Construct a bounded opaque attachment ref.
    pub fn new(value: impl Into<String>) -> Result<Self, BootstrapGraphError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(BootstrapGraphError::InvalidReference);
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for AttachmentRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttachmentRef(<opaque>)")
    }
}

/// The dependency snapshot required before VMM launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapGraph {
    /// Device references.
    pub devices: Vec<ResourceRef>,
    /// Network references.
    pub networks: Vec<ResourceRef>,
    /// Virtiofs volume references.
    pub volumes: Vec<ResourceRef>,
    /// Opaque attachment tickets resolved by Core.
    pub attachments: Vec<AttachmentRef>,
}

impl BootstrapGraph {
    /// Construct and validate the explicit KVM rule.
    pub fn new(
        devices: Vec<ResourceRef>,
        networks: Vec<ResourceRef>,
        volumes: Vec<ResourceRef>,
        attachments: Vec<AttachmentRef>,
    ) -> Result<Self, BootstrapGraphError> {
        if devices
            .iter()
            .chain(networks.iter())
            .chain(volumes.iter())
            .any(|reference| reference.resource_type().as_str() == "Host")
        {
            return Err(BootstrapGraphError::InvalidReference);
        }
        Ok(Self {
            devices,
            networks,
            volumes,
            attachments,
        })
    }

    /// Check the dependency barrier.
    pub fn readiness(
        &self,
        devices_ready: bool,
        networks_ready: bool,
        volumes_ready: bool,
    ) -> DependencyReadiness {
        if devices_ready && networks_ready && volumes_ready {
            DependencyReadiness::Ready
        } else {
            DependencyReadiness::Pending
        }
    }
}

/// Bootstrap graph construction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapGraphError {
    /// A reference or opaque ticket was invalid.
    InvalidReference,
}
