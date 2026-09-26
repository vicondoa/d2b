use serde::{Deserialize, Serialize};

pub mod wire {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct RequestEnvelope {
        pub request: BrokerRequest,
        #[serde(default)]
        pub caller_role: CallerRole,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "request", rename_all = "PascalCase", deny_unknown_fields)]
    pub enum BootstrapCall {
        Hello {
            client_version: String,
            #[serde(default)]
            supported_features: Vec<String>,
        },
        ExportBrokerAudit {
            #[serde(default)]
            since: Option<String>,
            #[serde(default)]
            filter: Option<String>,
        },
        ApplyNftables {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        ApplyNmUnmanaged {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        ApplyRoute {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        ApplySysctl {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        CreateOrReconcileUsersGroups {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        CreatePersistentTap {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        CreateTapFd {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        DelegateCgroupV2 {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        InjectSecretById {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        LaunchMinijailChild {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        ModprobeIfAllowed {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenCgroupDir {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenDevice {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenFuse {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenKvm {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenPidfd {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        OpenVhostNet {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        PrepareRuntimeDir {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        PrepareStateDir {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        /// Store-sync bootstrap probe stub. Real dispatch lives in the
        /// production runtime; the bootstrap brokerage returns a typed
        /// `Unimplemented` target_wave envelope.
        StoreSync {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        ReadSecretById {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        RotateSecretById {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        SetBridgePortFlags {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        SpawnRunner {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UpdateHostsFile {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipBind {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipBindFirewallRule {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipExplicitBind {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipExplicitFirewallRule {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipProxyReconcile {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
        UsbipUnbind {
            #[serde(default)]
            opaque_target_id: Option<String>,
        },
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
    #[serde(tag = "role", rename_all = "PascalCase", deny_unknown_fields)]
    pub enum CallerRole {
        AdminUid {
            uid: u32,
        },
        LauncherUid {
            uid: u32,
        },
        RootUid {
            uid: u32,
        },
        HostShutdownUid {
            uid: u32,
        },
        #[default]
        NotAuthorized,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "response", rename_all = "PascalCase", deny_unknown_fields)]
    pub enum BrokerResponse {
        HelloOk {
            server_version: String,
            selected_version: String,
            capabilities: Vec<String>,
        },
        ExportBrokerAuditOk {
            lines: Vec<String>,
        },
        Error {
            kind: String,
            operation: String,
            #[serde(default)]
            target_wave: Option<String>,
            message: String,
            remediation: String,
        },
    }

    pub type BrokerRequest = BootstrapCall;

    impl BootstrapCall {
        pub fn op_name(&self) -> &'static str {
            match self {
                Self::Hello { .. } => "Hello",
                Self::ExportBrokerAudit { .. } => "ExportBrokerAudit",
                Self::ApplyNftables { .. } => "ApplyNftables",
                Self::ApplyNmUnmanaged { .. } => "ApplyNmUnmanaged",
                Self::ApplyRoute { .. } => "ApplyRoute",
                Self::ApplySysctl { .. } => "ApplySysctl",
                Self::CreateOrReconcileUsersGroups { .. } => "CreateOrReconcileUsersGroups",
                Self::CreatePersistentTap { .. } => "CreatePersistentTap",
                Self::CreateTapFd { .. } => "CreateTapFd",
                Self::DelegateCgroupV2 { .. } => "DelegateCgroupV2",
                Self::InjectSecretById { .. } => "InjectSecretById",
                Self::LaunchMinijailChild { .. } => "LaunchMinijailChild",
                Self::ModprobeIfAllowed { .. } => "ModprobeIfAllowed",
                Self::OpenCgroupDir { .. } => "OpenCgroupDir",
                Self::OpenDevice { .. } => "OpenDevice",
                Self::OpenFuse { .. } => "OpenFuse",
                Self::OpenKvm { .. } => "OpenKvm",
                Self::OpenPidfd { .. } => "OpenPidfd",
                Self::OpenVhostNet { .. } => "OpenVhostNet",
                Self::PrepareRuntimeDir { .. } => "PrepareRuntimeDir",
                Self::PrepareStateDir { .. } => "PrepareStateDir",
                Self::StoreSync { .. } => "StoreSync",
                Self::ReadSecretById { .. } => "ReadSecretById",
                Self::RotateSecretById { .. } => "RotateSecretById",
                Self::SetBridgePortFlags { .. } => "SetBridgePortFlags",
                Self::SpawnRunner { .. } => "SpawnRunner",
                Self::UpdateHostsFile { .. } => "UpdateHostsFile",
                Self::UsbipBind { .. } => "UsbipBind",
                Self::UsbipBindFirewallRule { .. } => "UsbipBindFirewallRule",
                Self::UsbipExplicitBind { .. } => "UsbipExplicitBind",
                Self::UsbipExplicitFirewallRule { .. } => "UsbipExplicitFirewallRule",
                Self::UsbipProxyReconcile { .. } => "UsbipProxyReconcile",
                Self::UsbipUnbind { .. } => "UsbipUnbind",
            }
        }

        pub fn opaque_target_id(&self) -> &'static str {
            match self {
                Self::Hello { .. } => "daemon-handshake",
                Self::ExportBrokerAudit { .. } => "audit-log",
                _ => "operation",
            }
        }
    }

    pub fn probe_hello() -> RequestEnvelope {
        RequestEnvelope {
            request: BrokerRequest::Hello {
                client_version: "0.0.0-test".to_owned(),
                supported_features: vec!["layer1-bootstrap".to_owned()],
            },
            caller_role: CallerRole::NotAuthorized,
        }
    }

    pub fn probe_stub(operation: &str) -> Option<RequestEnvelope> {
        let request = match operation {
            "ApplyNftables" => BrokerRequest::ApplyNftables {
                opaque_target_id: None,
            },
            "ApplyNmUnmanaged" => BrokerRequest::ApplyNmUnmanaged {
                opaque_target_id: None,
            },
            "ApplyRoute" => BrokerRequest::ApplyRoute {
                opaque_target_id: None,
            },
            "ApplySysctl" => BrokerRequest::ApplySysctl {
                opaque_target_id: None,
            },
            "CreateOrReconcileUsersGroups" => BrokerRequest::CreateOrReconcileUsersGroups {
                opaque_target_id: None,
            },
            "CreatePersistentTap" => BrokerRequest::CreatePersistentTap {
                opaque_target_id: None,
            },
            "CreateTapFd" => BrokerRequest::CreateTapFd {
                opaque_target_id: None,
            },
            "DelegateCgroupV2" => BrokerRequest::DelegateCgroupV2 {
                opaque_target_id: None,
            },
            "InjectSecretById" => BrokerRequest::InjectSecretById {
                opaque_target_id: None,
            },
            "LaunchMinijailChild" => BrokerRequest::LaunchMinijailChild {
                opaque_target_id: None,
            },
            "ModprobeIfAllowed" => BrokerRequest::ModprobeIfAllowed {
                opaque_target_id: None,
            },
            "OpenCgroupDir" => BrokerRequest::OpenCgroupDir {
                opaque_target_id: None,
            },
            "OpenDevice" => BrokerRequest::OpenDevice {
                opaque_target_id: None,
            },
            "OpenFuse" => BrokerRequest::OpenFuse {
                opaque_target_id: None,
            },
            "OpenKvm" => BrokerRequest::OpenKvm {
                opaque_target_id: None,
            },
            "OpenPidfd" => BrokerRequest::OpenPidfd {
                opaque_target_id: None,
            },
            "OpenVhostNet" => BrokerRequest::OpenVhostNet {
                opaque_target_id: None,
            },
            "PrepareRuntimeDir" => BrokerRequest::PrepareRuntimeDir {
                opaque_target_id: None,
            },
            "PrepareStateDir" => BrokerRequest::PrepareStateDir {
                opaque_target_id: None,
            },
            "StoreSync" => BrokerRequest::StoreSync {
                opaque_target_id: None,
            },
            "ReadSecretById" => BrokerRequest::ReadSecretById {
                opaque_target_id: None,
            },
            "RotateSecretById" => BrokerRequest::RotateSecretById {
                opaque_target_id: None,
            },
            "SetBridgePortFlags" => BrokerRequest::SetBridgePortFlags {
                opaque_target_id: None,
            },
            "SpawnRunner" => BrokerRequest::SpawnRunner {
                opaque_target_id: None,
            },
            "UpdateHostsFile" => BrokerRequest::UpdateHostsFile {
                opaque_target_id: None,
            },
            "UsbipBind" => BrokerRequest::UsbipBind {
                opaque_target_id: None,
            },
            "UsbipBindFirewallRule" => BrokerRequest::UsbipBindFirewallRule {
                opaque_target_id: None,
            },
            "UsbipExplicitBind" => BrokerRequest::UsbipExplicitBind {
                opaque_target_id: None,
            },
            "UsbipExplicitFirewallRule" => BrokerRequest::UsbipExplicitFirewallRule {
                opaque_target_id: None,
            },
            "UsbipProxyReconcile" => BrokerRequest::UsbipProxyReconcile {
                opaque_target_id: None,
            },
            "UsbipUnbind" => BrokerRequest::UsbipUnbind {
                opaque_target_id: None,
            },
            _ => return None,
        };
        Some(RequestEnvelope {
            request,
            caller_role: CallerRole::NotAuthorized,
        })
    }

    pub fn probe_export_audit(caller_role: CallerRole) -> RequestEnvelope {
        RequestEnvelope {
            request: BrokerRequest::ExportBrokerAudit {
                since: None,
                filter: None,
            },
            caller_role,
        }
    }

    pub fn caller_role_from_cli(value: &str) -> Option<CallerRole> {
        if value == "not-authorized" {
            return Some(CallerRole::NotAuthorized);
        }
        if let Some(uid) = value.strip_prefix("admin:") {
            return uid.parse().ok().map(|uid| CallerRole::AdminUid { uid });
        }
        if let Some(uid) = value.strip_prefix("launcher:") {
            return uid.parse().ok().map(|uid| CallerRole::LauncherUid { uid });
        }
        if let Some(uid) = value.strip_prefix("root:") {
            return uid.parse().ok().map(|uid| CallerRole::RootUid { uid });
        }
        if let Some(uid) = value.strip_prefix("host-shutdown:") {
            return uid
                .parse()
                .ok()
                .map(|uid| CallerRole::HostShutdownUid { uid });
        }
        None
    }

    impl CallerRole {
        pub fn is_admin_uid(&self) -> bool {
            matches!(self, Self::AdminUid { .. })
        }
    }
}

impl wire::CallerRole {
    pub fn for_display(&self) -> &'static str {
        match self {
            wire::CallerRole::AdminUid { .. } => "d2b-admin",
            wire::CallerRole::LauncherUid { .. } => "d2b-launcher",
            wire::CallerRole::RootUid { .. } => "d2b-root",
            wire::CallerRole::HostShutdownUid { .. } => "d2b-host-shutdown",
            wire::CallerRole::NotAuthorized => "d2b-not-authorized",
        }
    }
}


