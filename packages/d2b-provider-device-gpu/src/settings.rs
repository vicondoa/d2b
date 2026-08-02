//! Strict GPU and video desired settings.

use core::fmt;
use d2b_contracts::v3::device::DeviceArbitration;
use serde::{Deserialize, Serialize};

/// Closed crosvm GPU context classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextType {
    /// Virgl context support.
    Virgl,
    /// Virgl2 context support.
    Virgl2,
    /// Cross-domain Wayland context support.
    CrossDomain,
}

/// One virtual display setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayConfig {
    /// Whether this display is hidden from the host compositor.
    pub hidden: bool,
}

/// Device-gpu Provider settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct GpuSettings {
    /// Render-node-only mode, which permits shared arbitration.
    pub render_node_only: bool,
    /// Spawn the separate video decoder worker.
    pub video_sidecar: bool,
    /// Expose the bounded NVIDIA decode devices to video.
    pub video_nvidia_decode: bool,
    /// Requested closed GPU context classes.
    pub context_types: Vec<ContextType>,
    /// Bounded virtual display list.
    pub displays: Vec<DisplayConfig>,
    /// Enable EGL.
    pub egl: bool,
    /// Enable Vulkan.
    pub vulkan: bool,
    /// Enable trusted cross-domain mode.
    pub cross_domain_trusted: bool,
    /// Experimental virgl video forwarding.
    pub virgl_video: bool,
}

impl Default for GpuSettings {
    fn default() -> Self {
        Self {
            render_node_only: false,
            video_sidecar: false,
            video_nvidia_decode: false,
            context_types: vec![
                ContextType::Virgl,
                ContextType::Virgl2,
                ContextType::CrossDomain,
            ],
            displays: vec![DisplayConfig { hidden: true }],
            egl: true,
            vulkan: true,
            cross_domain_trusted: false,
            virgl_video: false,
        }
    }
}

impl GpuSettings {
    /// Validate bounds and the shared-arbitration/render-node invariant.
    pub fn validate(&self, arbitration: DeviceArbitration) -> Result<(), GpuSettingsError> {
        if self.context_types.is_empty() || self.context_types.len() > 3 {
            return Err(GpuSettingsError::ContextTypesOutOfRange);
        }
        if self.displays.len() > 8 {
            return Err(GpuSettingsError::DisplaysOutOfRange);
        }
        if arbitration == DeviceArbitration::Shared && !self.render_node_only {
            return Err(GpuSettingsError::SharedRequiresRenderNodeOnly);
        }
        if self.video_sidecar && self.render_node_only {
            return Err(GpuSettingsError::VideoRequiresFullGpu);
        }
        if self.virgl_video && self.video_sidecar {
            return Err(GpuSettingsError::VideoModesConflict);
        }
        Ok(())
    }
}

/// Closed settings validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuSettingsError {
    /// Context types are empty or exceed the closed set.
    ContextTypesOutOfRange,
    /// More than eight displays were requested.
    DisplaysOutOfRange,
    /// Shared arbitration requires render-node-only mode.
    SharedRequiresRenderNodeOnly,
    /// The video sidecar requires a full GPU worker.
    VideoRequiresFullGpu,
    /// The two video modes cannot be enabled together.
    VideoModesConflict,
}

impl fmt::Display for GpuSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ContextTypesOutOfRange => "gpu-context-types-out-of-range",
            Self::DisplaysOutOfRange => "gpu-displays-out-of-range",
            Self::SharedRequiresRenderNodeOnly => "shared-arbitration-requires-render-node-only",
            Self::VideoRequiresFullGpu => "video-sidecar-requires-full-gpu",
            Self::VideoModesConflict => "gpu-video-modes-conflict",
        })
    }
}

impl std::error::Error for GpuSettingsError {}
