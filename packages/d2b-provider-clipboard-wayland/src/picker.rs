//! Metadata-only picker session protocol and one-use receipts.

use crate::ClipboardHistory;
use crate::service::{
    AuthenticatedClipboardSession, AuthenticatedPasteRoute, entry_owner_for_session,
    operation_id_for_sessions,
};

/// Picker request validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerError {
    /// A metadata field exceeded its bound.
    Bounds,
    /// A MIME value is not in the closed allowlist.
    MimeRejected,
    /// The picker result was not selected or did not match the operation.
    ResultMismatch,
}

impl core::fmt::Display for PickerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Bounds => "picker-request-bounds",
            Self::MimeRejected => "mime-rejected",
            Self::ResultMismatch => "picker-result-mismatch",
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
    /// Bind picker metadata to two authenticated service sessions.
    pub fn from_sessions(
        source: &AuthenticatedClipboardSession,
        destination: &AuthenticatedClipboardSession,
        mime_types: Vec<String>,
    ) -> Result<Self, PickerError> {
        if source.role() != crate::service::ClipboardServiceRole::Picker
            || destination.role() != crate::service::ClipboardServiceRole::Bridge
            || !destination.is_guest()
            || (!source.is_guest() && source.subject_ref().resource_type().as_str() != "User")
        {
            return Err(PickerError::ResultMismatch);
        }
        Self::new(
            operation_id_for_sessions(source, destination),
            source.zone(),
            destination.guest_ref(),
            mime_types,
        )
    }

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
            || mime_types.is_empty()
            || mime_types.len() > 16
        {
            return Err(PickerError::Bounds);
        }
        let mime_types = mime_types
            .iter()
            .map(|mime| crate::policy::normalize_mime(mime))
            .collect::<Vec<_>>();
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

    /// Borrow the operation correlation.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the authenticated source Zone label.
    pub fn source_zone(&self) -> &str {
        &self.source_zone
    }

    /// Borrow the destination Guest reference.
    pub fn destination_guest(&self) -> &str {
        &self.destination_guest
    }

    /// Borrow the requested MIME set.
    pub fn mime_types(&self) -> &[String] {
        &self.mime_types
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

/// A one-use picker receipt bound to one authenticated clipboard operation.
///
/// The receipt is intentionally not cloneable and exposes no constructor.
/// Only [`PickerAuthority::complete`] can mint it, and the clipboard service
/// consumes it before authorizing a paste.
pub struct PickerReceipt {
    operation_id: String,
    source_zone: String,
    destination_zone: String,
    destination_guest: String,
    entry_digest: String,
    entry_owner: String,
    expires_at: u64,
    source_reconnect_generation: u64,
    reconnect_generation: u64,
}

impl PickerReceipt {
    pub(crate) fn issue(
        source: &AuthenticatedClipboardSession,
        destination: &AuthenticatedClipboardSession,
        request: &PickerRequest,
        entry_digest: String,
        expires_at: u64,
    ) -> Result<Self, PickerError> {
        if request.destination_guest() != destination.guest_ref()
            || request.source_zone() != source.zone()
            || request.operation_id() != operation_id_for_sessions(source, destination)
            || !matches!(
                source.subject_ref().resource_type().as_str(),
                "Guest" | "User"
            )
            || !destination.is_guest()
            || !entry_digest.starts_with("sha256:")
        {
            return Err(PickerError::ResultMismatch);
        }
        Ok(Self {
            operation_id: request.operation_id().to_owned(),
            source_zone: source.zone().to_owned(),
            destination_zone: destination.zone().to_owned(),
            destination_guest: destination.guest_ref(),
            entry_digest,
            entry_owner: entry_owner_for_session(source),
            expires_at,
            source_reconnect_generation: source.reconnect_generation(),
            reconnect_generation: destination.reconnect_generation(),
        })
    }

    pub(crate) fn matches(
        &self,
        route: &AuthenticatedPasteRoute,
        entry_digest: &str,
        now_secs: u64,
    ) -> bool {
        self.source_zone == route.source_zone()
            && self.operation_id == route.operation_id()
            && self.source_reconnect_generation == route.source_reconnect_generation()
            && self.destination_zone == route.destination_zone()
            && self.destination_guest == route.destination_guest()
            && self.entry_digest == entry_digest
            && self.expires_at > now_secs
            && self.reconnect_generation == route.reconnect_generation()
    }

    pub(crate) fn source_owner(&self) -> &str {
        &self.entry_owner
    }

    /// Borrow the operation correlation without exposing content.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

impl core::fmt::Debug for PickerReceipt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PickerReceipt(REDACTED)")
    }
}

/// The picker-side completion authority.
pub struct PickerAuthority;

impl PickerAuthority {
    /// Complete a picker request and mint a receipt only for a selected entry.
    pub fn complete(
        source: &AuthenticatedClipboardSession,
        destination: &AuthenticatedClipboardSession,
        request: &PickerRequest,
        result: PickerResult,
        entry_digest: impl Into<String>,
        history: &mut ClipboardHistory,
        now_secs: u64,
    ) -> Result<PickerReceipt, PickerError> {
        let entry_digest = entry_digest.into();
        match result {
            PickerResult::Selected(selected) if selected == entry_digest => {
                let owner = entry_owner_for_session(source);
                if source.is_guest() && history.authorize_guest(&owner).is_err() {
                    return Err(PickerError::ResultMismatch);
                }
                if !history.entry_matches_mime(
                    &entry_digest,
                    &owner,
                    request.mime_types(),
                    now_secs,
                ) {
                    return Err(PickerError::ResultMismatch);
                }
                let Some(expires_at) = history.entry_expiry(&entry_digest, &owner, now_secs) else {
                    return Err(PickerError::ResultMismatch);
                };
                let receipt =
                    PickerReceipt::issue(source, destination, request, entry_digest, expires_at)?;
                let completion_key = format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    request.operation_id(),
                    source.zone(),
                    source.subject_ref().to_canonical_string(),
                    source.reconnect_generation(),
                    destination.zone(),
                    destination.guest_ref(),
                    destination.reconnect_generation(),
                );
                if !history.claim_picker_completion(completion_key, expires_at, now_secs) {
                    return Err(PickerError::ResultMismatch);
                }
                Ok(receipt)
            }
            PickerResult::Cancelled | PickerResult::TimedOut | PickerResult::Failed => {
                Err(PickerError::ResultMismatch)
            }
            PickerResult::Selected(_) => Err(PickerError::ResultMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PickerRequest;

    #[test]
    fn picker_normalizes_mime_metadata_before_matching_history() {
        let request = PickerRequest::new(
            "operation-1",
            "zone-a",
            "Guest/work",
            vec![" TEXT/PLAIN ".to_owned()],
        )
        .unwrap();
        assert_eq!(request.mime_types(), ["text/plain"]);
    }
}
