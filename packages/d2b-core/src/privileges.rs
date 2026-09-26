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
    pub destructive: Destructive,
    /// Whether secret or key material can be read or modified.
    pub secret_access: SecretAccess,
    /// Whether the private broker is required or conditionally used.
    pub broker_required: BrokerRequirement,
    /// Audit event requirement and retained fields.
    pub audit: AuditPolicy,
    /// Default policy for unknown/future operations.
    pub default_for_unknown: DefaultForUnknown,
}

/// Destructive class for an authorization row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Destructive {
    /// No state mutation, teardown, rollback, GC, or live routing change.
    No,
    /// State mutation, teardown, rollback, GC, or live routing changes are
    /// possible.
    Yes,
}

/// Secret exposure class for an authorization row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Destructive class.
    pub destructive: Destructive,
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
    OperationAuthzRow {
        operation: "hello",
        subject: "daemon",
        scope: "global",
        allowed_groups: &["any-local-client"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::DenyOnly,
    },
    OperationAuthzRow {
        operation: "capabilities",
        subject: "daemon",
        scope: "global",
        allowed_groups: &["any-local-client"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::DenyOnly,
    },
    OperationAuthzRow {
        operation: "auth status",
        subject: "daemon",
        scope: "global",
        allowed_groups: &["any-local-client"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::DenyOnly,
    },
    OperationAuthzRow {
        operation: "op",
        subject: "operation/realm state",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "resource",
        subject: "Zone Resource API",
        scope: "per-Zone",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "vm",
        subject: "VM command family",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "activation",
        subject: "VM/activation",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "device",
        subject: "device",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "display",
        subject: "VM/display",
        scope: "per-VM/per-realm",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "guest",
        subject: "Guest",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "clipboard",
        subject: "host clipboard",
        scope: "local-user-session",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "realm",
        subject: "realm command family",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "list",
        subject: "VM/env",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "status",
        subject: "VM/env",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "status --check-bridges",
        subject: "VM/env",
        scope: "global-or-scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "audit",
        subject: "host/VM",
        scope: "global",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audit --human",
        subject: "host/VM",
        scope: "global",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audit --json",
        subject: "host/VM",
        scope: "global",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host doctor --read-only",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::NoMutation,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host prepare",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::NoMutation,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host prepare --dry-run",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::NoMutation,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host destroy --dry-run",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::NoMutation,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host shutdown-hook --apply",
        subject: "host lifecycle",
        scope: "global",
        allowed_groups: &["host-shutdown"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host prepare --apply",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::PossiblePathsOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host reconcile-otel-acls --apply",
        subject: "host/observability",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "host destroy --apply",
        subject: "host",
        scope: "global",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::PossiblePathsOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "up",
        subject: "VM/env",
        scope: "per-VM/per-env",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "down",
        subject: "VM/env",
        scope: "per-VM/per-env",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "restart",
        subject: "VM/env",
        scope: "per-VM/per-env",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "console",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "config",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "build",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Conditional,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "generations",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "launch",
        subject: "workload/configured launch",
        scope: "per-workload/per-realm",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "exec",
        subject: "VM/process",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "shell",
        subject: "VM/persistent shell",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "vm display",
        subject: "VM/display",
        scope: "per-VM/per-realm",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "switch",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "boot",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "test",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "rollback",
        subject: "VM",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::MetadataOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "keys rotate",
        subject: "key",
        scope: "per-VM",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::ReadWrite,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audio",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "audio status",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::No,
        audit_mode: AuditMode::Errors,
    },
    OperationAuthzRow {
        operation: "audio mic",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audio speaker",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audio on",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "audio off",
        subject: "VM/audio",
        scope: "per-VM",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "usb",
        subject: "VM/USB busid",
        scope: "per-VM/per-env",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "usb attach",
        subject: "VM/USB busid",
        scope: "per-VM/per-env/per-busid",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::RedactedOnly,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "usb detach",
        subject: "VM/USB busid",
        scope: "per-VM/per-env/per-busid",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::Yes,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "usb probe",
        subject: "VM/USB busid",
        scope: "global",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "usb security-key",
        subject: "VM/USB security-key",
        scope: "scoped",
        allowed_groups: &["d2b-launcher", "d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::None,
        broker_required: BrokerRequirement::Yes,
        audit_mode: AuditMode::Yes,
    },
    OperationAuthzRow {
        operation: "debug bundle",
        subject: "diagnostics",
        scope: "scoped",
        allowed_groups: &["d2b-admin"],
        destructive: Destructive::No,
        secret_access: SecretAccess::RedactedOnly,
        broker_required: BrokerRequirement::NoMutation,
        audit_mode: AuditMode::Yes,
    },
];

include!("generated/broker_operation_authz.rs");

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
            secret_access: row.secret_access,
            broker_required: row.broker_required,
            audit: AuditPolicy {
                required: !matches!(row.audit_mode, AuditMode::DenyOnly | AuditMode::Errors),
                mode: row.audit_mode,
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
