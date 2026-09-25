//! Signed semantic Process worker declarations.

use crate::{GpuProcessRole, GpuProcessSelectionError, GpuSettings, process::GpuProcessDeclaration};
use d2b_contracts_resource::v3::ResourceUid;

/// Signed semantic GPU worker specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuWorkerSpec {
    process: GpuProcessDeclaration,
    template: &'static str,
}

impl GpuWorkerSpec {
    /// Build the fixed GPU or render-node worker shape.
    ///
    /// # Errors
    ///
    /// Returns [`GpuProcessSelectionError`] when the Device UID or role does
    /// not admit a GPU worker process declaration.
    pub fn gpu(
        device_uid: &ResourceUid,
        settings: &GpuSettings,
    ) -> Result<Self, GpuProcessSelectionError> {
        let role = if settings.render_node_only {
            GpuProcessRole::RenderNode
        } else {
            GpuProcessRole::FullGpu
        };
        let process = GpuProcessDeclaration::new(device_uid, role)?;
        let template = match role {
            GpuProcessRole::FullGpu => "gpu-worker",
            GpuProcessRole::RenderNode => "gpu-render-node",
            GpuProcessRole::Video => unreachable!("GPU settings select a GPU role"),
        };
        Ok(Self { process, template })
    }

    /// Borrow the deterministic Process declaration.
    pub const fn process(&self) -> &GpuProcessDeclaration {
        &self.process
    }

    /// Return the signed component template.
    pub const fn template(&self) -> &'static str {
        self.template
    }
}

/// Signed semantic video worker specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoWorkerSpec {
    process: GpuProcessDeclaration,
    template: &'static str,
}

impl VideoWorkerSpec {
    /// Build the separate video worker shape.
    ///
    /// # Errors
    ///
    /// Returns [`GpuProcessSelectionError`] when the Device UID or role does
    /// not admit a video worker process declaration.
    pub fn new(
        device_uid: &ResourceUid,
        settings: &GpuSettings,
    ) -> Result<Self, GpuProcessSelectionError> {
        let process = GpuProcessDeclaration::new(device_uid, GpuProcessRole::Video)?;
        // `videoNvidiaDecode` selects the closed NVIDIA-decode posture, not a
        // grant: the launch's device binds come from the template's entry in
        // `d2b-core`'s `device_worker_posture`, and the plain template never
        // widens to the NVIDIA nodes.
        let template = if settings.video_nvidia_decode {
            "video-worker-nvidia"
        } else {
            "video-worker"
        };
        Ok(Self { process, template })
    }

    /// Borrow the deterministic Process declaration.
    pub const fn process(&self) -> &GpuProcessDeclaration {
        &self.process
    }

    /// Return the signed component template.
    pub const fn template(&self) -> &'static str {
        self.template
    }
}
