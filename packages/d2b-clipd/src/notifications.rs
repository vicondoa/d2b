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
            reason.as_str()
        ),
    }
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
    fn failure_notification_uses_reason_and_realm_labels_only() {
        let notification = user_visible_failure(ReasonCode::PolicyDenied, "Host", "Personal");
        assert!(notification.body.contains("policy_denied"));
        assert!(notification.body.contains("Host"));
        assert!(notification.body.contains("Personal"));
        assert!(!notification.body.contains("secret"));
    }
}
