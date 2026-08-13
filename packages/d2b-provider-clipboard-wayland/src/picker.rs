//! Metadata-only picker session protocol.

/// Picker request validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerError {
    /// A metadata field exceeded its bound.
    Bounds,
    /// A MIME value is not in the closed allowlist.
    MimeRejected,
}

impl core::fmt::Display for PickerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Bounds => "picker-request-bounds",
            Self::MimeRejected => "mime-rejected",
        })
    }
}

impl std::error::Error for PickerError {}

/// Picker metadata sent over ComponentSession.
#[derive(Clone, PartialEq, Eq)]
pub struct PickerRequest {
    operation_id: String,
    source_zone: String,
    destination_guest: String,
    mime_types: Vec<String>,
}

impl PickerRequest {
    /// Validate and construct a metadata-only request.
    pub fn new(
        operation_id: impl Into<String>,
        source_zone: impl Into<String>,
        destination_guest: impl Into<String>,
        mime_types: Vec<String>,
    ) -> Result<Self, PickerError> {
        let operation_id = operation_id.into();
        let source_zone = source_zone.into();
        let destination_guest = destination_guest.into();
        if operation_id.is_empty()
            || operation_id.len() > 128
            || source_zone.is_empty()
            || source_zone.len() > 63
            || destination_guest.is_empty()
            || destination_guest.len() > 63
            || mime_types.len() > 16
        {
            return Err(PickerError::Bounds);
        }
        if mime_types
            .iter()
            .any(|mime| !crate::Policy::default().allows_mime(mime))
        {
            return Err(PickerError::MimeRejected);
        }
        Ok(Self {
            operation_id,
            source_zone,
            destination_guest,
            mime_types,
        })
    }
}

impl core::fmt::Debug for PickerRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PickerRequest(<redacted>)")
    }
}

/// One picker terminal result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerResult {
    /// A selected item, represented only by an opaque digest.
    Selected(String),
    /// User cancelled.
    Cancelled,
    /// Runtime deadline elapsed.
    TimedOut,
    /// Worker failed to start or complete.
    Failed,
}
