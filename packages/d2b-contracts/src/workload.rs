//! Workload summaries and lifecycle state (ADR 0032). A workload is a VM,
//! provider session, or sandbox addressed by a stable id/alias.

use crate::capability::CapabilitySet;
use crate::ids::{NodeId, WorkloadId};
use crate::realm::RealmPath;
use crate::token::ProtocolToken;
use serde::{Deserialize, Serialize};

/// Runtime provider family exposed to provider-neutral clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadProviderKind {
    /// Locally supervised NixOS VM.
    LocalVm,
    /// Locally supervised external-media QEMU runtime.
    QemuMedia,
    /// Runtime owned by a provider adapter.
    ProviderManaged,
    /// Host-user runtime with no isolation boundary.
    UnsafeLocal,
}

/// Security boundary presented by a workload runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationPosture {
    /// Hardware-virtualized VM boundary.
    VirtualMachine,
    /// Isolation is owned and described by a provider adapter.
    ProviderManaged,
    /// No isolation boundary; the process runs as the host user.
    UnsafeLocal,
}

/// Environment source used for workload execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentPosture {
    /// Environment is owned by the runtime/provider.
    RuntimeManaged,
    /// Environment is copied from the systemd user manager.
    SystemdUserManagerAmbient,
}

/// Graphical display routing applied to a workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayEnvironmentPosture {
    /// Display routing is owned by the runtime/provider.
    RuntimeManaged,
    /// `DISPLAY` is removed and `WAYLAND_DISPLAY` points at the d2b proxy.
    WaylandProxyOnly,
    /// The workload has no graphical display route.
    NotApplicable,
}

/// OS identity under which a workload executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionIdentityPosture {
    /// Runtime-configured workload user.
    WorkloadUser,
    /// Execution identity is owned by the provider.
    ProviderManaged,
    /// Exact authenticated local requester uid.
    AuthenticatedRequesterUid,
}

/// Lifetime authority for persistent workload sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SessionPersistencePosture {
    /// Session lifetime is owned by the runtime/provider.
    RuntimeManaged,
    /// Session survives helper/daemon reconnects but not user-manager exit.
    UserManagerLifetime,
}

/// Typed execution posture consumed by CLI and desktop clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadExecutionPosture {
    pub isolation: IsolationPosture,
    pub environment: EnvironmentPosture,
    pub display_environment: DisplayEnvironmentPosture,
    pub execution_identity: ExecutionIdentityPosture,
    pub session_persistence: SessionPersistencePosture,
}

/// Provider-neutral launcher item kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LauncherItemKind {
    /// Execute provider-private configured argv.
    Exec,
    /// Open or attach to a persistent shell.
    Shell,
}

/// Presentation-only icon metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LauncherIcon {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Public, argv-free launcher item metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LauncherItemSummary {
    pub id: ProtocolToken,
    pub name: String,
    #[serde(default)]
    pub icon: LauncherIcon,
    #[serde(rename = "type")]
    pub kind: LauncherItemKind,
    #[serde(default)]
    pub graphical: bool,
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

/// Coarse workload lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadState {
    /// Declared/known but not running.
    Stopped,
    /// Allocation/start in progress.
    Starting,
    /// Running.
    Running,
    /// Stop in progress.
    Stopping,
    /// Terminal failure.
    Failed,
}

/// A selector for listing workloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadSelector {
    /// All workloads on the node.
    All,
    /// A single workload by id.
    One(WorkloadId),
}

