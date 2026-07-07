//! Typed provider errors (ADR 0032). A provider that cannot support a
//! feature returns a typed capability denial, never a silent fallback.

use std::time::Duration;

use d2b_realm_core::{Capability, ConstellationError, ErrorKind};

const MAX_PROVIDER_FIELD_LEN: usize = 128;
const MAX_PROVIDER_MESSAGE_LEN: usize = 240;

/// Structured provider retry metadata. This stays provider-layer/internal for
/// now; it is not part of the public constellation wire error schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryHint {
    retry_after: Duration,
    applied_backoff: Duration,
}

impl RetryHint {
    /// Build a bounded retry hint. `jitter` is deterministic input supplied by
    /// the caller/test; this helper does not generate randomness.
    pub fn bounded(retry_after: Duration, jitter: Duration, max: Duration) -> Self {
        let applied = retry_after.saturating_add(jitter).min(max);
        Self {
            retry_after: retry_after.min(max),
            applied_backoff: applied,
        }
    }

    /// Provider-requested delay after bounds.
    pub fn retry_after(&self) -> Duration {
        self.retry_after
    }

    /// Actual delay after jitter and bounds.
    pub fn applied_backoff(&self) -> Duration {
        self.applied_backoff
    }
}

/// Allowlisted provider diagnostic fields. Never store raw response bodies,
/// endpoints, headers, tokens, resource ids, command payloads, or output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    code: Option<String>,
    message: Option<String>,
    correlation_id: Option<String>,
}

impl ProviderDiagnostic {
    /// Build from optional allowlisted provider fields.
    pub fn new(
        code: Option<impl Into<String>>,
        message: Option<impl Into<String>>,
        correlation_id: Option<impl Into<String>>,
    ) -> Self {
        Self {
            code: code.map(|s| allowlisted_provider_code(s.into())),
            message: message
                .map(|s| bound_provider_message(s.into()))
                .filter(|s| !s.trim().is_empty()),
            correlation_id: correlation_id
                .map(|s| bound_provider_field(s.into()))
                .filter(|s| !s.trim().is_empty()),
        }
    }

    /// Provider error code, if allowlisted.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Bounded sanitized provider message, if allowlisted.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Bounded safe correlation/request id, if present.
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }
}

/// A provider-layer error. Wraps the codec-neutral
/// [`ConstellationError`] so providers and the operation layer share one
/// typed-error vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    error: Box<ConstellationError>,
    retry_hint: Option<RetryHint>,
    diagnostic: Option<Box<ProviderDiagnostic>>,
}

impl ProviderError {
    /// A typed capability denial (the provider does not advertise `cap`).
    pub fn capability_denied(cap: Capability) -> Self {
        Self::from(ConstellationError::capability_denied(cap))
    }

    /// A typed "feature/transport mode not implemented in this build"
    /// refusal (never a silent fallback).
    pub fn unsupported(feature: impl Into<String>) -> Self {
        Self::from(ConstellationError::new(
            ErrorKind::UnsupportedFeature,
            feature.into(),
        ))
    }

    /// A generic typed error.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::from(ConstellationError::new(kind, message))
    }

    /// A typed provider rate-limit error with structured retry metadata.
    pub fn rate_limited(message: impl Into<String>, retry_hint: RetryHint) -> Self {
        Self::new(ErrorKind::Backpressure, message).with_retry_hint(retry_hint)
    }

    /// Attach structured retry metadata.
    pub fn with_retry_hint(mut self, retry_hint: RetryHint) -> Self {
        self.retry_hint = Some(retry_hint);
        self
    }

    /// Attach allowlisted provider diagnostic metadata.
    pub fn with_diagnostic(mut self, diagnostic: ProviderDiagnostic) -> Self {
        self.diagnostic = Some(Box::new(diagnostic));
        self
    }

    /// Structured retry metadata, if this error carries any.
    pub fn retry_hint(&self) -> Option<RetryHint> {
        self.retry_hint
    }

    /// Allowlisted provider diagnostics, if any.
    pub fn diagnostic(&self) -> Option<&ProviderDiagnostic> {
        self.diagnostic.as_deref()
    }

    /// The underlying error kind.
    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    /// The bounded, operator-safe underlying constellation error message.
    pub fn message(&self) -> &str {
        self.error.message()
    }

    /// The structured missing capability, if this is a capability denial.
    pub fn missing_capability(&self) -> Option<Capability> {
        self.error.missing_capability()
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.error)?;
        let _ = self.retry_hint;
        if let Some(diagnostic) = &self.diagnostic {
            if let Some(code) = diagnostic.code() {
                write!(f, " provider_code={code}")?;
            }
            if let Some(message) = diagnostic.message() {
                write!(f, " provider_message={message}")?;
            }
            if let Some(correlation_id) = diagnostic.correlation_id() {
                write!(f, " correlation_id={correlation_id}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ProviderError {}

impl From<ConstellationError> for ProviderError {
    fn from(e: ConstellationError) -> Self {
        Self {
            error: Box::new(e),
            retry_hint: None,
            diagnostic: None,
        }
    }
}

/// Provider result alias.
pub type ProviderResult<T> = Result<T, ProviderError>;

fn bound_provider_field(raw: String) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '\\' | '/' | ':'))
        .take(MAX_PROVIDER_FIELD_LEN)
        .collect()
}

