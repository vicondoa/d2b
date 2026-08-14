//! Clipboard controller placement and lifecycle projections.

use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_provider_display_wayland::DisplayDependencyProof;

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

/// Authenticated evidence that the display Provider is Ready for one User
/// and Zone generation.
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayDependencyEvidence {
    pub(crate) provider_ref: ResourceRef,
    pub(crate) zone: ZoneId,
    pub(crate) user_ref: ResourceRef,
    pub(crate) generation: u64,
}

impl DisplayDependencyEvidence {
    /// Consume Core-authenticated display readiness evidence.
    pub fn from_display_proof(proof: DisplayDependencyProof) -> Self {
        Self {
            provider_ref: proof.provider_ref().clone(),
            zone: proof.zone().clone(),
            user_ref: proof.user_ref().clone(),
            generation: proof.generation(),
        }
    }

    /// Borrow the authenticated display Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the authenticated Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the authenticated User.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    /// Return the Ready generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl core::fmt::Debug for DisplayDependencyEvidence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DisplayDependencyEvidence(REDACTED)")
    }
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

    /// Return display dependency state for authenticated evidence.
    pub fn dependency_status(
        &self,
        display: Option<&DisplayDependencyEvidence>,
    ) -> DependencyStatus {
        let Some(display) = display else {
            return DependencyStatus::Absent;
        };
        if display.provider_ref().resource_type().as_str() == "Provider"
            && display.user_ref() == &self.user_ref
            && display.generation() != 0
        {
            DependencyStatus::Ready
        } else {
            DependencyStatus::Degraded
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
