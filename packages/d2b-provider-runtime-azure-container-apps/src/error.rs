//! Bounded errors for the ACA gateway effect port.

use std::time::Duration;

use d2b_realm_core::{Capability, ConstellationError, ErrorKind};

const MAX_PROVIDER_FIELD_LEN: usize = 128;
const MAX_PROVIDER_MESSAGE_LEN: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryHint {
    retry_after: Duration,
    applied_backoff: Duration,
}

impl RetryHint {
    pub fn bounded(retry_after: Duration, jitter: Duration, max: Duration) -> Self {
        let applied_backoff = retry_after.saturating_add(jitter).min(max);
        Self {
            retry_after: retry_after.min(max),
            applied_backoff,
        }
    }

    pub fn retry_after(self) -> Duration {
        self.retry_after
    }

    pub fn applied_backoff(self) -> Duration {
        self.applied_backoff
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostic {
    code: Option<String>,
    message: Option<String>,
    correlation_id: Option<String>,
}

impl ProviderDiagnostic {
    pub fn new(
        code: Option<impl Into<String>>,
        message: Option<impl Into<String>>,
        correlation_id: Option<impl Into<String>>,
    ) -> Self {
        Self {
            code: code.map(|value| allowlisted_code(value.into())),
            message: message
                .map(|value| bounded_message(value.into()))
                .filter(|value| !value.is_empty()),
            correlation_id: correlation_id
                .map(|value| bounded_field(value.into()))
                .filter(|value| !value.is_empty()),
        }
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    error: Box<ConstellationError>,
    retry_hint: Option<RetryHint>,
    diagnostic: Option<Box<ProviderDiagnostic>>,
}

impl ProviderError {
    pub fn capability_denied(capability: Capability) -> Self {
        Self::from(ConstellationError::capability_denied(capability))
    }

    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::from(ConstellationError::new(kind, message))
    }

    pub fn rate_limited(message: impl Into<String>, retry_hint: RetryHint) -> Self {
        Self::new(ErrorKind::Backpressure, message).with_retry_hint(retry_hint)
    }

    pub fn with_retry_hint(mut self, retry_hint: RetryHint) -> Self {
        self.retry_hint = Some(retry_hint);
        self
    }

    pub fn with_diagnostic(mut self, diagnostic: ProviderDiagnostic) -> Self {
        self.diagnostic = Some(Box::new(diagnostic));
        self
    }

    pub fn retry_hint(&self) -> Option<RetryHint> {
        self.retry_hint
    }

    pub fn diagnostic(&self) -> Option<&ProviderDiagnostic> {
        self.diagnostic.as_deref()
    }

    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    pub fn message(&self) -> &str {
        self.error.message()
    }

    pub fn missing_capability(&self) -> Option<Capability> {
        self.error.missing_capability()
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)?;
        if let Some(diagnostic) = &self.diagnostic {
            if let Some(code) = diagnostic.code() {
                write!(formatter, " provider_code={code}")?;
            }
            if let Some(message) = diagnostic.message() {
                write!(formatter, " provider_message={message}")?;
            }
            if let Some(correlation_id) = diagnostic.correlation_id() {
                write!(formatter, " correlation_id={correlation_id}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ProviderError {}

impl From<ConstellationError> for ProviderError {
    fn from(error: ConstellationError) -> Self {
        Self {
            error: Box::new(error),
            retry_hint: None,
            diagnostic: None,
        }
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

fn bounded_field(raw: String) -> String {
    raw.chars()
        .filter(|character| {
            character.is_ascii_graphic()
                && !matches!(character, '"' | '\'' | '\\' | '/' | ':')
        })
        .take(MAX_PROVIDER_FIELD_LEN)
        .collect()
}

fn allowlisted_code(raw: String) -> String {
    match bounded_field(raw).as_str() {
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

fn bounded_message(raw: String) -> String {
    if contains_sensitive_shape(&raw) {
        return "provider message redacted".to_owned();
    }
    let mut message: String = raw
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_PROVIDER_MESSAGE_LEN)
        .collect();
    if raw.chars().count() > MAX_PROVIDER_MESSAGE_LEN {
        message.push_str("...");
    }
    message
}

fn contains_sensitive_shape(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("/subscriptions/")
        || raw.contains('{')
        || raw.contains('}')
        || raw
            .split(|character: char| !(character.is_ascii_hexdigit() || character == '-'))
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
                    .all(|part| part.chars().all(|character| character.is_ascii_hexdigit()))
    )
}