/// A workload's advertised summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSummary {
    /// Stable operator-facing alias/id.
    pub id: WorkloadId,
    /// Realm this workload belongs to. Inventory is realm-scoped; this does
    /// not imply a host-global workload registry.
    pub realm: RealmPath,
    /// Node that owns this workload.
    pub node: NodeId,
    /// Current state.
    pub state: WorkloadState,
    /// Capabilities this workload can present.
    pub capabilities: CapabilitySet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::ids::RealmId;

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    #[test]
    fn workload_summary_carries_realm_node_and_capabilities() {
        let summary = WorkloadSummary {
            id: WorkloadId::parse("demo").unwrap(),
            realm: realm("work"),
            node: NodeId::parse("gateway").unwrap(),
            state: WorkloadState::Running,
            capabilities: CapabilitySet::empty().with(Capability::Exec),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"realm\":[\"work\"]"));
        let back: WorkloadSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realm.target_form(), "work");
        assert_eq!(back.node.as_str(), "gateway");
        assert!(back.capabilities.has(Capability::Exec));
    }

    #[test]
    fn workload_summary_rejects_unknown_fields() {
        let json = "{\"id\":\"demo\",\"realm\":[\"work\"],\"node\":\"gateway\",\
                    \"state\":\"running\",\"capabilities\":[],\"unexpected\":\"redacted\"}";
        assert!(serde_json::from_str::<WorkloadSummary>(json).is_err());
    }

    #[test]
    fn unsafe_local_posture_round_trips_as_closed_typed_fields() {
        let posture = WorkloadExecutionPosture {
            isolation: IsolationPosture::UnsafeLocal,
            environment: EnvironmentPosture::SystemdUserManagerAmbient,
            display_environment: DisplayEnvironmentPosture::WaylandProxyOnly,
            execution_identity: ExecutionIdentityPosture::AuthenticatedRequesterUid,
            session_persistence: SessionPersistencePosture::UserManagerLifetime,
        };
        let json = serde_json::to_string(&posture).unwrap();
        assert!(json.contains("\"isolation\":\"unsafe-local\""));
        assert!(json.contains("\"environment\":\"systemd-user-manager-ambient\""));
        assert!(json.contains("\"displayEnvironment\":\"wayland-proxy-only\""));
        assert!(json.contains("\"executionIdentity\":\"authenticated-requester-uid\""));
        assert!(json.contains("\"sessionPersistence\":\"user-manager-lifetime\""));
        assert_eq!(
            serde_json::from_str::<WorkloadExecutionPosture>(&json).unwrap(),
            posture
        );
        assert!(
            serde_json::from_str::<WorkloadExecutionPosture>(
                r#"{"isolation":"unsafe-local","environment":"systemd-user-manager-ambient","displayEnvironment":"wayland-proxy-only","executionIdentity":"authenticated-requester-uid","sessionPersistence":"user-manager-lifetime","extra":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn downstream_posture_fixture_covers_each_provider_family() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/workload-execution-posture-v1.json"
        ))
        .unwrap();
        assert_eq!(fixture["schemaVersion"], "v1");

        let unsafe_local: WorkloadExecutionPosture =
            serde_json::from_value(fixture["postures"]["unsafeLocal"].clone()).unwrap();
        assert_eq!(unsafe_local.isolation, IsolationPosture::UnsafeLocal);
        assert_eq!(
            unsafe_local.execution_identity,
            ExecutionIdentityPosture::AuthenticatedRequesterUid
        );

        let local_vm: WorkloadExecutionPosture =
            serde_json::from_value(fixture["postures"]["localVm"].clone()).unwrap();
        assert_eq!(local_vm.isolation, IsolationPosture::VirtualMachine);
        assert_eq!(
            local_vm.execution_identity,
            ExecutionIdentityPosture::WorkloadUser
        );

        let provider_managed: WorkloadExecutionPosture =
            serde_json::from_value(fixture["postures"]["providerManaged"].clone()).unwrap();
        assert_eq!(
            provider_managed.isolation,
            IsolationPosture::ProviderManaged
        );
        assert_eq!(
            provider_managed.execution_identity,
            ExecutionIdentityPosture::ProviderManaged
        );
    }

    #[test]
    fn launcher_item_summary_contains_no_runtime_argv() {
        let item = LauncherItemSummary {
            id: ProtocolToken::parse("browser").unwrap(),
            name: "Browser".to_owned(),
            icon: LauncherIcon {
                id: Some("firefox".to_owned()),
                name: Some("web-browser".to_owned()),
            },
            kind: LauncherItemKind::Exec,
            graphical: true,
            capabilities: CapabilitySet::empty().with(Capability::ConfiguredLaunch),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"type\":\"exec\""));
        assert!(!json.contains("argv"));
        assert!(!json.contains("firefox --"));
    }
}
