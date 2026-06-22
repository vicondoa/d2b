//! Persistent-shell DTOs for the constellation semantic operation family
//! (ADR 0039). These types carry bounded shell metadata only: terminal bytes,
//! argv, environment, cwd, provider endpoints, and credentials stay in opaque
//! operation/stream payloads owned by higher layers and never appear here.

use crate::ids::StreamId;
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum bytes in a shell name.
pub const MAX_SHELL_NAME_LEN: usize = 64;
/// Maximum summaries returned by one list response.
pub const MAX_SHELL_SUMMARIES: usize = 256;
/// Maximum bytes in a shell attach/session correlation id.
pub const MAX_SHELL_OPAQUE_ID_LEN: usize = 128;

const SHELL_NAME_PATTERN: &str = "^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$";
const SHELL_OPAQUE_ID_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*$";

const SECRET_MARKERS: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "bearer",
    "credential",
    "apikey",
    "privatekey",
    "accesstoken",
    "refreshtoken",
    "sessiontoken",
];

/// Reason a shell name failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellNameError {
    Empty,
    TooLong,
    BadShape,
    Reserved,
}

impl core::fmt::Display for ShellNameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "shell name is empty"),
            Self::TooLong => write!(f, "shell name exceeds {MAX_SHELL_NAME_LEN} bytes"),
            Self::BadShape => write!(f, "shell name has an invalid shape"),
            Self::Reserved => write!(f, "shell name is reserved"),
        }
    }
}

impl std::error::Error for ShellNameError {}

/// A validated persistent shell name. The shape follows ADR 0038 and is
/// intentionally narrower than shpool templates.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ShellName(String);

impl ShellName {
    /// Validate and construct a shell name.
    pub fn parse(raw: impl Into<String>) -> Result<Self, ShellNameError> {
        let raw = raw.into();
        if raw.is_empty() {
            return Err(ShellNameError::Empty);
        }
        if raw.len() > MAX_SHELL_NAME_LEN {
            return Err(ShellNameError::TooLong);
        }
        if raw == "." || raw == ".." {
            return Err(ShellNameError::Reserved);
        }
        let mut bytes = raw.bytes();
        let Some(first) = bytes.next() else {
            return Err(ShellNameError::Empty);
        };
        if !(first.is_ascii_alphanumeric() || first == b'_') {
            return Err(ShellNameError::BadShape);
        }
        if !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return Err(ShellNameError::BadShape);
        }
        Ok(Self(raw))
    }

    /// Borrow the validated shell name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for ShellName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ShellName").field(&self.0).finish()
    }
}

impl core::fmt::Display for ShellName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ShellName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ShellName {
    fn schema_name() -> String {
        "ShellName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_SHELL_NAME_LEN as u32),
                min_length: Some(1),
                pattern: Some(SHELL_NAME_PATTERN.to_owned()),
            })),
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOpaqueIdError {
    Empty,
    TooLong,
    BadShape,
}

impl core::fmt::Display for ShellOpaqueIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "shell opaque id is empty"),
            Self::TooLong => write!(f, "shell opaque id exceeds {MAX_SHELL_OPAQUE_ID_LEN} bytes"),
            Self::BadShape => write!(f, "shell opaque id has an invalid shape"),
        }
    }
}

impl std::error::Error for ShellOpaqueIdError {}

fn shell_opaque_id_valid(raw: &str) -> bool {
    let Some(first) = raw.bytes().next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if !raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return false;
    }
    if raw.contains("..") {
        return false;
    }
    let compact = raw
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    !SECRET_MARKERS.iter().any(|marker| compact.contains(marker))
}

macro_rules! shell_opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: impl Into<String>) -> Result<Self, ShellOpaqueIdError> {
                let raw = raw.into();
                if raw.is_empty() {
                    return Err(ShellOpaqueIdError::Empty);
                }
                if raw.len() > MAX_SHELL_OPAQUE_ID_LEN {
                    return Err(ShellOpaqueIdError::TooLong);
                }
                if !shell_opaque_id_valid(&raw) {
                    return Err(ShellOpaqueIdError::BadShape);
                }
                Ok(Self(raw))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}(<{} bytes>)", stringify!($name), self.0.len())
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some(MAX_SHELL_OPAQUE_ID_LEN as u32),
                        min_length: Some(1),
                        pattern: Some(SHELL_OPAQUE_ID_PATTERN.to_owned()),
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

shell_opaque_id!(
    /// Opaque correlation id for one persistent shell attach handle.
    ShellAttachId
);
shell_opaque_id!(
    /// Opaque guest-observed persistent shell session instance id.
    ShellSessionInstanceId
);

/// Boot/daemon-generation metadata used to reject stale shell operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellGeneration {
    pub guest_boot_id: ProtocolToken,
    pub guestd_instance_id: ProtocolToken,
    pub shell_daemon_instance_id: ProtocolToken,
}

/// Coarse persistent-shell lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ShellState {
    Starting,
    Detached,
    Attached,
    Terminating,
    Exited,
    Lost,
}

impl ShellState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Lost)
    }
}

/// Bounded cause for an observed shell state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ShellCause {
    AdminDetach,
    ForceDetach,
    AdminKill,
    OwnerDisconnected,
    NetworkLoss,
    DaemonLoss,
    DaemonRestart,
    ResourceKill,
    OrphanReap,
    ReconciliationGap,
    SlowReader,
    OutputGap,
    Unknown,
}

/// Request body for `ShellList`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellListRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generation: Option<ShellGeneration>,
}

/// Request body for `ShellAttach`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellAttachRequest {
    pub name: ShellName,
    pub generation: ShellGeneration,
    pub attach_id: ShellAttachId,
    pub force: bool,
}

