//! Status-first component descriptor contract.

use core::fmt;

/// Signed component descriptor for the GPU Provider.
///
/// GPU operational state is represented by Device status and Core operation
/// rows. This descriptor intentionally declares no Provider state Volume or
/// `/state` mount.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuComponentDescriptor {
    provider_state_volumes: Vec<&'static str>,
    controller_mounts: Vec<&'static str>,
    worker_mounts: Vec<&'static str>,
}

impl GpuComponentDescriptor {
    /// Build the canonical status-first descriptor.
    pub const fn new() -> Self {
        Self {
            provider_state_volumes: Vec::new(),
            controller_mounts: Vec::new(),
            worker_mounts: Vec::new(),
        }
    }

    /// Whether the Provider declares no state Volume.
    pub fn provider_state_empty(&self) -> bool {
        self.provider_state_volumes.is_empty()
    }

    /// Borrow controller mounts.
    pub fn controller_mounts(&self) -> &[&'static str] {
        &self.controller_mounts
    }

    /// Borrow worker mounts.
    pub fn worker_mounts(&self) -> &[&'static str] {
        &self.worker_mounts
    }

    /// Validate the status-first invariant.
    pub fn validate(&self) -> Result<(), GpuDescriptorError> {
        if self.provider_state_volumes.is_empty()
            && self
                .controller_mounts
                .iter()
                .all(|mount| *mount != "/state")
            && self.worker_mounts.iter().all(|mount| *mount != "/state")
        {
            Ok(())
        } else {
            Err(GpuDescriptorError::StateVolumeDeclared)
        }
    }
}

impl Default for GpuComponentDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GpuComponentDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuComponentDescriptor")
            .field(
                "provider_state_volume_count",
                &self.provider_state_volumes.len(),
            )
            .field("controller_mount_count", &self.controller_mounts.len())
            .field("worker_mount_count", &self.worker_mounts.len())
            .finish()
    }
}

/// Descriptor validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuDescriptorError {
    /// A Provider state Volume or `/state` mount was declared.
    StateVolumeDeclared,
}

impl fmt::Display for GpuDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("gpu-provider-state-volume-forbidden")
    }
}

impl std::error::Error for GpuDescriptorError {}
