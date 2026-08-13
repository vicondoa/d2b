//! Process placement controller for desktop notification components.

use d2b_contracts::v3::ResourceRef;

/// A planned notification component process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessPlan {
    /// Stable process template.
    pub template: &'static str,
    /// Process execution domain.
    pub domain: &'static str,
    /// Whether a state Volume is mounted.
    pub mounts_state_volume: bool,
}

/// Notification placement controller.
pub struct NotificationController {
    provider_ref: ResourceRef,
}

impl NotificationController {
    /// Construct a controller for one exact Provider instance.
    pub fn new(provider_ref: impl AsRef<str>) -> Result<Self, &'static str> {
        let provider_ref = ResourceRef::parse(provider_ref.as_ref())
            .map_err(|_| "notification-provider-ref-invalid")?;
        if provider_ref.to_canonical_string() != crate::PROVIDER_REF {
            return Err("notification-provider-ref-invalid");
        }
        Ok(Self { provider_ref })
    }

    /// Plan component processes after the display dependency is Ready.
    pub fn plan(&self, display_ready: bool) -> Result<Vec<ProcessPlan>, &'static str> {
        if !display_ready {
            return Err("display-wayland-unavailable");
        }
        Ok(vec![
            ProcessPlan {
                template: "notification-desktop-controller",
                domain: "system",
                mounts_state_volume: false,
            },
            ProcessPlan {
                template: "notification-desktop-host-sink",
                domain: "user",
                mounts_state_volume: false,
            },
        ])
    }

    /// Notification state is transient and never has a Provider state Volume.
    pub const fn provider_state_set_empty(&self) -> bool {
        true
    }

    /// Borrow the exact Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }
}

impl core::fmt::Debug for NotificationController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationController(<redacted>)")
    }
}
