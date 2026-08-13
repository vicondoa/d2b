//! Validated Wayland resource specifications.

use d2b_contracts::v3::ResourceRef;
use serde::{Deserialize, Serialize};

use crate::policy::FilterInput;

const MAX_LABEL_BYTES: usize = 64;
const POLICY_RESOURCE_TYPE: &str = "display-wayland.d2bus.org.WaylandPolicy";

/// Errors raised while validating a Wayland resource specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaylandSpecError {
    /// A ResourceRef had the wrong closed ResourceType.
    InvalidReference,
    /// A display label did not match the closed identifier grammar.
    InvalidLabel,
    /// A color was not a six-digit RGB value.
    InvalidColor,
    /// A label or title exceeded its bound.
    LabelTooLong,
    /// A cross-domain session was not explicitly trusted.
    CrossDomainUntrusted,
    /// A border width exceeded the compositor-safe bound.
    BorderTooWide,
    /// A policy named an interface outside the compiled catalog.
    UnknownInterface,
    /// The pre-provisioned principal pool has no free account.
    NoPrincipalAvailable,
}

impl core::fmt::Display for WaylandSpecError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidReference => "wayland-reference-invalid",
            Self::InvalidLabel => "wayland-label-invalid",
            Self::InvalidColor => "wayland-color-invalid",
            Self::LabelTooLong => "wayland-label-too-long",
            Self::CrossDomainUntrusted => "cross-domain-not-trusted",
            Self::BorderTooWide => "wayland-border-too-wide",
            Self::UnknownInterface => "unknown-interface-rejected",
            Self::NoPrincipalAvailable => "no-principal-available",
        })
    }
}

impl std::error::Error for WaylandSpecError {}

/// Position of the optional identity label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayLabelPosition {
    /// Draw the label at the upper left.
    TopLeft,
    /// Draw the label centered at the top.
    TopCenter,
}

/// Compositor-agnostic display identity metadata.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayIdentity {
    label: String,
    active_color: String,
    inactive_color: String,
    urgent_color: String,
    border_enabled: bool,
    border_width: u32,
    label_enabled: bool,
    label_text: Option<String>,
    label_position: DisplayLabelPosition,
}

impl DisplayIdentity {
    /// Validate a display identity with default border and label settings.
    pub fn new(
        label: impl Into<String>,
        active_color: impl Into<String>,
        inactive_color: impl Into<String>,
        urgent_color: impl Into<String>,
    ) -> Result<Self, WaylandSpecError> {
        let label = label.into();
        let active_color = active_color.into();
        let inactive_color = inactive_color.into();
        let urgent_color = urgent_color.into();
        validate_label(&label)?;
        validate_color(&active_color)?;
        validate_color(&inactive_color)?;
        validate_color(&urgent_color)?;
        Ok(Self {
            label,
            active_color,
            inactive_color,
            urgent_color,
            border_enabled: true,
            border_width: 9,
            label_enabled: true,
            label_text: None,
            label_position: DisplayLabelPosition::TopLeft,
        })
    }

    /// Set the identity rail width.
    pub fn with_border(mut self, enabled: bool, width: u32) -> Result<Self, WaylandSpecError> {
        if width > 64 {
            return Err(WaylandSpecError::BorderTooWide);
        }
        self.border_enabled = enabled;
        self.border_width = width;
        Ok(self)
    }

    /// Set the optional presentation label.
    pub fn with_label(
        mut self,
        enabled: bool,
        text: Option<String>,
    ) -> Result<Self, WaylandSpecError> {
        if let Some(text) = &text
            && text.len() > MAX_LABEL_BYTES
        {
            return Err(WaylandSpecError::LabelTooLong);
        }
        self.label_enabled = enabled;
        self.label_text = text;
        Ok(self)
    }

    /// Set the presentation label position.
    pub fn with_label_position(mut self, position: DisplayLabelPosition) -> Self {
        self.label_position = position;
        self
    }

    /// Borrow the authenticated display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Borrow the active color.
    pub fn active_color(&self) -> &str {
        &self.active_color
    }

