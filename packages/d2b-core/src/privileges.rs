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
        "op",
        "operation/realm state",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "resource",
        "Zone Resource API",
        "per-Zone",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "vm",
        "VM command family",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "activation",
        "VM/activation",
        "global-or-scoped",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "device",
        "device",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "display",
        "VM/display",
        "per-VM/per-realm",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "guest",
        "Guest",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "clipboard",
        "host clipboard",
        "local-user-session",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::MetadataOnly,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "realm",
        "realm command family",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "list",
        "VM/env",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "status",
        "VM/env",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "status --check-bridges",
        "VM/env",
        "global-or-scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audit",
        "host/VM",
        "global",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "audit --human",
        "host/VM",
        "global",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "audit --json",
        "host/VM",
        "global",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "host doctor --read-only",
        "host",
        "global",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host prepare",
        "host",
        "global",
        &["d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host prepare --dry-run",
        "host",
        "global",
        &["d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host destroy --dry-run",
        "host",
        "global",
        &["d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
    row(
        "host shutdown-hook --apply",
        "host lifecycle",
        "global",
        &["host-shutdown"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "host prepare --apply",
        "host",
        "global",
        &["d2b-admin"],
        true,
        SecretAccess::PossiblePathsOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "host reconcile-otel-acls --apply",
        "host/observability",
        "global",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "host destroy --apply",
        "host",
        "global",
        &["d2b-admin"],
        true,
        SecretAccess::PossiblePathsOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "up",
        "VM/env",
        "per-VM/per-env",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "down",
        "VM/env",
        "per-VM/per-env",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "restart",
        "VM/env",
        "per-VM/per-env",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "console",
        "VM",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "config",
        "VM",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "build",
        "VM",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Conditional,
        AuditMode::Yes,
    ),
    row(
        "generations",
        "VM",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "launch",
        "workload/configured launch",
        "per-workload/per-realm",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "exec",
        "VM/process",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "shell",
        "VM/persistent shell",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "vm display",
        "VM/display",
        "per-VM/per-realm",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::No,
        AuditMode::Yes,
    ),
    row(
        "switch",
        "VM",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "boot",
        "VM",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "test",
        "VM",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "rollback",
        "VM",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::MetadataOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "keys rotate",
        "key",
        "per-VM",
        &["d2b-admin"],
        true,
        SecretAccess::ReadWrite,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audio status",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::No,
        AuditMode::Errors,
    ),
    row(
        "audio mic",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio speaker",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio on",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "audio off",
        "VM/audio",
        "per-VM",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb",
        "VM/USB busid",
        "per-VM/per-env",
        &["d2b-launcher", "d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb attach",
        "VM/USB busid",
        "per-VM/per-env/per-busid",
        &["d2b-admin"],
        true,
        SecretAccess::RedactedOnly,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb detach",
        "VM/USB busid",
        "per-VM/per-env/per-busid",
        &["d2b-admin"],
        true,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb probe",
        "VM/USB busid",
        "global",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "usb security-key",
        "VM/USB security-key",
        "scoped",
        &["d2b-launcher", "d2b-admin"],
        false,
        SecretAccess::None,
        BrokerRequirement::Yes,
        AuditMode::Yes,
    ),
    row(
        "debug bundle",
        "diagnostics",
        "scoped",
        &["d2b-admin"],
        false,
        SecretAccess::RedactedOnly,
        BrokerRequirement::NoMutation,
        AuditMode::Yes,
    ),
];

include!("generated/broker_operation_authz.rs");

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
    pub fn from_const_rows(schema_version: impl Into<String>) -> Self {
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
        let matrix = PrivilegesJson::from_const_rows("v1");
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
