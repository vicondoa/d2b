//! Zone-local notification stream roles.

/// One notification service Role projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationRole {
    /// Role name.
    pub name: &'static str,
    /// Exact stream or method verbs.
    pub verbs: &'static [&'static str],
}

/// Canonical notification RBAC table.
pub struct NotificationRbac;

impl NotificationRbac {
    /// Return the fixed Role table.
    pub const fn roles() -> &'static [NotificationRole] {
        &[
            NotificationRole {
                name: "notification-desktop-sink-service",
                verbs: &["connect", "open-stream"],
            },
            NotificationRole {
                name: "notification-desktop-source",
                verbs: &["connect", "open-stream"],
            },
            NotificationRole {
                name: "notification-desktop-observer",
                verbs: &["connect", "open-stream", "invoke"],
            },
        ]
    }
}
