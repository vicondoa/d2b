use serde::{Deserialize, Serialize};

pub const ALLOWED_MIME_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "text/html",
    "image/png",
];

pub const SECRET_HINT_MIME_TYPES: &[&str] = &[
    "x-kde-passwordmanagerhint",
    "application/x-kde-passwordmanagerhint",
    "x-gnome-passwordmanagerhint",
    "application/x-gnome-passwordmanagerhint",
    "x-keepassxc-secret",
    "application/x-keepassxc-secret",
    "application/x-secret-service",
];
pub const MAX_OFFER_MIME_TYPES: usize = 32;
pub const MAX_MIME_TYPE_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPolicy {
    pub max_item_bytes: u64,
    pub max_held_transfers: usize,
}

impl Default for ClipboardPolicy {
    fn default() -> Self {
        Self {
            max_item_bytes: 8 * 1024 * 1024,
            max_held_transfers: 64,
        }
    }
}

impl ClipboardPolicy {
    pub fn new(max_item_bytes: u64, max_held_transfers: usize) -> Result<Self, ReasonCode> {
        if max_item_bytes == 0 || max_held_transfers == 0 {
            return Err(ReasonCode::PolicyDenied);
        }
        Ok(Self {
            max_item_bytes,
            max_held_transfers,
        })
    }

    pub fn validate_offer(&self, byte_count: u64, mime_type: &str) -> Result<(), ReasonCode> {
        if !is_mime_allowed(mime_type) {
            return Err(ReasonCode::MimeRejected);
        }
        if byte_count == 0 || byte_count > self.max_item_bytes {
            return Err(ReasonCode::MemoryCapExceeded);
        }
        Ok(())
    }

    pub fn decide_transfer(&self, request: &TransferRequest<'_>) -> Result<(), ReasonCode> {
        self.validate_offer(request.byte_count, request.mime_type)?;
        if !request.audit_available {
            return Err(ReasonCode::AuditFailure);
        }
        if request.source_realm == request.destination_realm {
            return Ok(());
        }
        if !request.explicit_cross_realm_allow {
            return Err(ReasonCode::PolicyDenied);
        }
        if !request.trusted_paste_intent {
            return Err(ReasonCode::IntentMissing);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferRequest<'a> {
    pub source_realm: &'a str,
    pub destination_realm: &'a str,
    pub mime_type: &'a str,
    pub byte_count: u64,
    pub explicit_cross_realm_allow: bool,
    pub trusted_paste_intent: bool,
    pub audit_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    if mime.len() > MAX_MIME_TYPE_BYTES {
        return false;
    }
    let normalized = normalize_mime(mime);
    ALLOWED_MIME_TYPES.contains(&normalized.as_str())
}

pub fn has_secret_hint<'a>(mime_names: impl IntoIterator<Item = &'a str>) -> bool {
    mime_names.into_iter().any(is_bounded_secret_hint)
}

pub fn normalize_mime(mime: &str) -> String {
    mime.trim()
        .split(';')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(";")
        .to_ascii_lowercase()
}

pub fn is_bounded_secret_hint(mime: &str) -> bool {
    mime.len() <= MAX_MIME_TYPE_BYTES
        && SECRET_HINT_MIME_TYPES.contains(&normalize_mime(mime).as_str())
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
        assert!(!is_bounded_secret_hint(
            &"x".repeat(MAX_MIME_TYPE_BYTES + 1)
        ));
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

    fn request<'a>(source: &'a str, destination: &'a str) -> TransferRequest<'a> {
        TransferRequest {
            source_realm: source,
            destination_realm: destination,
            mime_type: "text/plain",
            byte_count: 12,
            explicit_cross_realm_allow: false,
            trusted_paste_intent: false,
            audit_available: true,
        }
    }

    #[test]
    fn same_realm_transfer_needs_no_cross_realm_grant() {
        let policy = ClipboardPolicy::default();
        assert_eq!(policy.decide_transfer(&request("work", "work")), Ok(()));
    }

    #[test]
    fn cross_realm_transfer_requires_policy_intent_and_audit() {
        let policy = ClipboardPolicy::default();
        let mut transfer = request("work", "personal");
        assert_eq!(
            policy.decide_transfer(&transfer),
            Err(ReasonCode::PolicyDenied)
        );
        transfer.explicit_cross_realm_allow = true;
        assert_eq!(
            policy.decide_transfer(&transfer),
            Err(ReasonCode::IntentMissing)
        );
        transfer.trusted_paste_intent = true;
        assert_eq!(policy.decide_transfer(&transfer), Ok(()));
        transfer.audit_available = false;
        assert_eq!(
            policy.decide_transfer(&transfer),
            Err(ReasonCode::AuditFailure)
        );
    }

    #[test]
    fn offer_caps_are_fail_closed() {
        let policy = ClipboardPolicy::new(16, 1).unwrap();
        assert_eq!(
            policy.validate_offer(17, "text/plain"),
            Err(ReasonCode::MemoryCapExceeded)
        );
        assert_eq!(
            policy.validate_offer(4, "application/octet-stream"),
            Err(ReasonCode::MimeRejected)
        );
    }
}
