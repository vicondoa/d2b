use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Serialize};

/// Authorization matrix artifact for public API and private broker operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivilegesJson {
    /// Schema version used by this artifact.
    pub schema_version: String,
    /// Public CLI/API authorization rows.
    pub public_operations: Vec<OperationAuthz>,
    /// Private broker authorization rows.
    pub broker_operations: Vec<OperationAuthz>,
}

/// One explicit authorization row; unknown future operations always deny.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationAuthz {
    /// Stable operation enum or command name.
    #[schemars(schema_with = "operation_schema")]
    pub operation: String,
    /// VM, env, host, key, bundle, daemon, or global subject.
    pub subject: String,
    /// Per-VM, per-env, per-role, per-busid, scoped, or global resource scope.
    pub scope: String,
    /// Groups allowed to invoke the operation; empty denies by default.
    pub allowed_groups: Vec<String>,
    /// Whether state mutation, teardown, rollback, GC, or live routing changes are possible.
    pub destructive: bool,
    /// Whether secret or key material can be read or modified.
    pub secret_access: SecretAccess,
    /// Whether the private broker is required or conditionally used.
    pub broker_required: BrokerRequirement,
    /// Audit event requirement and retained fields.
    pub audit: AuditPolicy,
    /// Default policy for unknown/future operations.
    pub default_for_unknown: DefaultForUnknown,
}

/// Secret exposure class for an authorization row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SecretAccess {
    None,
    PublicKeyOnly,
    RedactedOnly,
    MetadataOnly,
    PossiblePathsOnly,
    HostKeyMetadata,
    ReadWrite,
}

/// Broker-use class for an authorization row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BrokerRequirement {
    No,
    NoMutation,
    Conditional,
    Yes,
}

/// Audit requirement for an authorization row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditPolicy {
    /// Whether a successful operation must emit an audit event.
    pub required: bool,
    /// Whether deny-only or error-only auditing is sufficient.
    pub mode: AuditMode,
    /// Field names retained in the audit event.
    pub retained_fields: Vec<String>,
}

/// Audit mode for compact policy rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditMode {
    DenyOnly,
    Errors,
    Yes,
}

/// Required default for any unknown operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultForUnknown {
    DenyAndAudit,
}

/// Const-friendly authorization row used as the build-time contract matrix.
pub struct OperationAuthzRow {
    /// Stable operation name.
    pub operation: &'static str,
    /// Operation subject.
    pub subject: &'static str,
    /// Operation scope.
    pub scope: &'static str,
    /// Allowed groups.
    pub allowed_groups: &'static [&'static str],
    /// Destructive flag.
    pub destructive: bool,
    /// Secret access class.
    pub secret_access: SecretAccess,
    /// Broker requirement class.
    pub broker_required: BrokerRequirement,
    /// Audit mode.
    pub audit_mode: AuditMode,
}

fn operation_schema(_gen: &mut SchemaGenerator) -> Schema {
    let mut operations: Vec<_> = PUBLIC_OPERATION_AUTHZ
        .iter()
        .chain(BROKER_OPERATION_AUTHZ.iter())
        .map(|row| row.operation.to_owned())
        .collect();
    operations.sort();
    operations.dedup();
    let operations = operations
        .into_iter()
        .map(serde_json::Value::String)
        .collect();

    let mut obj = SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        enum_values: Some(operations),
        ..Default::default()
    };
    obj.metadata = Some(Box::new(Metadata {
        description: Some("Closed public CLI/API and broker operation name.".to_owned()),
        ..Default::default()
    }));
    Schema::Object(obj)
}

