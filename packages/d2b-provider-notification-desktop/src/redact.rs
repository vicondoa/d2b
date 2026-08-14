//! Content sanitization at the presentation boundary.

use crate::types::{MAX_BODY_CHARS, MAX_SUMMARY_CHARS, NotificationError, NotificationRequest};

/// Sanitized notification content used only by the presentation sink.
#[derive(Clone, PartialEq, Eq)]
pub struct SanitizedNotification {
    summary: String,
    body: String,
    icon_ref: Option<String>,
    urgency: crate::NotificationUrgency,
    category: crate::Category,
    expire_timeout_secs: u32,
    actions: Vec<(String, String)>,
}

impl SanitizedNotification {
    /// Borrow the sanitized summary.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Borrow the sanitized body.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Borrow the closed category.
    pub const fn category(&self) -> crate::Category {
        self.category
    }

    /// Borrow sanitized action IDs and labels.
    pub fn actions(&self) -> &[(String, String)] {
        &self.actions
    }

    /// Borrow the signed icon ID.
    pub fn icon_ref(&self) -> Option<&str> {
        self.icon_ref.as_deref()
    }

    /// Return urgency.
    pub const fn urgency(&self) -> crate::NotificationUrgency {
        self.urgency
    }

    /// Return D-Bus expiry timeout.
    pub const fn expire_timeout_secs(&self) -> u32 {
        self.expire_timeout_secs
    }
}

impl core::fmt::Debug for SanitizedNotification {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SanitizedNotification(<redacted>)")
    }
}

/// Sanitize a request without exposing content to diagnostics.
pub fn sanitize(request: &NotificationRequest) -> Result<SanitizedNotification, NotificationError> {
    let summary = sanitize_text(request.summary(), MAX_SUMMARY_CHARS);
    let body = sanitize_text(request.body().unwrap_or_default(), MAX_BODY_CHARS);
    let actions = request
        .actions()
        .iter()
        .map(|action| (action.id().to_owned(), sanitize_text(action.label(), 64)))
        .collect();
    Ok(SanitizedNotification {
        summary,
        body,
        icon_ref: request.icon_ref().map(str::to_owned),
        urgency: request.urgency(),
        category: request.category(),
        expire_timeout_secs: request.expire_timeout_secs(),
        actions,
    })
}

/// Sanitize arbitrary presentation text.
pub fn sanitize_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .take(max_chars)
        .map(|character| match character {
            '\n' | '\r' | '\t' => ' ',
            character if character.is_control() => '\u{FFFD}',
            character => character,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
