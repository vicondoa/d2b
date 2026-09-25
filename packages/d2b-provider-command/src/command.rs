//! Command ResourceType contract: one declared launch shape.
//!
//! A `Command` row is fixed at startup and declares how a worker is launched:
//! the executable, the argument vector with its placeholder slots, the
//! parameters those slots draw from (as a JSON Schema), the role the worker
//! runs as, and the intent facet that maps bundle entries onto the command.
//!
//! The argument vector is strictly argv-shaped, never a shell string:
//! placeholders occupy whole slots and must name a declared parameter, so the
//! closed placeholder list is the only substitution surface. The command's
//! role reference resolves against the committed roles at seed time, and the
//! command itself is committed by the foundation seed, never created at
//! runtime.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::payload_schema::PayloadSchema;
use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken, PrimitiveSpecError, redacted_debug};

/// Canonical `Command` ResourceType name.
pub const COMMAND_RESOURCE_TYPE: &str = "Command";
/// Maximum argument slots in one command.
pub const MAX_COMMAND_ARGV_SLOTS: usize = 64;
/// Maximum bytes of one argument slot.
pub const MAX_COMMAND_ARGV_SLOT_BYTES: usize = 4096;
/// Maximum bytes of one executable path.
pub const MAX_COMMAND_EXEC_BYTES: usize = 4096;

/// A validated absolute executable path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CommandExec(String);

impl CommandExec {
    /// Parse an absolute path with no control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, CommandContractError> {
        let value = value.into();
        if !value.starts_with('/') || value.len() > MAX_COMMAND_EXEC_BYTES {
            return Err(CommandContractError::InvalidExec);
        }
        if value.chars().any(char::is_control) || value.contains('\u{0}') {
            return Err(CommandContractError::InvalidExec);
        }
        Ok(Self(value))
    }

    /// Borrow the path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(CommandExec);

