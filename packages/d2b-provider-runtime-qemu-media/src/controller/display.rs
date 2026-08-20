//! WaylandSession dependency projection.

use d2b_contracts_zone_session::v3::ResourceRef;
use serde::{Deserialize, Serialize};

/// WaylandSession lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySessionPhase {
    /// Session is pending.
    Pending,
    /// Session is ready.
    Ready,
    /// Session failed.
    Failed,
}

/// Opaque endpoint attachment returned by display-wayland.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayAttachment {
    /// Endpoint ResourceRef.
    pub endpoint_ref: ResourceRef,
    /// Opaque attachment class.
    pub attachment_class: String,
}

/// Observed WaylandSession status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayObservation {
    /// Session phase.
    pub phase: DisplaySessionPhase,
    /// Optional endpoint attachment.
    pub attachment: Option<DisplayAttachment>,
}

/// Resource spec for the delegated WaylandSession.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaylandSessionSpec {
    /// Display Provider reference.
    pub provider_ref: ResourceRef,
    /// Guest owner reference.
    pub guest_ref: ResourceRef,
}

impl WaylandSessionSpec {
    /// Construct the exact minimal delegated display spec.
    pub fn new(
        display_provider_ref: Option<ResourceRef>,
        guest_ref: ResourceRef,
    ) -> Result<Self, DisplaySessionError> {
        let provider_ref = display_provider_ref.ok_or(DisplaySessionError::ProviderMissing)?;
        if provider_ref.to_canonical_string() != "Provider/display-wayland"
            || guest_ref.resource_type().as_str() != "Guest"
        {
            return Err(DisplaySessionError::InvalidReference);
        }
        Ok(Self {
            provider_ref,
            guest_ref,
        })
    }

    /// Read the endpoint attachment from a Ready status.
    pub fn endpoint_attachment(
        observation: &DisplayObservation,
    ) -> Result<&DisplayAttachment, DisplaySessionError> {
        if observation.phase != DisplaySessionPhase::Ready {
            return Err(DisplaySessionError::NotReady);
        }
        observation
            .attachment
            .as_ref()
            .ok_or(DisplaySessionError::AttachmentMissing)
    }
}

/// Display dependency failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaySessionError {
    /// The Provider ref was omitted.
    ProviderMissing,
    /// A ref had the wrong type or Provider.
    InvalidReference,
    /// The session is not ready.
    NotReady,
    /// A Ready session had no endpoint attachment.
    AttachmentMissing,
}
