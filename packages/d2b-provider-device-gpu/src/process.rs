//! GPU, render-node, and video Process declarations.

use core::fmt;
use d2b_contracts::v3::{ResourceUid, device::DeviceArbitration};

use crate::settings::{GpuSettings, GpuSettingsError};

/// Combined GPU Provider Process role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProcessRole {
    /// Full GPU virtio worker.
    FullGpu,
    /// Render-node-only worker.
    RenderNode,
    /// Separate crosvm video decoder.
    Video,
}

/// Deterministic Process resource declaration.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuProcessDeclaration {
    name: String,
    role: GpuProcessRole,
    placement: &'static str,
}

impl GpuProcessDeclaration {
    /// Construct a declaration from a Device UID and selected role.
    pub fn new(
        device_uid: &ResourceUid,
        role: GpuProcessRole,
    ) -> Result<Self, GpuProcessSelectionError> {
        Ok(Self {
            name: gpu_process_name(device_uid, role)?,
            role,
            placement: "host",
        })
    }

    /// Borrow the Process resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the Process role.
    pub const fn role(&self) -> GpuProcessRole {
        self.role
    }

    /// Return the fixed Host placement.
    pub const fn placement(&self) -> &'static str {
        self.placement
    }
}

impl fmt::Debug for GpuProcessDeclaration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuProcessDeclaration")
            .field("name", &self.name)
            .field("role", &self.role)
            .field("placement", &self.placement)
            .finish()
    }
}

/// Closed Process selection failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProcessSelectionError {
    /// Settings failed the common Device arbitration rule.
    Settings(GpuSettingsError),
    /// The Device UID could not produce a canonical short name.
    InvalidUid,
}

impl fmt::Display for GpuProcessSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Settings(error) => return error.fmt(formatter),
            Self::InvalidUid => "gpu-process-uid-invalid",
        })
    }
}

impl std::error::Error for GpuProcessSelectionError {}

/// Select the worker set for one Device.
pub fn select_processes(
    device_uid: &ResourceUid,
    arbitration: DeviceArbitration,
    settings: &GpuSettings,
) -> Result<Vec<GpuProcessDeclaration>, GpuProcessSelectionError> {
    settings
        .validate(arbitration)
        .map_err(GpuProcessSelectionError::Settings)?;
    let role = if settings.render_node_only {
        GpuProcessRole::RenderNode
    } else {
        GpuProcessRole::FullGpu
    };
    let mut declarations = vec![GpuProcessDeclaration::new(device_uid, role)?];
    if settings.video_sidecar {
        declarations.push(GpuProcessDeclaration::new(
            device_uid,
            GpuProcessRole::Video,
        )?);
    }
    Ok(declarations)
}

/// Derive the required `device-<uid-short>-*` name.
pub fn gpu_process_name(
    device_uid: &ResourceUid,
    role: GpuProcessRole,
) -> Result<String, GpuProcessSelectionError> {
    let short = device_uid
        .as_str()
        .bytes()
        .filter(|byte| *byte != b'-')
        .take(12)
        .collect::<Vec<_>>();
    if short.len() != 12 {
        return Err(GpuProcessSelectionError::InvalidUid);
    }
    let component = match role {
        GpuProcessRole::FullGpu => "gpu",
        GpuProcessRole::RenderNode => "render-node",
        GpuProcessRole::Video => "video",
    };
    let short = String::from_utf8(short).map_err(|_| GpuProcessSelectionError::InvalidUid)?;
    Ok(format!("device-{short}-{component}"))
}
