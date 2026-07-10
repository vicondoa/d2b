//! Private unsafe-local helper protocol.
//!
//! The authenticated Unix peer credential is the execution identity. No frame
//! carries a uid, environment, cwd, compositor path, or arbitrary public argv.

use crate::public_wire::ShellName;
use d2b_core::{configured_argv::ConfiguredArgv, workload_identity::WorkloadIdentity};
use d2b_realm_core::{ids::OperationId, token::ProtocolToken};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION: u32 = 1;
pub const UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION: u32 = 1;
/// Every terminal-ready frame carries exactly one connected Unix stream fd.
pub const UNSAFE_LOCAL_TERMINAL_FD_COUNT: usize = 1;
pub const MAX_HELPER_FRAME_SIZE: usize = 256 * 1024;
pub const MAX_HELPER_QUEUE_DEPTH: usize = 128;
pub const MAX_HELPER_SNAPSHOT_SCOPES: usize = 1024;
pub const MAX_COMPLETED_OPERATIONS_PER_UID: usize = 1024;
pub const MAX_COMPLETED_OPERATION_AGE_SECS: u64 = 24 * 60 * 60;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHello {
    pub protocol_version: u32,
    pub generation: u64,
    #[serde(default)]
    pub features: Vec<ProtocolToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHelloAccepted {
    pub protocol_version: u32,
    pub generation: u64,
    pub heartbeat_interval_secs: u32,
    pub operation_timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHeartbeat {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeKind {
    LauncherApp,
    WaylandProxy,
    PersistentShell,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeIdentity {
    pub invocation_id: String,
    pub kind: HelperScopeKind,
}

impl fmt::Debug for ScopeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeIdentity")
            .field("invocation_id", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeState {
    Starting,
    Active,
    Stopping,
    Exited,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperScopeSnapshot {
    pub operation_id: OperationId,
    pub workload: WorkloadIdentity,
    pub scope: ScopeIdentity,
    pub state: HelperScopeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperSnapshot {
    pub generation: u64,
    pub scopes: Vec<HelperScopeSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperLaunchRequest {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub workload: WorkloadIdentity,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "args", rename_all = "camelCase")]
pub enum HelperShellRequest {
    List {
        request_id: u64,
        workload: WorkloadIdentity,
    },
    Attach {
        request_id: u64,
        operation_id: OperationId,
        workload: WorkloadIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<ShellName>,
        tty: bool,
    },
    Detach {
        request_id: u64,
        operation_id: OperationId,
        workload: WorkloadIdentity,
        name: ShellName,
    },
    Kill {
        request_id: u64,
        operation_id: OperationId,
        workload: WorkloadIdentity,
        name: ShellName,
    },
}

impl fmt::Debug for HelperShellRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List {
                request_id,
                workload,
            } => f
                .debug_struct("HelperShellRequest::List")
                .field("request_id", request_id)
                .field("workload", workload)
                .finish(),
            Self::Attach {
                request_id,
                operation_id,
                workload,
                name,
                tty,
            } => f
                .debug_struct("HelperShellRequest::Attach")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &name.as_ref().map(|_| "<redacted>"))
                .field("tty", tty)
                .finish(),
            Self::Detach {
                request_id,
                operation_id,
                workload,
                ..
            } => f
                .debug_struct("HelperShellRequest::Detach")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &"<redacted>")
                .finish(),
            Self::Kill {
                request_id,
                operation_id,
                workload,
                ..
            } => f
                .debug_struct("HelperShellRequest::Kill")
                .field("request_id", request_id)
                .field("operation_id", operation_id)
                .field("workload", workload)
                .field("name", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperFailureCode {
    InvalidRequest,
    OperationIdConflict,
    QueueFull,
    Timeout,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
    GraphicalSessionInactive,
    WaylandUnavailable,
    ProxyUnavailable,
    FirstClientTimeout,
    ShellUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperOperationDisposition {
    Committed,
    AlreadyCommitted,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationResult {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub disposition: HelperOperationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperTerminalReady {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub terminal_protocol_version: u32,
    pub transport: HelperTerminalTransport,
    pub scope: ScopeIdentity,
}

/// Transport represented by the single fd attached to a terminal-ready frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperTerminalTransport {
    /// A connected `AF_UNIX` `SOCK_STREAM`; listeners and datagram sockets are invalid.
    ConnectedUnixStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationRejected {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub code: HelperFailureCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum DaemonToUnsafeLocalHelper {
    HelloAccepted(HelperHelloAccepted),
    Heartbeat(HelperHeartbeat),
    Launch(HelperLaunchRequest),
    Shell(HelperShellRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum UnsafeLocalHelperToDaemon {
    Hello(HelperHello),
    Snapshot(HelperSnapshot),
    Heartbeat(HelperHeartbeat),
    Operation(HelperOperationResult),
    TerminalReady(HelperTerminalReady),
    Rejected(HelperOperationRejected),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalHelperWireSchema {
    pub protocol_version: u32,
    pub terminal_protocol_version: u32,
    pub daemon_to_helper: DaemonToUnsafeLocalHelper,
    pub helper_to_daemon: UnsafeLocalHelperToDaemon,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_bounded_and_debug_redacted() {
        let canary = "private-canary-argv";
        let argv = ConfiguredArgv::new(vec!["firefox".to_owned(), canary.to_owned()]).unwrap();
        let debug = format!("{argv:?}");
        assert!(!debug.contains(canary));
        assert!(!debug.contains("firefox"));
        assert!(debug.contains("argc"));
        assert!(ConfiguredArgv::new(Vec::new()).is_err());
        assert!(ConfiguredArgv::new(vec!["x\0y".to_owned()]).is_err());
    }

    #[test]
    fn helper_frames_reject_uid_environment_and_cwd_fields() {
        let frame = r#"{
          "type":"hello",
          "payload":{"protocolVersion":1,"generation":1,"features":[],"uid":1000}
        }"#;
        assert!(serde_json::from_str::<UnsafeLocalHelperToDaemon>(frame).is_err());

        let launch = r#"{
          "requestId":1,
          "operationId":"op-1",
          "workload":{
            "workloadId":"tools",
            "realmId":"host",
            "realmPath":["host"],
            "canonicalTarget":"tools.host.d2b"
          },
          "itemId":"browser",
          "argv":["firefox"],
          "graphical":true,
          "cwd":"/tmp"
        }"#;
        assert!(serde_json::from_str::<HelperLaunchRequest>(launch).is_err());
    }

    #[test]
    fn scope_identity_debug_hides_invocation_id() {
        let canary = "private-canary-invocation";
        let scope = ScopeIdentity {
            invocation_id: canary.to_owned(),
            kind: HelperScopeKind::LauncherApp,
        };
        assert!(!format!("{scope:?}").contains(canary));
    }

    #[test]
    fn shell_request_debug_hides_shell_name() {
        let canary = "private-shell-name-canary";
        let request = HelperShellRequest::Attach {
            request_id: 1,
            operation_id: OperationId::parse("op-1").unwrap(),
            workload: serde_json::from_value(serde_json::json!({
                "workloadId": "tools",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "tools.host.d2b"
            }))
            .unwrap(),
            name: Some(ShellName::new(canary).unwrap()),
            tty: true,
        };
        assert!(!format!("{request:?}").contains(canary));
    }

    #[test]
    fn terminal_ready_freezes_single_connected_stream_transport() {
        assert_eq!(UNSAFE_LOCAL_TERMINAL_FD_COUNT, 1);
        let ready = HelperTerminalReady {
            request_id: 1,
            operation_id: OperationId::parse("op-1").unwrap(),
            terminal_protocol_version: UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
            transport: HelperTerminalTransport::ConnectedUnixStream,
            scope: ScopeIdentity {
                invocation_id: "opaque".to_owned(),
                kind: HelperScopeKind::PersistentShell,
            },
        };
        let json = serde_json::to_string(&ready).unwrap();
        assert!(json.contains("\"transport\":\"connected-unix-stream\""));
        assert!(
            serde_json::from_str::<HelperTerminalReady>(
                r#"{"requestId":1,"operationId":"op-1","terminalProtocolVersion":1,"transport":"unix-datagram","scope":{"invocationId":"opaque","kind":"persistent-shell"}}"#
            )
            .is_err()
        );
    }
}
