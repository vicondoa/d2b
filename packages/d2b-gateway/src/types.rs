//! Owned, redacted value types carried through the orchestrator (design §3
//! `api-testability`). `AppCommand` and `DisplaySocket` wrap operator-supplied
//! argv and a socket path; their `Debug` is redacted so a stray
//! `{:?}`/error/trace can never leak the command line or a filesystem path.

use d2b_realm_core::{OperationId, PrincipalId, RealmPath, WorkloadId};

/// Fixed width retained for the opaque display-session capability passed only
/// inside the gateway guest.
pub const SECRET_LEN: usize = 32;

/// Opaque, non-secret display-session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DisplaySessionId(String);

impl DisplaySessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for DisplaySessionId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque gateway-guest capability material. It is never a host-side realm
/// credential and is not itself a wire handshake.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionSecret([u8; SECRET_LEN]);

impl SessionSecret {
    pub fn from_bytes(bytes: [u8; SECRET_LEN]) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }
}

impl core::fmt::Debug for SessionSecret {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SessionSecret(<redacted>)")
    }
}

/// Non-authoritative correlation fields for a gateway display operation.
/// ComponentSession state, not this value, supplies authentication.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    pub realm: String,
    pub generation: u64,
    pub session_id: String,
    pub epoch: u64,
    pub operation_id: String,
    pub principal: String,
    pub workload: String,
    pub not_after: u64,
}

impl SessionBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        realm: &RealmPath,
        generation: u64,
        session_id: &DisplaySessionId,
        epoch: u64,
        operation_id: &OperationId,
        principal: &PrincipalId,
        workload: &WorkloadId,
        not_after: u64,
    ) -> Self {
        Self {
            realm: realm.target_form(),
            generation,
            session_id: session_id.as_str().to_owned(),
            epoch,
            operation_id: operation_id.as_str().to_owned(),
            principal: principal.as_str().to_owned(),
            workload: workload.as_str().to_owned(),
            not_after,
        }
    }
}

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
        // Exact redacted output; no path component may leak.
        assert_eq!(d, "DisplaySocket(<redacted>)");
        assert!(!d.contains("1000") && !d.contains("wpc") && !d.contains("run"));
        assert_eq!(s.path(), "/run/user/1000/wpc.sock");
    }
}
