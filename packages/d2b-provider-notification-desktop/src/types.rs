//! Bounded notification stream DTOs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Maximum summary length.
pub const MAX_SUMMARY_CHARS: usize = 256;
/// Maximum body length.
pub const MAX_BODY_CHARS: usize = 2048;
/// Maximum actions per request.
pub const MAX_ACTIONS: usize = 4;
const MAX_ACTION_ID_CHARS: usize = 32;
const MAX_ACTION_LABEL_CHARS: usize = 64;
const MAX_ICON_CHARS: usize = 64;

/// Closed notification category set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// A device became available.
    #[serde(rename = "device.added")]
    DeviceAdded,
    /// A device was removed.
    #[serde(rename = "device.removed")]
    DeviceRemoved,
    /// A device reported an error.
    #[serde(rename = "device.error")]
    DeviceError,
    /// A network connection was established.
    #[serde(rename = "network.connected")]
    NetworkConnected,
    /// A network connection ended.
    #[serde(rename = "network.disconnected")]
    NetworkDisconnected,
    /// A network error occurred.
    #[serde(rename = "network.error")]
    NetworkError,
    /// A presence session came online.
    #[serde(rename = "presence.online")]
    PresenceOnline,
    /// A presence session went offline.
    #[serde(rename = "presence.offline")]
    PresenceOffline,
    /// A security event occurred.
    #[serde(rename = "security.event")]
    SecurityEvent,
    /// A security error occurred.
    #[serde(rename = "security.error")]
    SecurityError,
    /// A transfer completed.
    #[serde(rename = "transfer.complete")]
    TransferComplete,
    /// A transfer failed.
    #[serde(rename = "transfer.error")]
    TransferError,
    /// A transfer was cancelled.
    #[serde(rename = "transfer.cancelled")]
    TransferCancelled,
    /// An update is available.
    #[serde(rename = "update.available")]
    UpdateAvailable,
    /// An update is downloading.
    #[serde(rename = "update.downloading")]
    UpdateDownloading,
    /// An update is ready.
    #[serde(rename = "update.ready")]
    UpdateReady,
    /// An update failed.
    #[serde(rename = "update.error")]
    UpdateError,
    /// Informational system event.
    #[serde(rename = "system.info")]
    SystemInfo,
    /// Warning system event.
    #[serde(rename = "system.warning")]
    SystemWarning,
    /// Error system event.
    #[serde(rename = "system.error")]
    SystemError,
}

impl Category {
    /// Return the stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeviceAdded => "device.added",
            Self::DeviceRemoved => "device.removed",
            Self::DeviceError => "device.error",
            Self::NetworkConnected => "network.connected",
            Self::NetworkDisconnected => "network.disconnected",
            Self::NetworkError => "network.error",
            Self::PresenceOnline => "presence.online",
            Self::PresenceOffline => "presence.offline",
            Self::SecurityEvent => "security.event",
            Self::SecurityError => "security.error",
            Self::TransferComplete => "transfer.complete",
            Self::TransferError => "transfer.error",
            Self::TransferCancelled => "transfer.cancelled",
            Self::UpdateAvailable => "update.available",
            Self::UpdateDownloading => "update.downloading",
            Self::UpdateReady => "update.ready",
            Self::UpdateError => "update.error",
            Self::SystemInfo => "system.info",
            Self::SystemWarning => "system.warning",
            Self::SystemError => "system.error",
        }
    }
}

/// Closed desktop urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationUrgency {
    /// Low-priority notification.
    Low,
    /// Normal-priority notification.
    Normal,
    /// High-priority notification.
    Critical,
}

/// One bounded action descriptor.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSpec {
    id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActionSpecWire {
    id: String,
    label: String,
}

impl TryFrom<ActionSpecWire> for ActionSpec {
    type Error = NotificationError;

    fn try_from(value: ActionSpecWire) -> Result<Self, Self::Error> {
        Self::new(value.id, value.label)
    }
}

impl ActionSpec {
    /// Validate an action ID and presentation label.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Result<Self, NotificationError> {
        let id = id.into();
        let label = label.into();
        if !valid_id(&id)
            || id.chars().count() > MAX_ACTION_ID_CHARS
            || label.chars().count() > MAX_ACTION_LABEL_CHARS
        {
            return Err(NotificationError::FieldBounds);
        }
        Ok(Self { id, label })
    }

    /// Borrow the stable action ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Borrow the presentation label.
    pub fn label(&self) -> &str {
        &self.label
    }
}

impl core::fmt::Debug for ActionSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ActionSpec(<redacted>)")
    }
}

/// Notification DTO validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationError {
    /// One field exceeded its fixed bound.
    FieldBounds,
    /// An icon reference was not a signed catalog ID.
    InvalidIcon,
    /// The action list contained a duplicate ID or too many entries.
    InvalidActions,
    /// A timeout was outside the supported range.
    InvalidTimeout,
    /// A correlation or idempotency key exceeded its bound.
    InvalidOpaqueKey,
    /// The authenticated observer stream is disabled by Provider policy.
    ObserverDisabled,
}

impl core::fmt::Display for NotificationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::FieldBounds => "notification-field-bounds",
            Self::InvalidIcon => "notification-icon-invalid",
            Self::InvalidActions => "notification-actions-invalid",
            Self::InvalidTimeout => "notification-timeout-invalid",
            Self::InvalidOpaqueKey => "notification-opaque-key-invalid",
            Self::ObserverDisabled => "notification-observer-disabled",
        })
    }
}

impl std::error::Error for NotificationError {}