impl<'de> Deserialize<'de> for CommandExec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CommandExec {
    fn schema_name() -> String {
        "CommandExec".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().pattern = Some("^/[^\\u0000]*$".to_owned());
        schema.string().max_length = Some(MAX_COMMAND_EXEC_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// One argument slot: a literal, or a whole-slot `{placeholder}`.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandArgvSlot(String);

impl CommandArgvSlot {
    /// Parse one slot. A slot carrying a brace must be a single placeholder.
    pub fn parse(value: impl Into<String>) -> Result<Self, CommandContractError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_COMMAND_ARGV_SLOT_BYTES {
            return Err(CommandContractError::InvalidArgvSlot);
        }
        if value.contains('\u{0}') {
            return Err(CommandContractError::InvalidArgvSlot);
        }
        if value.contains(['{', '}']) {
            let Some(name) = value
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            else {
                return Err(CommandContractError::InvalidArgvSlot);
            };
            if !d2b_contracts_resource::v3::payload_schema::valid_property_name(name) {
                return Err(CommandContractError::InvalidArgvSlot);
            }
            Ok(Self(format!("{{{name}}}")))
        } else {
            Ok(Self(value))
        }
    }

    /// The placeholder's parameter name, when this slot is a placeholder.
    pub fn placeholder(&self) -> Option<&str> {
        self.0.strip_prefix('{')?.strip_suffix('}')
    }

    /// The literal slot text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(CommandArgvSlot);

impl<'de> Deserialize<'de> for CommandArgvSlot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CommandArgvSlot {
    fn schema_name() -> String {
        "CommandArgvSlot".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.metadata().description = Some(
            "One argument slot: a literal, or a whole-slot {placeholder} naming a declared parameter."
                .to_owned(),
        );
        schema.string().max_length = Some(MAX_COMMAND_ARGV_SLOT_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// The intent facet: how bundle entries map onto one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandIntent {
    grammar: BoundedText,
    mint: BoundedToken,
}

impl CommandIntent {
    /// Construct one intent facet.
    pub fn new(grammar: BoundedText, mint: BoundedToken) -> Self {
        Self { grammar, mint }
    }

    /// The intent-id grammar this command's entries are spelled with.
    pub fn grammar(&self) -> &BoundedText {
        &self.grammar
    }

    /// The mint rule that turns an entry into an executable intent.
    pub fn mint(&self) -> &BoundedToken {
        &self.mint
    }
}

/// The `Command` desired spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    exec: CommandExec,
    argv: Vec<CommandArgvSlot>,
    params: PayloadSchema,
    role_ref: ResourceRef,
    intent: CommandIntent,
}

impl CommandSpec {
    /// Construct a command spec after validating slots against parameters.
    pub fn new(
        exec: CommandExec,
        argv: Vec<CommandArgvSlot>,
        params: PayloadSchema,
        role_ref: ResourceRef,
        intent: CommandIntent,
    ) -> Result<Self, CommandContractError> {
        if argv.is_empty() || argv.len() > MAX_COMMAND_ARGV_SLOTS {
            return Err(CommandContractError::InvalidArgvSlot);
        }
        d2b_contracts_resource::v3::execution_policy::require_resource_type(&role_ref, "Role")
            .map_err(|_| CommandContractError::InvalidRoleRef)?;
        for slot in &argv {
            if let Some(name) = slot.placeholder()
                && !params.declares(name)
            {
                return Err(CommandContractError::UndefinedPlaceholder);
            }
        }
        Ok(Self {
            exec,
            argv,
            params,
            role_ref,
            intent,
        })
    }

    /// The executable path.
    pub fn exec(&self) -> &CommandExec {
        &self.exec
    }

    /// The argument vector.
    pub fn argv(&self) -> &[CommandArgvSlot] {
        &self.argv
    }

    /// The parameter schema the placeholders draw from.
    pub fn params(&self) -> &PayloadSchema {
        &self.params
    }

    /// The worker role this command runs as.
    pub fn role_ref(&self) -> &ResourceRef {
        &self.role_ref
    }

    /// The intent facet.
    pub fn intent(&self) -> &CommandIntent {
        &self.intent
    }
}

redacted_debug!(CommandSpec);

impl<'de> Deserialize<'de> for CommandSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            exec: CommandExec,
            argv: Vec<CommandArgvSlot>,
            params: PayloadSchema,
            role_ref: ResourceRef,
            intent: CommandIntent,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.exec,
            wire.argv,
            wire.params,
            wire.role_ref,
            wire.intent,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// One invalid command declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandContractError {
    /// The executable is not an absolute, control-free, bounded path.
    InvalidExec,
    /// The argument vector is empty, over bound, or carries a malformed slot.
    InvalidArgvSlot,
    /// The role reference does not name a `Role`.
    InvalidRoleRef,
    /// A placeholder slot names a parameter the schema does not declare.
    UndefinedPlaceholder,
}

impl core::fmt::Display for CommandContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidExec => "command executable is not an absolute path",
            Self::InvalidArgvSlot => "command argument slot is malformed",
            Self::InvalidRoleRef => "command roleRef does not name a Role",
            Self::UndefinedPlaceholder => "command placeholder names an undeclared parameter",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for CommandContractError {}

impl From<PrimitiveSpecError> for CommandContractError {
    fn from(_: PrimitiveSpecError) -> Self {
        Self::InvalidRoleRef
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params() -> PayloadSchema {
        PayloadSchema::parse(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["socketPath", "sharedDir"],
            "properties": {
                "socketPath": { "type": "string", "pattern": "^/run/d2b/vfs/[A-Za-z0-9._-]+$" },
                "sharedDir": { "type": "string" },
                "supervisorToken": { "type": "string", "writeOnly": true, "minLength": 32 },
                "workerCount": { "type": "integer", "minimum": 1, "maximum": 64, "default": 2 }
            }
        }))
        .expect("params validate")
    }

    fn virtiofsd() -> CommandSpec {
        CommandSpec::new(
            CommandExec::parse("/usr/lib/d2b/libexec/virtiofsd").unwrap(),
            vec![
                CommandArgvSlot::parse("--socket-path").unwrap(),
                CommandArgvSlot::parse("{socketPath}").unwrap(),
                CommandArgvSlot::parse("--shared-dir").unwrap(),
                CommandArgvSlot::parse("{sharedDir}").unwrap(),
            ],
            params(),
            ResourceRef::parse("Role/virtiofsd-worker").unwrap(),
            CommandIntent::new(
                BoundedText::parse("<zone>/<command>/<name>").unwrap(),
                BoundedToken::parse("per-bundle-entry").unwrap(),
            ),
        )
        .expect("command validates")
    }

    #[test]
    fn a_placeholder_slot_draws_from_a_declared_parameter() {
        let command = virtiofsd();
        assert_eq!(command.argv()[1].placeholder(), Some("socketPath"));
        assert_eq!(command.argv()[0].placeholder(), None);
        assert!(command.params().declares("supervisorToken"));
    }

    #[test]
    fn an_undeclared_placeholder_is_refused() {
        let error = CommandSpec::new(
            CommandExec::parse("/bin/true").unwrap(),
            vec![CommandArgvSlot::parse("{missing}").unwrap()],
            params(),
            ResourceRef::parse("Role/worker").unwrap(),
            virtiofsd().intent().clone(),
        )
        .expect_err("undeclared placeholder");
        assert_eq!(error, CommandContractError::UndefinedPlaceholder);
    }

    #[test]
    fn argv_slots_and_exec_and_slot_bytes_are_bounded() {
        // Empty argv and more than 64 slots are refused.
        assert_eq!(
            CommandSpec::new(
                CommandExec::parse("/bin/true").unwrap(),
                vec![],
                params(),
                ResourceRef::parse("Role/worker").unwrap(),
                virtiofsd().intent().clone(),
            )
            .expect_err("empty argv"),
            CommandContractError::InvalidArgvSlot
        );
        let mut overlong = Vec::new();
        for index in 0..=MAX_COMMAND_ARGV_SLOTS {
            overlong
                .push(CommandArgvSlot::parse(format!("--flag-{index}")).unwrap());
        }
        assert_eq!(
            CommandSpec::new(
                CommandExec::parse("/bin/true").unwrap(),
                overlong,
                params(),
                ResourceRef::parse("Role/worker").unwrap(),
                virtiofsd().intent().clone(),
            )
            .expect_err("over-bound argv"),
            CommandContractError::InvalidArgvSlot
        );

        // The 4096-byte slot ceiling: exactly at the bound is admitted, one
        // byte over is refused.
        let at_boundary = "a".repeat(MAX_COMMAND_ARGV_SLOT_BYTES);
        assert!(CommandArgvSlot::parse(at_boundary).is_ok());
        assert!(
            CommandArgvSlot::parse("a".repeat(MAX_COMMAND_ARGV_SLOT_BYTES + 1)).is_err()
        );

        // The 4096-byte exec ceiling, same boundary.
        let exec_at_boundary = format!("/{}", "a".repeat(MAX_COMMAND_EXEC_BYTES - 1));
        assert!(CommandExec::parse(exec_at_boundary).is_ok());
        assert!(
            CommandExec::parse(format!("/{}", "a".repeat(MAX_COMMAND_EXEC_BYTES))).is_err()
        );
    }

    #[test]
    fn partial_braces_and_foreign_roles_are_refused() {
        for slot in ["-{socketPath}", "{socketPath", "a}b", "{Upper}"] {
            assert!(CommandArgvSlot::parse(slot).is_err(), "{slot:?}");
        }
        assert!(
            CommandSpec::new(
                CommandExec::parse("/bin/true").unwrap(),
                vec![CommandArgvSlot::parse("--flag").unwrap()],
                params(),
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                virtiofsd().intent().clone(),
            )
            .is_err()
        );
        for exec in ["bin/true", "", "/bin/true\u{0}"] {
            assert!(CommandExec::parse(exec).is_err(), "{exec:?}");
        }
    }

    #[test]
    fn the_wire_shape_round_trips_and_refuses_unknown_fields() {
        let command = virtiofsd();
        let bytes = d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&command).expect("canonical");
        let restored: CommandSpec = serde_json::from_slice(&bytes).expect("parses");
        assert_eq!(restored, command);
        let unknown = json!({
            "exec": "/bin/true",
            "argv": ["--flag"],
            "params": { "type": "object", "additionalProperties": false, "properties": {} },
            "roleRef": "Role/worker",
            "intent": { "grammar": "<zone>/<name>", "mint": "per-bundle-entry" },
            "shell": "true"
        });
        assert!(serde_json::from_value::<CommandSpec>(unknown).is_err());
    }
}
