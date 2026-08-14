//! Zone-local clipboard service roles and bindings.

/// One closed service Role projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardRole {
    /// Role name.
    pub name: &'static str,
    /// Exact service verbs.
    pub verbs: &'static [&'static str],
}

/// One closed RoleBinding projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardRoleBinding {
    /// Binding name.
    pub name: &'static str,
    /// Exact subject selector.
    pub subject: &'static str,
    /// Role name.
    pub role: &'static str,
}

/// Canonical clipboard RBAC table.
pub struct ClipboardRbac;

impl ClipboardRbac {
    /// Return all service Roles.
    pub const fn roles() -> &'static [ClipboardRole] {
        &[
            ClipboardRole {
                name: "clipboard-admin",
                verbs: &["connect", "invoke", "stream"],
            },
            ClipboardRole {
                name: "clipboard-viewer",
                verbs: &["connect", "stream"],
            },
            ClipboardRole {
                name: "clipboard-bridge-peer",
                verbs: &["connect", "stream"],
            },
            ClipboardRole {
                name: "clipboard-picker-worker",
                verbs: &["connect", "stream"],
            },
        ]
    }

    /// Return all service RoleBindings.
    pub const fn bindings() -> &'static [ClipboardRoleBinding] {
        &[
            ClipboardRoleBinding {
                name: "display-wayland-bridge",
                subject: "Provider/display-wayland",
                role: "clipboard-bridge-peer",
            },
            ClipboardRoleBinding {
                name: "host-admin-clipboard",
                subject: "User/<configured>",
                role: "clipboard-admin",
            },
            ClipboardRoleBinding {
                name: "picker-session-worker",
                subject: "Process/picker-*",
                role: "clipboard-picker-worker",
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::ClipboardRbac;

    #[test]
    fn viewer_cannot_invoke_clipboard_mutations() {
        let viewer = ClipboardRbac::roles()
            .iter()
            .find(|role| role.name == "clipboard-viewer")
            .expect("viewer role");
        assert!(!viewer.verbs.contains(&"invoke"));
        assert!(viewer.verbs.contains(&"connect"));
        assert!(viewer.verbs.contains(&"stream"));
    }
}
