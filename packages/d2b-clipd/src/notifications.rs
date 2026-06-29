use crate::policy::ReasonCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub summary: String,
    pub body: String,
}

pub trait Notifier {
    fn notify(&mut self, notification: Notification);
}

#[derive(Debug, Default)]
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&mut self, notification: Notification) {
        if let Err(error) = notify_rust::Notification::new()
            .appname("d2b-clipd")
            .summary(&notification.summary)
            .body(&notification.body)
            .show()
        {
            log::warn!("d2b-clipd: desktop notification failed: {error}");
        }
    }
}

#[derive(Debug, Default)]
pub struct RecordingNotifier {
    pub notifications: Vec<Notification>,
}

impl Notifier for RecordingNotifier {
    fn notify(&mut self, notification: Notification) {
        self.notifications.push(notification);
    }
}

pub fn fallback_ready(target_label: &str) -> Notification {
    Notification {
        summary: "d2b clipboard ready to paste".to_owned(),
        body: format!(
            "Ready to paste: press Ctrl+V in {}.",
            sanitize_notification_text(target_label, 80)
        ),
    }
}

pub fn emit_fallback_ready<N: Notifier>(notifier: &mut N, target_label: &str) {
    notifier.notify(fallback_ready(target_label));
}

pub fn user_visible_failure(
    reason: ReasonCode,
    source_realm: &str,
    destination_realm: &str,
) -> Notification {
    Notification {
        summary: "d2b clipboard paste blocked".to_owned(),
        body: format!(
            "Paste from {} to {} failed: {}.",
            sanitize_notification_text(source_realm, 48),
            sanitize_notification_text(destination_realm, 48),
            reason_label(reason)
        ),
    }
}

fn reason_label(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::Allowed => "allowed",
        ReasonCode::MimeRejected => "MIME type is not allowed",
        ReasonCode::PolicyDenied => "policy denied the transfer",
        ReasonCode::BackgroundProbe => "request did not match recent paste intent",
        ReasonCode::IntentMissing => "paste intent is missing",
        ReasonCode::PickerNotConfigured => "clipboard picker is not configured",
        ReasonCode::PickerBusy => "clipboard picker is already open",
        ReasonCode::PickerCrashed => "clipboard picker exited unexpectedly",
        ReasonCode::PickerTimeout => "clipboard picker timed out",
        ReasonCode::RequestExpired => "paste request expired",
        ReasonCode::FdWriteTimeout => "paste transfer timed out",
        ReasonCode::FdClosed => "paste target closed the transfer",
        ReasonCode::FdCapExceeded => "too many paste transfers are already pending",
        ReasonCode::BridgeUnavailable => "clipboard bridge is unavailable",
        ReasonCode::SourceMaterializeTimeout => "clipboard source timed out",
        ReasonCode::MaterializationRateLimited => "clipboard source was rate limited",
        ReasonCode::MemoryCapExceeded => "clipboard memory cap was exceeded",
        ReasonCode::LoopSuppressed => "broker feedback loop was suppressed",
        ReasonCode::AuditFailure => "audit queue is unavailable",
    }
}

pub fn emit_user_visible_failure<N: Notifier>(
    notifier: &mut N,
    reason: ReasonCode,
    source_realm: &str,
    destination_realm: &str,
) {
    notifier.notify(user_visible_failure(
        reason,
        source_realm,
        destination_realm,
    ));
}

pub fn sanitize_notification_text(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => out.push('�'),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_ready_is_content_free_and_bounded() {
        let notification = fallback_ready("Personal\nFirefox");
        assert!(notification.summary.contains("ready"));
        assert!(notification.body.contains("Personal Firefox"));
        assert!(!notification.body.contains('\n'));
    }

    #[test]
    fn fallback_ready_is_emitted_through_notifier() {
        let mut notifier = RecordingNotifier::default();
        emit_fallback_ready(&mut notifier, "Personal Firefox");
        assert_eq!(notifier.notifications.len(), 1);
        assert!(notifier.notifications[0].body.contains("Ctrl+V"));
    }

    #[test]
    fn failure_notification_uses_reason_and_realm_labels_only() {
        let notification = user_visible_failure(ReasonCode::PolicyDenied, "Host", "Personal");
        assert!(notification.body.contains("policy denied"));
        assert!(notification.body.contains("Host"));
        assert!(notification.body.contains("Personal"));
        assert!(!notification.body.contains("secret"));
    }

    #[test]
    fn failure_notification_is_emitted_through_notifier() {
        let mut notifier = RecordingNotifier::default();
        emit_user_visible_failure(&mut notifier, ReasonCode::PolicyDenied, "Host", "Personal");
        assert_eq!(notifier.notifications.len(), 1);
        assert!(notifier.notifications[0].body.contains("policy denied"));
    }
}