/// Request body for `ShellDetach`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellDetachRequest {
    pub name: ShellName,
    pub generation: ShellGeneration,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attach_id: Option<ShellAttachId>,
}

/// Request body for `ShellKill`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellKillRequest {
    pub name: ShellName,
    pub generation: ShellGeneration,
}

/// Bounded summary returned by `ShellList` and status-like responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellSummary {
    pub name: ShellName,
    pub state: ShellState,
    pub generation: ShellGeneration,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_instance_id: Option<ShellSessionInstanceId>,
    pub attached: bool,
    pub is_default: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_cause: Option<ShellCause>,
}

/// Response body for `ShellList`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellListResponse {
    pub generation: ShellGeneration,
    #[schemars(length(max = 256))]
    pub summaries: Vec<ShellSummary>,
}

impl<'de> Deserialize<'de> for ShellListResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            generation: ShellGeneration,
            summaries: Vec<ShellSummary>,
        }

        let raw = Raw::deserialize(deserializer)?;
        if raw.summaries.len() > MAX_SHELL_SUMMARIES {
            return Err(serde::de::Error::custom(format!(
                "shell list exceeds {MAX_SHELL_SUMMARIES} summaries"
            )));
        }
        Ok(Self {
            generation: raw.generation,
            summaries: raw.summaries,
        })
    }
}

/// Successful `ShellAttach` metadata. The terminal stream itself is opened as
/// `StreamKind::ShellPty`, so raw terminal bytes stay out of this DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellAttachSummary {
    pub name: ShellName,
    pub generation: ShellGeneration,
    pub session_instance_id: ShellSessionInstanceId,
    pub attach_id: ShellAttachId,
    pub stream_id: StreamId,
    pub force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation() -> ShellGeneration {
        ShellGeneration {
            guest_boot_id: ProtocolToken::parse("boot-1").unwrap(),
            guestd_instance_id: ProtocolToken::parse("guestd-1").unwrap(),
            shell_daemon_instance_id: ProtocolToken::parse("shell-daemon-1").unwrap(),
        }
    }

    #[test]
    fn shell_names_follow_adr38_shape() {
        for good in ["default", "admin_1", "a.b-c_D", "_ops"] {
            assert!(ShellName::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "-bad",
            "bad/name",
            "bad name",
            "{template}",
            "x{y}",
            "\u{00e9}",
        ] {
            assert_eq!(ShellName::parse(bad).unwrap_err(), ShellNameError::BadShape);
        }
        assert_eq!(ShellName::parse("").unwrap_err(), ShellNameError::Empty);
        assert_eq!(ShellName::parse(".").unwrap_err(), ShellNameError::Reserved);
        assert_eq!(
            ShellName::parse("a".repeat(MAX_SHELL_NAME_LEN + 1)).unwrap_err(),
            ShellNameError::TooLong
        );
        assert!(serde_json::from_str::<ShellName>("\"ops\"").is_ok());
        assert!(serde_json::from_str::<ShellName>("\"bad/name\"").is_err());
    }

    #[test]
    fn shell_request_decode_rejects_unknown_fields() {
        let json = "{\"name\":\"default\",\"generation\":{\"guest_boot_id\":\"boot-1\",\
                    \"guestd_instance_id\":\"guestd-1\",\
                    \"shell_daemon_instance_id\":\"shell-daemon-1\"},\
                    \"attach_id\":\"attach-1\",\
                    \"force\":false,\"argv\":[\"sh\"]}";
        assert!(serde_json::from_str::<ShellAttachRequest>(json).is_err());
    }

    #[test]
    fn shell_summaries_are_redaction_safe_metadata() {
        let summary = ShellSummary {
            name: ShellName::parse("default").unwrap(),
            state: ShellState::Attached,
            generation: generation(),
            session_instance_id: Some(ShellSessionInstanceId::parse("sess-1").unwrap()),
            attached: true,
            is_default: true,
            last_cause: Some(ShellCause::ForceDetach),
        };
        let json = serde_json::to_string(&summary).unwrap();
        for forbidden in [
            "argv",
            "env",
            "cwd",
            "TOKEN=",
            "provider_endpoint",
            "credential",
            "attach-1",
            "terminal bytes",
            "/nix/store",
        ] {
            assert!(
                !json.contains(forbidden),
                "summary leaked {forbidden}: {json}"
            );
        }
        let debug = format!("{:?}", summary.session_instance_id.unwrap());
        assert!(debug.contains("ShellSessionInstanceId(<6 bytes>)"));
        assert!(!debug.contains("sess-1"));
    }

    #[test]
    fn shell_list_response_is_bounded() {
        let ok = ShellListResponse {
            generation: generation(),
            summaries: Vec::new(),
        };
        let json = serde_json::to_string(&ok).unwrap();
        assert!(serde_json::from_str::<ShellListResponse>(&json).is_ok());

        let one = "{\"name\":\"default\",\"state\":\"detached\",\
                   \"generation\":{\"guest_boot_id\":\"boot-1\",\
                   \"guestd_instance_id\":\"guestd-1\",\
                   \"shell_daemon_instance_id\":\"shell-daemon-1\"},\
                   \"attached\":false,\"is_default\":true}";
        let too_many = format!(
            "{{\"generation\":{{\"guest_boot_id\":\"boot-1\",\
             \"guestd_instance_id\":\"guestd-1\",\
             \"shell_daemon_instance_id\":\"shell-daemon-1\"}},\"summaries\":[{}]}}",
            std::iter::repeat_n(one, MAX_SHELL_SUMMARIES + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(serde_json::from_str::<ShellListResponse>(&too_many).is_err());
    }
}