impl<'de> Deserialize<'de> for ActionSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ActionSpecWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// A transient notification request.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationRequest {
    summary: String,
    body: Option<String>,
    icon_ref: Option<String>,
    urgency: NotificationUrgency,
    category: Category,
    expire_timeout_secs: u32,
    actions: Vec<ActionSpec>,
    correlation_id: Option<String>,
    idempotency_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotificationRequestWire {
    summary: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    icon_ref: Option<String>,
    #[serde(default = "default_notification_urgency")]
    urgency: NotificationUrgency,
    category: Category,
    #[serde(default)]
    expire_timeout_secs: u32,
    #[serde(default)]
    actions: Vec<ActionSpec>,
    #[serde(default)]
    correlation_id: Option<String>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

fn default_notification_urgency() -> NotificationUrgency {
    NotificationUrgency::Normal
}

impl TryFrom<NotificationRequestWire> for NotificationRequest {
    type Error = NotificationError;

    fn try_from(value: NotificationRequestWire) -> Result<Self, Self::Error> {
        let mut request = Self::new(
            value.summary,
            value.body.unwrap_or_default(),
            value.category,
        )?
        .with_urgency(value.urgency)?
        .with_expire_timeout(value.expire_timeout_secs)?
        .with_actions(value.actions)?;
        if let Some(icon_ref) = value.icon_ref {
            request = request.with_icon_ref(icon_ref)?;
        }
        if let Some(correlation_id) = value.correlation_id {
            request = request.with_correlation_id(correlation_id)?;
        }
        if let Some(idempotency_key) = value.idempotency_key {
            request = request.with_idempotency_key(idempotency_key)?;
        }
        Ok(request)
    }
}

impl<'de> Deserialize<'de> for NotificationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = NotificationRequestWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl NotificationRequest {
    /// Construct a request with the default normal urgency.
    pub fn new(
        summary: impl Into<String>,
        body: impl Into<String>,
        category: Category,
    ) -> Result<Self, NotificationError> {
        let summary = summary.into();
        let body = body.into();
        if summary.chars().count() > MAX_SUMMARY_CHARS
            || body.chars().count() > MAX_BODY_CHARS
            || summary.is_empty()
        {
            return Err(NotificationError::FieldBounds);
        }
        Ok(Self {
            summary,
            body: (!body.is_empty()).then_some(body),
            icon_ref: None,
            urgency: NotificationUrgency::Normal,
            category,
            expire_timeout_secs: 0,
            actions: Vec::new(),
            correlation_id: None,
            idempotency_key: None,
        })
    }

    /// Set urgency.
    pub fn with_urgency(mut self, urgency: NotificationUrgency) -> Result<Self, NotificationError> {
        self.urgency = urgency;
        Ok(self)
    }

    /// Set a signed icon catalog reference.
    pub fn with_icon_ref(mut self, icon_ref: impl Into<String>) -> Result<Self, NotificationError> {
        let icon_ref = icon_ref.into();
        if !valid_id(&icon_ref) || icon_ref.len() > MAX_ICON_CHARS {
            return Err(NotificationError::InvalidIcon);
        }
        self.icon_ref = Some(icon_ref);
        Ok(self)
    }

    /// Add bounded actions.
    pub fn with_actions(mut self, actions: Vec<ActionSpec>) -> Result<Self, NotificationError> {
        if actions.len() > MAX_ACTIONS {
            return Err(NotificationError::InvalidActions);
        }
        let mut ids = BTreeSet::new();
        if actions.iter().any(|action| !ids.insert(action.id.as_str())) {
            return Err(NotificationError::InvalidActions);
        }
        self.actions = actions;
        Ok(self)
    }

    /// Set the D-Bus notification timeout.
    pub fn with_expire_timeout(mut self, seconds: u32) -> Result<Self, NotificationError> {
        if seconds > 3600 {
            return Err(NotificationError::InvalidTimeout);
        }
        self.expire_timeout_secs = seconds;
        Ok(self)
    }

    /// Set an opaque correlation key.
    pub fn with_correlation_id(
        mut self,
        value: impl Into<String>,
    ) -> Result<Self, NotificationError> {
        let value = value.into();
        validate_opaque_key(&value)?;
        self.correlation_id = Some(value);
        Ok(self)
    }

    /// Set an opaque idempotency key.
    pub fn with_idempotency_key(
        mut self,
        value: impl Into<String>,
    ) -> Result<Self, NotificationError> {
        let value = value.into();
        validate_opaque_key(&value)?;
        self.idempotency_key = Some(value);
        Ok(self)
    }

    /// Sanitize this request for the presentation sink.
    pub fn sanitize(&self) -> Result<crate::SanitizedNotification, NotificationError> {
        crate::sanitize(self)
    }

    /// Borrow the category.
    pub const fn category(&self) -> Category {
        self.category
    }

    pub(crate) fn summary(&self) -> &str {
        &self.summary
    }

    pub(crate) fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    /// Borrow actions.
    pub fn actions(&self) -> &[ActionSpec] {
        &self.actions
    }

    /// Return the configured urgency.
    pub const fn urgency(&self) -> NotificationUrgency {
        self.urgency
    }

    /// Return the configured timeout.
    pub const fn expire_timeout_secs(&self) -> u32 {
        self.expire_timeout_secs
    }

    /// Borrow the optional icon ID.
    pub fn icon_ref(&self) -> Option<&str> {
        self.icon_ref.as_deref()
    }

    /// Borrow the optional idempotency key.
    pub(crate) fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }
}

impl core::fmt::Debug for NotificationRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationRequest(<redacted>)")
    }
}

fn valid_id(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn validate_opaque_key(value: &str) -> Result<(), NotificationError> {
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        Err(NotificationError::InvalidOpaqueKey)
    } else {
        Ok(())
    }
}
