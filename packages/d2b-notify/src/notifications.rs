// SPDX-License-Identifier: Apache-2.0
//! Desktop notification structs, pluggable `Notifier` trait, and per-event
//! notification builders for security-key ceremonies.
//!
//! Design mirrors `d2b-clipd/src/notifications.rs` but adds:
//! - Optional [`NotificationAction`] list for Freedesktop action buttons.
//! - Per-action nonce tokens (from [`crate::nonce`]) embedded in the action
//!   key so the daemon can validate callbacks fail-closed.
//!
//! The `Notifier` trait is kept pluggable so callers can inject a
//! `RecordingNotifier` in tests and a `DesktopNotifier` (backed by
//! `notify-rust` or direct D-Bus) in production.

use crate::events::{BlockReason, BusyDetail, SecurityKeyEvent};
use crate::nonce::action_key_for;

/// An action button on a desktop notification.
///
/// The `action_key` encodes the nonce in the `d2b-sk-<verb>:<hex>` format
/// produced by [`crate::nonce::action_key_for`].  The notification daemon
/// passes this string back on `ActionInvoked`; the callback looks it up with
/// [`crate::nonce::ActionNonceStore::validate_and_consume`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationAction {
    /// Full Freedesktop action key including the embedded nonce.
    pub action_key: String,
    /// Human-readable label shown on the button.
    pub label: String,
}

impl NotificationAction {
    /// Build a `Cancel request` action for the given pre-minted nonce.
    pub fn cancel(nonce: &str) -> Self {
        Self {
            action_key: action_key_for("cancel", nonce),
            label: "Cancel request".to_owned(),
        }
    }

    /// Build an `Open status` action for the given pre-minted nonce.
    pub fn open_status(nonce: &str) -> Self {
        Self {
            action_key: action_key_for("open-status", nonce),
            label: "Open status".to_owned(),
        }
    }
}

/// A ready-to-emit desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// One-line summary shown as the notification title.
    pub summary: String,
    /// Optional longer body text.
    pub body: String,
    /// Freedesktop action buttons.  Empty when no actions are available or
    /// when the caller chose not to populate them.
    pub actions: Vec<NotificationAction>,
}

/// Pluggable notification sink.
///
/// Two implementations are provided here:
/// - [`DesktopNotifier`]: logs to stderr (for test builds) or calls the real
///   OS notification API.  Production callers that link `notify-rust` wire
///   this to D-Bus; minimal callers that do not want a D-Bus dependency can
///   shell out to `notify-send`.
/// - [`RecordingNotifier`]: collects emissions for hermetic tests.
pub trait Notifier {
    fn notify(&mut self, notification: Notification);
}

/// A `Notifier` implementation that records every emitted notification.
/// Used in tests to assert notification content and action payloads without
/// touching the real D-Bus session.
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    pub notifications: Vec<Notification>,
}

impl Notifier for RecordingNotifier {
    fn notify(&mut self, notification: Notification) {
        self.notifications.push(notification);
    }
}

