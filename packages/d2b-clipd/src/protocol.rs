//! Typed clipboard-picker domain values.
//!
//! The generated `d2b.clipboard.picker.v2` messages remain the only public wire
//! DTOs. The ComponentSession composition layer converts admitted requests into
//! these bounded values; this module does not provide a second wire format.

use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_OFFERS_PER_PAGE: usize = 64;
pub const MAX_RETAINED_OFFERS: usize = 256;
pub const MAX_OPAQUE_ID_BYTES: usize = 64;
pub const MAX_MIME_TYPE_BYTES: usize = 96;
pub const MAX_PREVIEW_BYTES: usize = 2 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 256 * 1024;
pub const MAX_APP_LABEL_BYTES: usize = 128;

/// Exact d2b endpoint identity authenticated by a dedicated bridge listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEndpointIdentity {
    pub canonical_target: WorkloadTarget,
    pub provider_kind: WorkloadProviderKind,
    pub legacy_vm_name: Option<String>,
}

impl ClipboardEndpointIdentity {
    pub fn realm_label(&self) -> String {
        self.canonical_target.realm.target_form()
    }

    pub fn target_label(&self) -> String {
        self.canonical_target.to_canonical()
    }

    pub fn provider_label(&self) -> &'static str {
        match self.provider_kind {
            WorkloadProviderKind::LocalVm => "local-vm",
            WorkloadProviderKind::QemuMedia => "qemu-media",
            WorkloadProviderKind::ProviderManaged => "provider-managed",
            WorkloadProviderKind::UnsafeLocal => "unsafe-local",
        }
    }

    pub fn app_id_prefix(&self) -> String {
        format!("d2b.{}.", self.target_label())
    }

    pub fn bridge_component(&self) -> String {
        self.legacy_vm_name.clone().unwrap_or_else(|| {
            let digest = Sha256::digest(self.target_label().as_bytes());
            let mut encoded = String::with_capacity(24);
            for byte in &digest[..12] {
                use std::fmt::Write as _;
                let _ = write!(encoded, "{byte:02x}");
            }
            format!("endpoint-{encoded}")
        })
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpaquePickerId(String);

impl OpaquePickerId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_OPAQUE_ID_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        valid.then_some(Self(value)).ok_or(ProtocolError::InvalidId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for OpaquePickerId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpaquePickerId(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardTarget {
    Host,
    Workload {
        canonical_target: WorkloadTarget,
        provider_kind: WorkloadProviderKind,
    },
}

impl ClipboardTarget {
    pub fn workload(
        canonical_target: &str,
        provider_kind: WorkloadProviderKind,
    ) -> Result<Self, ProtocolError> {
        let canonical_target =
            WorkloadTarget::parse(canonical_target).map_err(|_| ProtocolError::InvalidTarget)?;
        Ok(Self::Workload {
            canonical_target,
            provider_kind,
        })
    }

    pub fn canonical_label(&self) -> String {
        match self {
            Self::Host => "host.local.d2b".to_owned(),
            Self::Workload {
                canonical_target, ..
            } => canonical_target.to_canonical(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionQuality {
    ExactClient,
    FocusedWindowGuess,
    CacheStaleFocusedWindowGuess,
    BrokerInjectedDebug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityPreflight {
    Satisfied,
    Denied,
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PickerOffer {
    offer_id: OpaquePickerId,
    source: ClipboardTarget,
    destination: ClipboardTarget,
    mime_type: String,
    preview: Option<String>,
    thumbnail_png: Option<Vec<u8>>,
    source_application: Option<String>,
    attribution: AttributionQuality,
    capability_preflight: CapabilityPreflight,
    byte_count: Option<u64>,
    observed_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    confirmation_required: bool,
}

impl std::fmt::Debug for PickerOffer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PickerOffer")
            .field("offer_id", &"<redacted>")
            .field("source", &self.source.canonical_label())
            .field("destination", &self.destination.canonical_label())
            .field("mime_type", &self.mime_type)
            .field("has_preview", &self.preview.is_some())
            .field("has_thumbnail", &self.thumbnail_png.is_some())
            .field("attribution", &self.attribution)
            .field("capability_preflight", &self.capability_preflight)
            .field("byte_count", &self.byte_count)
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .field("confirmation_required", &self.confirmation_required)
            .finish()
    }
}

pub struct PickerOfferInput {
    pub offer_id: OpaquePickerId,
    pub source: ClipboardTarget,
    pub destination: ClipboardTarget,
    pub mime_type: String,
    pub preview: Option<String>,
    pub thumbnail_png: Option<Vec<u8>>,
    pub source_application: Option<String>,
    pub attribution: AttributionQuality,
    pub capability_preflight: CapabilityPreflight,
    pub byte_count: Option<u64>,
    pub observed_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub confirmation_required: bool,
}

impl PickerOffer {
    pub fn new(input: PickerOfferInput) -> Result<Self, ProtocolError> {
        if !valid_mime_type(&input.mime_type)
            || input
                .preview
                .as_deref()
                .is_some_and(|value| !valid_preview(value))
            || input
                .thumbnail_png
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > MAX_THUMBNAIL_BYTES)
            || input
                .source_application
                .as_deref()
                .is_some_and(|value| !valid_label(value))
            || input.observed_at_unix_ms == 0
            || input.expires_at_unix_ms <= input.observed_at_unix_ms
        {
            return Err(ProtocolError::InvalidOffer);
        }
        Ok(Self {
            offer_id: input.offer_id,
            source: input.source,
            destination: input.destination,
            mime_type: input.mime_type,
            preview: input.preview,
            thumbnail_png: input.thumbnail_png,
            source_application: input.source_application,
            attribution: input.attribution,
            capability_preflight: input.capability_preflight,
            byte_count: input.byte_count,
            observed_at_unix_ms: input.observed_at_unix_ms,
            expires_at_unix_ms: input.expires_at_unix_ms,
            confirmation_required: input.confirmation_required,
        })
    }

    pub fn offer_id(&self) -> &OpaquePickerId {
        &self.offer_id
    }

    pub fn source(&self) -> &ClipboardTarget {
        &self.source
    }

    pub fn destination(&self) -> &ClipboardTarget {
        &self.destination
    }

    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub fn preview(&self) -> Option<&str> {
        self.preview.as_deref()
    }

    pub fn thumbnail_png(&self) -> Option<&[u8]> {
        self.thumbnail_png.as_deref()
    }

    pub fn source_application(&self) -> Option<&str> {
        self.source_application.as_deref()
    }

    pub const fn attribution(&self) -> AttributionQuality {
        self.attribution
    }

    pub const fn capability_preflight(&self) -> CapabilityPreflight {
        self.capability_preflight
    }

    pub const fn byte_count(&self) -> Option<u64> {
        self.byte_count
    }

    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    pub const fn confirmation_required(&self) -> bool {
        self.confirmation_required
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferQuery {
    pub destination: ClipboardTarget,
    pub page_size: usize,
}

impl OfferQuery {
    pub fn new(destination: ClipboardTarget, page_size: usize) -> Result<Self, ProtocolError> {
        if page_size == 0 || page_size > MAX_OFFERS_PER_PAGE {
            return Err(ProtocolError::InvalidPageSize);
        }
        Ok(Self {
            destination,
            page_size,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferSelection {
    pub selection_id: OpaquePickerId,
    pub offer_id: OpaquePickerId,
    pub destination: ClipboardTarget,
}

impl OfferSelection {
    pub fn new(
        selection_id: &str,
        offer_id: &str,
        destination: ClipboardTarget,
    ) -> Result<Self, ProtocolError> {
        Ok(Self {
            selection_id: OpaquePickerId::parse(selection_id)?,
            offer_id: OpaquePickerId::parse(offer_id)?,
            destination,
        })
    }
}

fn valid_mime_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MIME_TYPE_BYTES
        && value.is_ascii()
        && value.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'+' | b'.'))
}

fn valid_preview(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PREVIEW_BYTES
        && value
            .chars()
            .all(|character| !character.is_control() || matches!(character, '\n' | '\t'))
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_APP_LABEL_BYTES
        && value
            .chars()
            .all(|character| !character.is_control() && character != '\u{7f}')
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("clipboard-picker-id-invalid")]
    InvalidId,
    #[error("clipboard-picker-target-invalid")]
    InvalidTarget,
    #[error("clipboard-picker-offer-invalid")]
    InvalidOffer,
    #[error("clipboard-picker-page-size-invalid")]
    InvalidPageSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(value: &str) -> ClipboardTarget {
        ClipboardTarget::workload(value, WorkloadProviderKind::LocalVm).unwrap()
    }

    fn offer() -> PickerOffer {
        PickerOffer::new(PickerOfferInput {
            offer_id: OpaquePickerId::parse("offer-1").unwrap(),
            source: ClipboardTarget::Host,
            destination: target("browser.personal.d2b"),
            mime_type: "text/plain".to_owned(),
            preview: Some("bounded preview".to_owned()),
            thumbnail_png: None,
            source_application: Some("Editor".to_owned()),
            attribution: AttributionQuality::ExactClient,
            capability_preflight: CapabilityPreflight::Satisfied,
            byte_count: Some(15),
            observed_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
            confirmation_required: true,
        })
        .unwrap()
    }

    #[test]
    fn endpoint_component_is_stable_and_provider_neutral() {
        let endpoint = ClipboardEndpointIdentity {
            canonical_target: WorkloadTarget::parse("tools.host.d2b").unwrap(),
            provider_kind: WorkloadProviderKind::UnsafeLocal,
            legacy_vm_name: None,
        };
        assert_eq!(
            endpoint.bridge_component(),
            "endpoint-fc002cd9909aab17c2232e85"
        );
    }

    #[test]
    fn canonical_targets_are_parsed_not_split() {
        assert!(ClipboardTarget::workload("browser", WorkloadProviderKind::LocalVm).is_err());
        assert_eq!(
            target("browser.personal.d2b").canonical_label(),
            "browser.personal.d2b"
        );
        assert_eq!(ClipboardTarget::Host.canonical_label(), "host.local.d2b");
    }

    #[test]
    fn offer_metadata_and_selections_are_bounded() {
        let offer = offer();
        assert_eq!(offer.offer_id().as_str(), "offer-1");
        assert_eq!(offer.preview(), Some("bounded preview"));
        assert!(OpaquePickerId::parse("x".repeat(MAX_OPAQUE_ID_BYTES + 1)).is_err());
        assert!(OfferQuery::new(target("browser.personal.d2b"), 0).is_err());
        assert!(OfferQuery::new(target("browser.personal.d2b"), MAX_OFFERS_PER_PAGE + 1).is_err());
    }

    #[test]
    fn malformed_display_metadata_fails_closed() {
        let invalid = PickerOffer::new(PickerOfferInput {
            offer_id: OpaquePickerId::parse("offer-1").unwrap(),
            source: ClipboardTarget::Host,
            destination: target("browser.personal.d2b"),
            mime_type: "text/plain".to_owned(),
            preview: Some("escape\u{1b}[31m".to_owned()),
            thumbnail_png: None,
            source_application: None,
            attribution: AttributionQuality::FocusedWindowGuess,
            capability_preflight: CapabilityPreflight::Unknown,
            byte_count: None,
            observed_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
            confirmation_required: false,
        });
        assert_eq!(invalid.unwrap_err(), ProtocolError::InvalidOffer);
    }
}
