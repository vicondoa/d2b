//! Clipboard controller placement and lifecycle projections.

use d2b_contracts::v3::ResourceRef;

/// Display dependency state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyStatus {
    /// Dependency is Ready.
    Ready,
    /// No display Provider was configured; host-only mode.
    Absent,
    /// Dependency exists but is not Ready.
    Degraded,
}

/// Core-created clipboard Process projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessPlan {
    /// Process template.
    pub template: &'static str,
    /// Process domain.
    pub domain: &'static str,
    /// Execution reference.
    pub execution_ref: ResourceRef,
    /// Optional user reference.
    pub user_ref: Option<ResourceRef>,
    /// Whether a Provider state Volume is mounted.
    pub mounts_state_volume: bool,
}

/// Clipboard controller.
pub struct ClipboardController {
    execution_ref: ResourceRef,
    user_ref: ResourceRef,
}

impl ClipboardController {
    /// Construct a controller for Host/system and User placement.
    pub fn new(
        execution_ref: impl AsRef<str>,
        user_ref: impl AsRef<str>,
    ) -> Result<Self, &'static str> {
        let execution_ref = ResourceRef::parse(execution_ref.as_ref())
            .map_err(|_| "clipboard-placement-invalid")?;
        let user_ref =
            ResourceRef::parse(user_ref.as_ref()).map_err(|_| "clipboard-placement-invalid")?;
        if execution_ref.resource_type().as_str() != "Host"
            || user_ref.resource_type().as_str() != "User"
        {
            return Err("clipboard-placement-invalid");
        }
        Ok(Self {
            execution_ref,
            user_ref,
        })
    }

    /// Return display dependency state for an optional dependency.
    pub const fn dependency_status(&self, display_ready: Option<bool>) -> DependencyStatus {
        match display_ready {
            None => DependencyStatus::Absent,
            Some(true) => DependencyStatus::Ready,
            Some(false) => DependencyStatus::Degraded,
        }
    }

    /// Return the two Core-created component plans.
    pub fn plan_processes(&self) -> Vec<ProcessPlan> {
        vec![
            ProcessPlan {
                template: "clipboard-controller",
                domain: "system",
                execution_ref: self.execution_ref.clone(),
                user_ref: None,
                mounts_state_volume: false,
            },
            ProcessPlan {
                template: "clipd-host",
                domain: "user",
                execution_ref: self.execution_ref.clone(),
                user_ref: Some(self.user_ref.clone()),
                mounts_state_volume: false,
            },
        ]
    }

    /// Clipboard has no Provider state Volume.
    pub const fn provider_state_set_empty(&self) -> bool {
        true
    }
}

impl core::fmt::Debug for ClipboardController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ClipboardController(<redacted>)")
    }
}
