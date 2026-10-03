use serde::{Deserialize, Serialize};

pub use d2b_provider_clipboard_wayland::{
    ALLOWED_MIME_TYPES, ClipboardEndpointRefusal, SECRET_HINT_MIME_TYPES, normalize_mime,
};
use d2b_contracts_resource::v3::{AdmissionStage, RefusalReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    Allowed,
    MimeRejected,
    PolicyDenied,
    BackgroundProbe,
    IntentMissing,
    PickerNotConfigured,
    PickerBusy,
    PickerCrashed,
    PickerTimeout,
    RequestExpired,
    FdWriteTimeout,
    FdClosed,
    FdCapExceeded,
    BridgeUnavailable,
    SourceMaterializeTimeout,
    MaterializationRateLimited,
    MemoryCapExceeded,
    AuditFailure,
    VirtualKeyboardFailed,
    /// The delivery channel carries no admitted endpoint relationship.
    ///
    /// This is the fail-closed answer for a missing endpoint: the host stops
    /// here instead of reaching for another host channel.
    EndpointAbsent,
    /// The delivery channel's endpoint relationship was revoked or draining.
    EndpointWithdrawn,
    /// The presented endpoint relationship is not the declared one.
    EndpointRefused,
}

impl Serialize for ReasonCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl ReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::MimeRejected => "mime_rejected",
            Self::PolicyDenied => "policy_denied",
            Self::BackgroundProbe => "background_probe",
            Self::IntentMissing => "intent_missing",
            Self::PickerNotConfigured => "picker_not_configured",
            Self::PickerBusy => "picker_busy",
            Self::PickerCrashed => "picker_crashed",
            Self::PickerTimeout => "picker_timeout",
            Self::RequestExpired => "request_expired",
            Self::FdWriteTimeout => "fd_write_timeout",
            Self::FdClosed => "fd_closed",
            Self::FdCapExceeded => "fd_cap_exceeded",
            Self::BridgeUnavailable => "bridge_unavailable",
            Self::SourceMaterializeTimeout => "source_materialize_timeout",
            Self::MaterializationRateLimited => "materialization_rate_limited",
            Self::MemoryCapExceeded => "memory_cap_exceeded",
            Self::AuditFailure => "audit_failure",
            Self::VirtualKeyboardFailed => "virtual_keyboard_failed",
            Self::EndpointAbsent => "endpoint_absent",
            Self::EndpointWithdrawn => "endpoint_withdrawn",
            Self::EndpointRefused => "endpoint_refused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionQuality {
    ExactClient,
    FocusedWindowGuess,
    CacheStaleFocusedWindowGuess,
    BrokerInjectedDebug,
}

pub fn is_mime_allowed(mime: &str) -> bool {
    let normalized = normalize_mime(mime);
    ALLOWED_MIME_TYPES.contains(&normalized.as_str())
}

pub fn has_secret_hint<'a>(mime_names: impl IntoIterator<Item = &'a str>) -> bool {
    mime_names
        .into_iter()
        .map(normalize_mime)
        .any(|mime| SECRET_HINT_MIME_TYPES.contains(&mime.as_str()))
}

/// Classify one clipboard endpoint refusal into the host's closed reason
/// vocabulary.
///
/// The mapping is by the graph's own stage, so an absent relationship, a
/// withdrawn relationship, and a relationship the gate refused for another
/// reason stay distinguishable in the host's audit trail without the refusal
/// ever carrying a Zone, a socket, or payload text.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed when the clipd host delivers over an admitted endpoint (U28/U34)"
    )
)]
pub fn endpoint_refusal_reason(refusal: ClipboardEndpointRefusal) -> ReasonCode {
    match (refusal.stage(), refusal.reason()) {
        (AdmissionStage::Activate, RefusalReason::StaleAuthority) => ReasonCode::EndpointAbsent,
        (AdmissionStage::Revoke, _) | (AdmissionStage::Drain, _) => ReasonCode::EndpointWithdrawn,
        _ => ReasonCode::EndpointRefused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_initial_mimes_only() {
        assert!(is_mime_allowed("text/plain"));
        assert!(is_mime_allowed("Text/Plain ; Charset=UTF-8"));
        assert!(is_mime_allowed("image/png"));
        assert!(!is_mime_allowed("image/png; exploit=1"));
        assert!(!is_mime_allowed("text/plain; exploit=1"));
        assert!(!is_mime_allowed("application/octet-stream"));
        assert!(!is_mime_allowed("text/uri-list"));
    }

    #[test]
    fn secret_hints_are_detected_case_insensitively() {
        assert!(has_secret_hint(["text/plain", "x-kde-passwordManagerHint"]));
        assert!(has_secret_hint(["application/x-secret-service"]));
        assert!(!has_secret_hint(["text/plain", "image/png"]));
    }

    #[test]
    fn reason_codes_are_low_cardinality_json_labels() {
        assert_eq!(
            serde_json::to_string(&ReasonCode::AuditFailure).expect("json"),
            "\"audit_failure\""
        );
        assert_eq!(
            serde_json::to_string(&ReasonCode::FdCapExceeded).expect("json"),
            "\"fd_cap_exceeded\""
        );
        assert_eq!(
            serde_json::to_string(&ReasonCode::VirtualKeyboardFailed).expect("json"),
            "\"virtual_keyboard_failed\""
        );
    }

    #[test]
    fn endpoint_refusals_classify_by_the_graphs_own_stage() {
        use ClipboardEndpointRefusal as Refusal;

        assert_eq!(
            endpoint_refusal_reason(Refusal::relationship_absent()),
            ReasonCode::EndpointAbsent
        );
        assert_eq!(
            endpoint_refusal_reason(Refusal::relationship_revoked()),
            ReasonCode::EndpointWithdrawn
        );
        assert_eq!(
            endpoint_refusal_reason(Refusal::relationship_draining()),
            ReasonCode::EndpointWithdrawn
        );
        assert_eq!(
            endpoint_refusal_reason(Refusal::channel_mismatch()),
            ReasonCode::EndpointRefused
        );
        assert_eq!(
            endpoint_refusal_reason(Refusal::relationship_superseded()),
            ReasonCode::EndpointRefused
        );
    }
}
