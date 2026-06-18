//! Owned, redacted value types carried through the orchestrator (design §3
//! `api-testability`). `AppCommand` and `DisplaySocket` wrap operator-supplied
//! argv and a socket path; their `Debug` is redacted so a stray
//! `{:?}`/error/trace can never leak the command line or a filesystem path.

use nixling_constellation_core::{OperationId, PrincipalId, RealmPath};

use crate::handshake::DisplaySessionId;

/// The redacted, owned context for a display session. Carries only non-secret
/// identifiers; the session secret never lives here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySessionContext {
    /// Opaque session id (non-secret, safe to log/audit).
    pub session_id: DisplaySessionId,
    /// Authorizing operation id.
    pub operation_id: OperationId,
    /// Realm of the authorizing operation.
    pub realm: RealmPath,
    /// Gateway generation that owns the session.
    pub generation: u64,
    /// Authorizing caller principal.
    pub peer_principal: PrincipalId,
}

/// An operator-supplied Wayland app command (`["foot", "--title=..."]`). The
/// argv is bounded and **Debug-redacted** so it never leaks into a trace,
/// error, or log; only the program name (argv[0]) is exposed for audit.
#[derive(Clone, PartialEq, Eq)]
pub struct AppCommand {
    argv: Vec<String>,
}

impl AppCommand {
    /// Build from argv; the first element is the program.
    pub fn new(argv: Vec<String>) -> Option<Self> {
        if argv.is_empty() || argv.iter().any(|a| a.is_empty()) {
            return None;
        }
        Some(Self { argv })
    }
    /// The program name (argv[0]).
    pub fn program(&self) -> &str {
        &self.argv[0]
    }
    /// Borrow the full argv (for handing to the agent, never for logging).
    pub fn argv(&self) -> &[String] {
        &self.argv
    }
}

impl core::fmt::Debug for AppCommand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Only the program name; arguments are redacted.
        write!(
            f,
            "AppCommand({}, <{} args redacted>)",
            self.argv[0],
            self.argv.len() - 1
        )
    }
}

/// An operator-owned per-session display unix-socket path. **Debug-redacted**
/// (a socket path can disclose the operator uid / runtime dir layout).
#[derive(Clone, PartialEq, Eq)]
pub struct DisplaySocket {
    path: String,
}

impl DisplaySocket {
    /// Wrap a unix-socket path.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
    /// Borrow the path (for connecting, never for logging).
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl core::fmt::Debug for DisplaySocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DisplaySocket(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_command_debug_redacts_args() {
        let c = AppCommand::new(vec!["foot".into(), "--title=secret".into()]).unwrap();
        let d = format!("{c:?}");
        assert!(d.contains("foot"));
        assert!(!d.contains("secret"), "args must be redacted: {d}");
        assert_eq!(c.program(), "foot");
    }

    #[test]
    fn app_command_rejects_empty() {
        assert!(AppCommand::new(vec![]).is_none());
        assert!(AppCommand::new(vec!["".into()]).is_none());
    }

    #[test]
    fn display_socket_debug_redacts_path() {
        let s = DisplaySocket::new("/run/user/1000/wpc.sock");
        let d = format!("{s:?}");
        assert!(!d.contains("1000"), "path must be redacted: {d}");
        assert_eq!(s.path(), "/run/user/1000/wpc.sock");
    }
}