/// Complete initial public CLI/API authorization matrix from the portability plan.
pub const PUBLIC_OPERATION_AUTHZ: &[OperationAuthzRow] = &[
    row(
        "hello",
        "daemon",
        "global",
        &["any-local-client"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::DenyOnly,
    ),
    row(
        "capabilities",
        "daemon",
        "global",
        &["any-local-client"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::DenyOnly,
    ),
    row(
        "auth status",
        "daemon",
        "global",
        &["any-local-client"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::DenyOnly,
    ),
    row(
        "list",
        "VM/env",
        "global-or-scoped",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "status",
        "VM/env",
        "global-or-scoped",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "status --check-bridges",
        "VM/env",
        "global-or-scoped",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audit",
        "host/VM",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "audit --human",
        "host/VM",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "audit --json",
        "host/VM",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "host check",
        "host",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host doctor --read-only",
        "host",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host prepare",
        "host",
        "global",
        &["nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host prepare --dry-run",
        "host",
        "global",
        &["nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    // `host install` now accepts the standard `--dry-run` / `--apply`
    // flag pair, but the authz surface remains a single plain
    // `host install` verb row: the broker authorizes the operation,
    // not the specific flag flavor. Keep the privilege row stable even
    // as the CLI routes `--apply` through the daemon → broker
    // `RunHostInstall` path.
    row(
        "host install",
        "host",
        "global",
        &["nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host destroy --dry-run",
        "host",
        "global",
        &["nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host prepare --apply",
        "host",
        "global",
        &["nixling-admin"],
        true,
        SecretAccess::PossiblePathsOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "host reconcile-otel-acls --apply",
        "host/observability",
        "global",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "host destroy --apply",
        "host",
        "global",
        &["nixling-admin"],
        true,
        SecretAccess::PossiblePathsOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "up",
        "VM/env",
        "per-VM/per-env",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "down",
        "VM/env",
        "per-VM/per-env",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "restart",
        "VM/env",
        "per-VM/per-env",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "console",
        "VM",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "config",
        "VM",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "build",
        "VM",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "generations",
        "VM",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "exec",
        "VM/process",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "switch",
        "VM",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "boot",
        "VM",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "test",
        "VM",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "rollback",
        "VM",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "gc",
        "VM/global",
        "per-VM/global",
        &["nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "store verify",
        "store/VM",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "keys list",
        "key",
        "per-VM",
        &["nixling-admin"],
        false,
        SecretAccess::PublicKeyOnly,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "keys show",
        "key",
        "per-VM",
        &["nixling-admin"],
        false,
        SecretAccess::PublicKeyOnly,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "keys rotate",
        "key",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "trust",
        "key/known-host",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::HostKeyMetadata,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "rotate-known-host",
        "key/known-host",
        "per-VM",
        &["nixling-admin"],
        true,
        SecretAccess::HostKeyMetadata,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "audio",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audio status",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audio mic",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio speaker",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio on",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio off",
        "VM/audio",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb",
        "VM/USB busid",
        "per-VM/per-env",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb attach",
        "VM/USB busid",
        "per-VM/per-env/per-busid",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb detach",
        "VM/USB busid",
        "per-VM/per-env/per-busid",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb probe",
        "VM/USB busid",
        "global",
        &["nixling-launcher", "nixling-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "debug bundle",
        "diagnostics",
        "scoped",
        &["nixling-admin"],
        false,
        SecretAccess::RedactedOnly,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "migrate",
        "host/state",
        "global",
        &["nixling-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
];

/// Complete initial private broker enum authorization matrix from the portability plan.
pub const BROKER_OPERATION_AUTHZ: &[OperationAuthzRow] = &[
    row(
        "Hello",
        "handshake",
        "global",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ValidateBundle",
        "bundle",
        "global",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunHostInstall",
        "installer",
        "global",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunMigrate",
        "installer",
        "global",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunActivation",
        "VM",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunGc",
        "VM/global",
        "per-VM/global",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunKeysRotate",
        "key",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunHostKeyTrust",
        "key/known-host",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::HostKeyMetadata,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RunRotateKnownHost",
        "key/known-host",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::HostKeyMetadata,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "PrepareRuntimeDir",
        "fs",
        "global/per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "PrepareStateDir",
        "fs",
        "global/per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "CreateOrReconcileUsersGroups",
        "account",
        "global/per-role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    // The cgroup delegation op only chowns
    // /sys/fs/cgroup/nixling.slice and its descendants; it does not
    // destroy data. The broker variant plan classes it
    // `Destructive: no (chown only)`. The earlier baseline row
    // above incorrectly marked it destructive; align with the
    // typed broker flag table.
    row(
        "DelegateCgroupV2",
        "cgroup",
        "global/per-VM/role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OpenCgroupDir",
        "cgroup",
        "global/per-VM/role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OpenKvm",
        "device",
        "per-role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    // OpenPidfd handles daemon-side reconcile-and-adopt. The
    // broker calls pidfd_open(pid) AND re-verifies field-22
    // start-time atomically, returning the fd via SCM_RIGHTS.
    row(
        "OpenPidfd",
        "pidfd",
        "per-VM/role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OpenVhostNet",
        "device",
        "per-role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OpenFuse",
        "device",
        "per-role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OpenDevice",
        "device",
        "per-role",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "CreateTapFd",
        "network",
        "per-env/VM/TAP",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "CreatePersistentTap",
        "network",
        "per-env/VM/TAP",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SetBridgePortFlags",
        "network",
        "per-env/VM/TAP",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ApplyNftables",
        "network-host",
        "global/per-env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ApplyRoute",
        "network-host",
        "global/per-env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ApplySysctl",
        "network-host",
        "global/per-env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ApplyNmUnmanaged",
        "network-host",
        "global/per-env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "UpdateHostsFile",
        "name-resolution",
        "global",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "BindUnixSocket",
        "socket",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SetSocketAcl",
        "socket",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SetupMountNamespace",
        "mount/store",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "PrepareStoreView",
        "mount/store",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "LaunchMinijailChild",
        "process",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ReadSecretById",
        "secret/key",
        "per-VM/key",
        &["nixlingd"],
        false,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "GuestControlSign",
        "guest-control token",
        "per-VM",
        &["nixlingd"],
        false,
        SecretAccess::RedactedOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "InjectSecretById",
        "secret/key",
        "per-VM/key",
        &["nixlingd"],
        false,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "RotateSecretById",
        "secret/key",
        "per-VM/key",
        &["nixlingd"],
        true,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "UsbipBind",
        "USBIP",
        "per-busid/env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "UsbipUnbind",
        "USBIP",
        "per-busid/env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "UsbipProxyReconcile",
        "USBIP",
        "per-busid/env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ModprobeIfAllowed",
        "kernel-module",
        "global/feature",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "PauseBroker",
        "broker-admin",
        "global",
        &["nixlingd"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ResumeBroker",
        "broker-admin",
        "global",
        &["nixlingd"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "ExportBrokerAudit",
        "broker-admin",
        "global",
        &["nixlingd"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "UsbipBindFirewallRule",
        "USBIP firewall",
        "per-busid",
        &["nixlingd"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SignalRunner",
        "runner",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "DeregisterRunnerPidfd",
        "runner",
        "per-VM",
        &["nixling-launcher", "nixling-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "PollChildReaped",
        "runner",
        "global",
        &["nixlingd"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "StoreSync",
        "store",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "StoreVerify",
        "store",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SeedDnsmasqLease",
        "network",
        "per-VM/env",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "BindMountFromHardlinkFarm",
        "mount/store",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "OwnershipMatrixCheck",
        "host",
        "per-VM",
        &["nixlingd"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "SshHostKeyPreflight",
        "ssh-host-key",
        "per-VM",
        &["nixlingd"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "DiskInit",
        "disk",
        "per-VM",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    // Spawning a CH / virtiofsd / swtpm role. The broker dispatcher
    // returns `Unimplemented` until the broker-side spawn implementation
    // lands.
    row(
        "SpawnRunner",
        "vm-runner",
        "per-VM/role",
        &["nixlingd"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
];

#[allow(clippy::too_many_arguments)]
const fn row(
    operation: &'static str,
    subject: &'static str,
    scope: &'static str,
    allowed_groups: &'static [&'static str],
    destructive: bool,
    secret_access: SecretAccess,
    broker_required: BrokerRequirement,
    audit_mode: AuditMode,
) -> OperationAuthzRow {
    OperationAuthzRow {
        operation,
        subject,
        scope,
        allowed_groups,
        destructive,
        secret_access,
        broker_required,
        audit_mode,
    }
}

impl From<&OperationAuthzRow> for OperationAuthz {
    fn from(row: &OperationAuthzRow) -> Self {
        Self {
            operation: row.operation.to_owned(),
            subject: row.subject.to_owned(),
            scope: row.scope.to_owned(),
            allowed_groups: row
                .allowed_groups
                .iter()
                .map(|group| (*group).to_owned())
                .collect(),
            destructive: row.destructive,
            secret_access: row.secret_access.clone(),
            broker_required: row.broker_required.clone(),
            audit: AuditPolicy {
                required: !matches!(row.audit_mode, AuditMode::DenyOnly | AuditMode::Errors),
                mode: row.audit_mode.clone(),
                retained_fields: vec![
                    "operation".to_owned(),
                    "subject".to_owned(),
                    "scope".to_owned(),
                    "result".to_owned(),
                ],
            },
            default_for_unknown: DefaultForUnknown::DenyAndAudit,
        }
    }
}

impl PrivilegesJson {
    /// Builds the canonical privileges matrix from the const rows.
    pub fn w1(schema_version: impl Into<String>) -> Self {
        Self {
            schema_version: schema_version.into(),
            public_operations: PUBLIC_OPERATION_AUTHZ
                .iter()
                .map(OperationAuthz::from)
                .collect(),
            broker_operations: BROKER_OPERATION_AUTHZ
                .iter()
                .map(OperationAuthz::from)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BROKER_OPERATION_AUTHZ, PUBLIC_OPERATION_AUTHZ, PrivilegesJson};

    #[test]
    fn w1_matrix_contains_public_and_broker_rows() {
        let matrix = PrivilegesJson::w1("v1");
        assert_eq!(matrix.public_operations.len(), PUBLIC_OPERATION_AUTHZ.len());
        assert_eq!(matrix.broker_operations.len(), BROKER_OPERATION_AUTHZ.len());
        assert!(
            matrix
                .broker_operations
                .iter()
                .any(|row| row.operation == "DelegateCgroupV2")
        );
    }

    #[test]
    fn privileges_json_denies_unknown_fields() {
        let err = serde_json::from_str::<PrivilegesJson>(
            r#"{"schemaVersion":"v1","publicOperations":[],"brokerOperations":[],"extra":true}"#,
        )
        .expect_err("unknown fields fail closed");
        assert!(err.to_string().contains("unknown field"));
    }
}