    /// Borrow the inactive color.
    pub fn inactive_color(&self) -> &str {
        &self.inactive_color
    }

    /// Borrow the urgent color.
    pub fn urgent_color(&self) -> &str {
        &self.urgent_color
    }

    /// Return the configured rail width.
    pub const fn border_width(&self) -> u32 {
        self.border_width
    }

    /// Whether the rail is enabled.
    pub const fn border_enabled(&self) -> bool {
        self.border_enabled
    }

    /// Whether the label is enabled.
    pub const fn label_enabled(&self) -> bool {
        self.label_enabled
    }

    /// Borrow the optional configured label text.
    pub fn label_text(&self) -> Option<&str> {
        self.label_text.as_deref()
    }

    /// Return the label position.
    pub const fn label_position(&self) -> DisplayLabelPosition {
        self.label_position
    }
}

impl core::fmt::Debug for DisplayIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DisplayIdentity(<redacted>)")
    }
}

/// Authenticated desired state for one Wayland display session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaylandSessionSpec {
    guest_ref: ResourceRef,
    host_ref: ResourceRef,
    user_ref: ResourceRef,
    policy_ref: ResourceRef,
    identity: DisplayIdentity,
    cross_domain_trusted: bool,
    virgl_video: bool,
    filter: FilterInput,
}

impl WaylandSessionSpec {
    /// Validate and construct a trusted cross-domain session.
    pub fn new(
        guest_ref: ResourceRef,
        host_ref: ResourceRef,
        user_ref: ResourceRef,
        policy_ref: ResourceRef,
        identity: DisplayIdentity,
        cross_domain_trusted: bool,
    ) -> Result<Self, WaylandSpecError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || host_ref.resource_type().as_str() != "Host"
            || user_ref.resource_type().as_str() != "User"
            || policy_ref.resource_type().as_str() != POLICY_RESOURCE_TYPE
        {
            return Err(WaylandSpecError::InvalidReference);
        }
        if !cross_domain_trusted {
            return Err(WaylandSpecError::CrossDomainUntrusted);
        }
        Ok(Self {
            guest_ref,
            host_ref,
            user_ref,
            policy_ref,
            identity,
            cross_domain_trusted,
            virgl_video: false,
            filter: FilterInput::default(),
        })
    }

    /// Enable the experimental virgl video path.
    pub fn with_virgl_video(mut self, enabled: bool) -> Self {
        self.virgl_video = enabled;
        self
    }

    /// Replace the session filter overrides.
    pub fn with_filter(mut self, filter: FilterInput) -> Self {
        self.filter = filter;
        self
    }

    /// Borrow the Guest reference.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the Host reference.
    pub const fn host_ref(&self) -> &ResourceRef {
        &self.host_ref
    }

    /// Borrow the user reference.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    /// Borrow the policy reference.
    pub const fn policy_ref(&self) -> &ResourceRef {
        &self.policy_ref
    }

    /// Borrow identity metadata.
    pub const fn identity(&self) -> &DisplayIdentity {
        &self.identity
    }

    /// Whether the cross-domain opt-in is present.
    pub const fn cross_domain_trusted(&self) -> bool {
        self.cross_domain_trusted
    }

    /// Whether the experimental virgl video path is requested.
    pub const fn virgl_video(&self) -> bool {
        self.virgl_video
    }

    /// Borrow session-specific filter overrides.
    pub const fn filter(&self) -> &FilterInput {
        &self.filter
    }
}

impl core::fmt::Debug for WaylandSessionSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("WaylandSessionSpec(<redacted>)")
    }
}

fn validate_label(value: &str) -> Result<(), WaylandSpecError> {
    if value.is_empty() || value.len() > MAX_LABEL_BYTES {
        return Err(if value.is_empty() {
            WaylandSpecError::InvalidLabel
        } else {
            WaylandSpecError::LabelTooLong
        });
    }
    let mut chars = value.chars();
    if !chars
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        || !chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(WaylandSpecError::InvalidLabel);
    }
    Ok(())
}

fn validate_color(value: &str) -> Result<(), WaylandSpecError> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(WaylandSpecError::InvalidColor);
    }
    Ok(())
}