/// Sanitize user-controlled text before including it in a notification body.
///
/// Strips control characters, collapses whitespace, and caps length.  This
/// prevents notification injection via a malicious VM name or RP ID.
pub fn sanitize(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Per-event notification builders
// ---------------------------------------------------------------------------

/// Build the notification for a [`SecurityKeyEvent::Started`] event.
pub fn started(
    vm_name: &str,
    rp_id: Option<&str>,
    actions: Vec<NotificationAction>,
) -> Notification {
    let vm = sanitize(vm_name, 64);
    let body = match rp_id {
        Some(rp) => format!(
            "{vm} is authenticating with a security key on {}.",
            sanitize(rp, 128)
        ),
        None => format!("{vm} is authenticating with a security key."),
    };
    Notification {
        summary: "Security key request".to_owned(),
        body,
        actions,
    }
}

/// Build the notification for a [`SecurityKeyEvent::TouchNeeded`] event.
pub fn touch_needed(vm_name: &str, actions: Vec<NotificationAction>) -> Notification {
    let vm = sanitize(vm_name, 64);
    Notification {
        summary: "Touch your security key".to_owned(),
        body: format!("{vm} is waiting for a physical touch on the security key."),
        actions,
    }
}

/// Build the notification for a [`SecurityKeyEvent::Busy`] event.
pub fn busy(vm_name: &str, detail: &BusyDetail, actions: Vec<NotificationAction>) -> Notification {
    let vm = sanitize(vm_name, 64);
    let holder = sanitize(&detail.holder_vm, 64);
    Notification {
        summary: "Security key busy".to_owned(),
        body: format!("{vm} is waiting — {holder} is currently using the security key."),
        actions,
    }
}

/// Build the notification for a [`SecurityKeyEvent::TimedOut`] event.
pub fn timed_out(vm_name: &str, actions: Vec<NotificationAction>) -> Notification {
    let vm = sanitize(vm_name, 64);
    Notification {
        summary: "Security key request timed out".to_owned(),
        body: format!("{vm} did not receive a security key response in time."),
        actions,
    }
}

/// Build the notification for a [`SecurityKeyEvent::Failed`] event.
pub fn failed(vm_name: &str, reason: &str, actions: Vec<NotificationAction>) -> Notification {
    let vm = sanitize(vm_name, 64);
    let r = sanitize(reason, 128);
    Notification {
        summary: "Security key request failed".to_owned(),
        body: format!("{vm}: {r}"),
        actions,
    }
}

/// Build the notification for a [`SecurityKeyEvent::Canceled`] event.
pub fn canceled(vm_name: &str) -> Notification {
    let vm = sanitize(vm_name, 64);
    Notification {
        summary: "Security key request canceled".to_owned(),
        body: format!("The security key request from {vm} was canceled."),
        actions: vec![],
    }
}

/// Build the notification for a [`SecurityKeyEvent::Blocked`] event.
pub fn blocked(
    vm_name: &str,
    reason: &BlockReason,
    actions: Vec<NotificationAction>,
) -> Notification {
    let vm = sanitize(vm_name, 64);
    let reason_text = match reason {
        BlockReason::KeyNotPresent => "the security key is not present",
        BlockReason::PolicyDenied => "policy denied access to the security key",
        BlockReason::VmNotOptedIn => "the VM has not opted into security-key proxy",
        BlockReason::BrokerError => "an internal broker error prevented the request",
    };
    Notification {
        summary: "Security key request blocked".to_owned(),
        body: format!("{vm}: {reason_text}."),
        actions,
    }
}

/// Dispatch a [`SecurityKeyEvent`] to the `Notifier` using the appropriate
/// builder, attaching the provided `actions`.
///
/// [`SecurityKeyEvent::Queued`] and [`SecurityKeyEvent::Completed`] are
/// intentionally not surfaced as desktop notifications: queuing is internal
/// bookkeeping and completion is the silent success case.
pub fn emit_for_event<N: Notifier>(
    notifier: &mut N,
    event: &SecurityKeyEvent,
    actions: Vec<NotificationAction>,
) {
    if !event.is_user_visible() {
        return;
    }
    let notification = match event {
        SecurityKeyEvent::Started { vm_name, rp_id, .. } => {
            started(vm_name, rp_id.as_deref(), actions)
        }
        SecurityKeyEvent::TouchNeeded { vm_name, .. } => touch_needed(vm_name, actions),
        SecurityKeyEvent::Busy {
            vm_name, detail, ..
        } => busy(vm_name, detail, actions),
        SecurityKeyEvent::TimedOut { vm_name, .. } => timed_out(vm_name, actions),
        SecurityKeyEvent::Failed {
            vm_name, reason, ..
        } => failed(vm_name, reason, actions),
        SecurityKeyEvent::Canceled { vm_name, .. } => canceled(vm_name),
        SecurityKeyEvent::Blocked {
            vm_name, reason, ..
        } => blocked(vm_name, reason, actions),
        // Queued and Completed are handled by is_user_visible() returning false above.
        SecurityKeyEvent::Queued { .. } | SecurityKeyEvent::Completed { .. } => return,
    };
    notifier.notify(notification);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockReason, BusyDetail, SecurityKeyEvent};

    fn no_actions() -> Vec<NotificationAction> {
        vec![]
    }

    #[test]
    fn started_summary_is_stable() {
        let n = started("personal-dev", None, no_actions());
        assert_eq!(n.summary, "Security key request");
        assert!(n.body.contains("personal-dev"));
    }

    #[test]
    fn started_with_rp_id_includes_it() {
        let n = started("work-aad", Some("github.com"), no_actions());
        assert!(n.body.contains("github.com"), "rp_id must appear in body");
        assert!(!n.body.contains('\n'));
    }

    #[test]
    fn touch_needed_summary_is_stable() {
        let n = touch_needed("work-aad", no_actions());
        assert_eq!(n.summary, "Touch your security key");
        assert!(n.body.contains("work-aad"));
    }

    #[test]
    fn busy_mentions_holder_vm() {
        let detail = BusyDetail {
            holder_vm: "personal-dev".to_owned(),
            waiting_vms: vec![],
        };
        let n = busy("work-aad", &detail, no_actions());
        assert!(n.body.contains("personal-dev"));
        assert!(n.body.contains("work-aad"));
    }

    #[test]
    fn sanitize_strips_newlines_and_controls() {
        let out = sanitize("vm\nname\r\n", 80);
        assert!(!out.contains('\n'));
        assert!(!out.contains('\r'));
    }

    #[test]
    fn sanitize_truncates_at_max_chars() {
        let long = "a".repeat(200);
        let out = sanitize(&long, 80);
        assert!(out.len() <= 80);
    }

    #[test]
    fn cancel_action_has_expected_label() {
        let a = NotificationAction::cancel("aabbccdd");
        assert_eq!(a.label, "Cancel request");
        assert!(a.action_key.starts_with("d2b-sk-cancel:"));
    }

    #[test]
    fn open_status_action_has_expected_label() {
        let a = NotificationAction::open_status("aabbccdd");
        assert_eq!(a.label, "Open status");
        assert!(a.action_key.starts_with("d2b-sk-open-status:"));
    }

    #[test]
    fn emit_for_event_skips_non_user_visible() {
        let mut rec = RecordingNotifier::default();
        let completed = SecurityKeyEvent::Completed {
            session_id: "s1".to_owned(),
            vm_name: "vm1".to_owned(),
        };
        emit_for_event(&mut rec, &completed, no_actions());
        assert!(
            rec.notifications.is_empty(),
            "Completed must not emit a notification"
        );

        let queued = SecurityKeyEvent::Queued {
            session_id: "s1".to_owned(),
            vm_name: "vm1".to_owned(),
            queue_position: 1,
        };
        emit_for_event(&mut rec, &queued, no_actions());
        assert!(
            rec.notifications.is_empty(),
            "Queued must not emit a notification"
        );
    }

    #[test]
    fn emit_for_event_dispatches_all_user_visible() {
        let events: Vec<SecurityKeyEvent> = vec![
            SecurityKeyEvent::Started {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
                rp_id: None,
            },
            SecurityKeyEvent::TouchNeeded {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::Busy {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
                detail: BusyDetail {
                    holder_vm: "other".to_owned(),
                    waiting_vms: vec![],
                },
            },
            SecurityKeyEvent::TimedOut {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::Failed {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
                reason: "test".to_owned(),
            },
            SecurityKeyEvent::Canceled {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::Blocked {
                session_id: "s".to_owned(),
                vm_name: "v".to_owned(),
                reason: BlockReason::PolicyDenied,
            },
        ];
        let mut rec = RecordingNotifier::default();
        for event in &events {
            emit_for_event(&mut rec, event, no_actions());
        }
        assert_eq!(rec.notifications.len(), events.len());
    }

    #[test]
    fn no_notification_body_contains_raw_newline() {
        let events: Vec<SecurityKeyEvent> = vec![
            SecurityKeyEvent::Started {
                session_id: "s".to_owned(),
                vm_name: "vm\nwith\nnewlines".to_owned(),
                rp_id: Some("rp\nid".to_owned()),
            },
            SecurityKeyEvent::Busy {
                session_id: "s".to_owned(),
                vm_name: "vm".to_owned(),
                detail: BusyDetail {
                    holder_vm: "other\nholder".to_owned(),
                    waiting_vms: vec![],
                },
            },
        ];
        let mut rec = RecordingNotifier::default();
        for event in &events {
            emit_for_event(&mut rec, event, no_actions());
        }
        for n in &rec.notifications {
            assert!(
                !n.body.contains('\n'),
                "notification body must not contain newline"
            );
            assert!(
                !n.summary.contains('\n'),
                "notification summary must not contain newline"
            );
        }
    }
}
