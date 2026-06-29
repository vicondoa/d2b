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
    LoopSuppressed,
    AuditFailure,
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
            Self::LoopSuppressed => "loop_suppressed",
            Self::AuditFailure => "audit_failure",
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

fn normalize_mime(mime: &str) -> String {
    mime.trim()
        .split(';')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(";")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_initial_mimes_only() {
        assert!(is_mime_allowed("text/plain"));
        assert!(is_mime_allowed("Text/Plain ; Charset=UTF-8"));
        assert!(is_mime_allowed("image/png"));
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
    }
}