fn allowlisted_provider_code(raw: String) -> String {
    let bounded = bound_provider_field(raw);
    match bounded.as_str() {
        code if code.eq_ignore_ascii_case("AuthorizationFailed") => {
            "AuthorizationFailed".to_owned()
        }
        code if code.eq_ignore_ascii_case("RevisionProvisioningFailed") => {
            "RevisionProvisioningFailed".to_owned()
        }
        code if code.eq_ignore_ascii_case("QuotaExceeded") => "QuotaExceeded".to_owned(),
        code if code.eq_ignore_ascii_case("TooManyRequests") => "TooManyRequests".to_owned(),
        code if code.eq_ignore_ascii_case("ContainerAppNotFound") => {
            "ContainerAppNotFound".to_owned()
        }
        _ => "unknown".to_owned(),
    }
}

fn bound_provider_message(raw: String) -> String {
    if contains_sensitive_shape(&raw) {
        return "provider message redacted".to_owned();
    }
    let truncated = raw.chars().count() > MAX_PROVIDER_MESSAGE_LEN;
    let mut out: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_PROVIDER_MESSAGE_LEN)
        .collect();
    if truncated {
        out.push_str("...");
    }
    out
}

fn contains_sensitive_shape(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("/subscriptions/")
        || lower.contains("provider-code=")
        || lower.contains("provider_code=")
        || lower.contains("provider-message=")
        || lower.contains("provider_message=")
        || lower.contains("correlation_id=")
        || raw.contains('{')
        || raw.contains('}')
        || raw
            .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
            .any(looks_like_uuid)
}

fn looks_like_uuid(token: &str) -> bool {
    let mut parts = token.split('-');
    matches!(
        (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next()
        ),
        (Some(a), Some(b), Some(c), Some(d), Some(e), None)
            if a.len() == 8
                && b.len() == 4
                && c.len() == 4
                && d.len() == 4
                && e.len() == 12
                && [a, b, c, d, e]
                    .iter()
                    .all(|part| part.chars().all(|ch| ch.is_ascii_hexdigit()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_hint_bounds_applied_delay() {
        let hint = RetryHint::bounded(
            Duration::from_secs(90),
            Duration::from_secs(30),
            Duration::from_secs(60),
        );
        assert_eq!(hint.retry_after(), Duration::from_secs(60));
        assert_eq!(hint.applied_backoff(), Duration::from_secs(60));

        let jitter_capped = RetryHint::bounded(
            Duration::from_secs(50),
            Duration::from_secs(30),
            Duration::from_secs(60),
        );
        assert_eq!(jitter_capped.retry_after(), Duration::from_secs(50));
        assert_eq!(jitter_capped.applied_backoff(), Duration::from_secs(60));
    }

    #[test]
    fn provider_diagnostic_strips_path_and_endpoint_shapes() {
        let d = ProviderDiagnostic::new(
            Some("AuthorizationFailed"),
            Some(
                [
                    "bad /sub",
                    "scriptions/00000000-0000-",
                    "0000-0000-000000000000 h",
                    "ttps://example.invalid",
                ]
                .concat(),
            ),
            Some("corr/../../secret"),
        );
        assert_eq!(d.code(), Some("AuthorizationFailed"));
        assert_eq!(d.message(), Some("provider message redacted"));
        assert!(!d.message().unwrap().contains('\n'));
        assert!(!d.correlation_id().unwrap().contains('/'));
    }

    #[test]
    fn provider_diagnostic_unknown_code_is_low_cardinality() {
        let d = ProviderDiagnostic::new(
            Some("UnexpectedPerTenantCode"),
            None::<String>,
            None::<String>,
        );
        assert_eq!(d.code(), Some("unknown"));
    }

    #[test]
    fn provider_diagnostic_codes_are_case_stable() {
        let d = ProviderDiagnostic::new(
            Some("authorizationfailed"),
            Some("bounded message"),
            None::<String>,
        );
        assert_eq!(d.code(), Some("AuthorizationFailed"));
        assert_eq!(d.message(), Some("bounded message"));
    }

    #[test]
    fn provider_diagnostic_code_allowlist_is_bounded() {
        let quota = ProviderDiagnostic::new(Some("quotaexceeded"), None::<String>, None::<String>);
        assert_eq!(quota.code(), Some("QuotaExceeded"));

        let too_many =
            ProviderDiagnostic::new(Some("toomanyrequests"), None::<String>, None::<String>);
        assert_eq!(too_many.code(), Some("TooManyRequests"));

        let unknown = ProviderDiagnostic::new(
            Some("TenantSpecificDynamicCode"),
            None::<String>,
            None::<String>,
        );
        assert_eq!(unknown.code(), Some("unknown"));
    }

    #[test]
    fn provider_diagnostic_redacts_json_objects() {
        let message = serde_json::json!({
            "error": "dynamic provider payload",
        })
        .to_string();
        let d = ProviderDiagnostic::new(
            Some("RevisionProvisioningFailed"),
            Some(message),
            None::<String>,
        );
        assert_eq!(d.message(), Some("provider message redacted"));
    }

    #[test]
    fn provider_diagnostic_redacts_internal_diagnostic_details() {
        let d = ProviderDiagnostic::new(
            Some("RevisionProvisioningFailed"),
            Some("provider_message=dynamic evidence"),
            None::<String>,
        );
        assert_eq!(d.message(), Some("provider message redacted"));
    }

    #[test]
    fn provider_message_marks_truncation() {
        let raw = "a".repeat(MAX_PROVIDER_MESSAGE_LEN + 1);
        let d = ProviderDiagnostic::new(None::<String>, Some(raw), None::<String>);
        assert!(d.message().unwrap().ends_with("..."));
    }
}
