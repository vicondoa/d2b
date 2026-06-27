//! Strongly-typed identifiers for the constellation model (ADR 0032).
//!
//! Two families:
//!
//! - **Label-shaped** ids (`RealmId` labels, [`NodeId`], [`WorkloadId`],
//!   [`ProviderId`]) reuse the d2b lowercase label shape
//!   `^[a-z][a-z0-9-]*$`.
//! - **Opaque** ids ([`GatewayId`], [`ExecutionId`], [`StreamId`],
//!   [`PrincipalId`], [`OperationId`], [`IdempotencyKey`]) are bounded,
//!   non-empty, log-safe tokens. They deliberately reject path-like and
//!   credential-shaped strings because opaque ids appear in audit and
//!   diagnostic metadata.
//!
//! All constructors validate; malformed input is rejected with
//! [`IdError`] rather than silently accepted (fail-closed).

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum length for any identifier token.
pub const MAX_ID_LEN: usize = 128;

/// ECMA-regex for the d2b lowercase label shape `^[a-z][a-z0-9-]*$`.
const LABEL_PATTERN: &str = "^[a-z][a-z0-9-]*$";
/// ECMA-regex for a non-empty opaque token with an alphanumeric first
/// character and only URL/filename-safe separators afterwards. Additional
/// path/secret-like checks are enforced by [`is_opaque_token`].
const OPAQUE_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*$";

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

/// Reason an identifier failed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    /// The token was empty.
    Empty,
    /// The token exceeded [`MAX_ID_LEN`].
    TooLong,
    /// The token did not match the required shape.
    BadShape,
}

impl core::fmt::Display for IdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IdError::Empty => write!(f, "identifier is empty"),
            IdError::TooLong => write!(f, "identifier exceeds {MAX_ID_LEN} bytes"),
            IdError::BadShape => write!(f, "identifier has an invalid shape"),
        }
    }
}

impl std::error::Error for IdError {}

/// True for the d2b lowercase label shape `^[a-z][a-z0-9-]*$`.
pub fn is_label(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// True for a bounded, non-empty opaque token safe for audit/log metadata.
fn is_opaque_token(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return false;
    }
    if s.contains("..") {
        return false;
    }
    let compact = s
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect::<String>();
    !SECRET_MARKERS.iter().any(|marker| compact.contains(marker))
}

#[derive(Clone, Copy)]
enum IdDebug {
    Clear,
    Redacted,
}

fn fmt_id_debug(
    name: &str,
    value: &str,
    mode: IdDebug,
    f: &mut core::fmt::Formatter<'_>,
) -> core::fmt::Result {
    match mode {
        IdDebug::Clear => f.debug_tuple(name).field(&value).finish(),
        IdDebug::Redacted => write!(f, "{name}(<{} bytes>)", value.len()),
    }
}

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident, $validate:expr_2021, $pattern:expr_2021, $debug:expr_2021) => {
        $(#[$meta])*
        #[derive(
            Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and construct. Returns [`IdError`] on malformed
            /// input (fail-closed).
            pub fn parse(raw: impl Into<String>) -> Result<Self, IdError> {
                let s = raw.into();
                if s.is_empty() {
                    return Err(IdError::Empty);
                }
                if s.len() > MAX_ID_LEN {
                    return Err(IdError::TooLong);
                }
                let validate: fn(&str) -> bool = $validate;
                if !validate(&s) {
                    return Err(IdError::BadShape);
                }
                Ok(Self(s))
            }

            /// Borrow the underlying token.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                fmt_id_debug(stringify!($name), &self.0, $debug, f)
            }
        }

        // Fail-closed decode: deserialization routes through `parse` so a
        // codec/serde path can never instantiate a malformed identifier.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }

        // Schema carries the bound + regular shape. Additional semantic
        // deny-list checks (path/credential-shaped tokens) are enforced by
        // `parse`/`Deserialize`.
        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some(MAX_ID_LEN as u32),
                        min_length: Some(1),
                        pattern: Some($pattern.to_owned()),
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

id_newtype!(
    /// A single realm label. A full realm path is a `.`-joined sequence
    /// of these; see [`crate::realm::RealmPath`].
    RealmId,
    is_label,
    LABEL_PATTERN,
    IdDebug::Clear
);
id_newtype!(
    /// A node within a realm (a host, gateway, or provider-managed node).
    NodeId,
    is_label,
    LABEL_PATTERN,
    IdDebug::Clear
);
id_newtype!(
    /// A workload (VM, session, or sandbox) on a node.
    WorkloadId,
    is_label,
    LABEL_PATTERN,
    IdDebug::Clear
);
id_newtype!(
    /// A provider implementation id.
    ProviderId,
    is_label,
    LABEL_PATTERN,
    IdDebug::Clear
);
id_newtype!(
    /// A realm gateway guest identity. Opaque (not operator-typed).
    GatewayId,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// A durable execution id.
    ExecutionId,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// A multiplexed stream id within a peer session.
    StreamId,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// An opaque resume cursor for a `Logs` stream. The peer echoes the
    /// last cursor it durably consumed so a re-opened logs stream resumes
    /// without gaps or replay. Opaque + bounded (never operator-typed).
    StreamCursor,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// An authenticated principal (never a relay credential).
    PrincipalId,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// Audit/correlation id for a single operation.
    OperationId,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);
id_newtype!(
    /// Caller-generated key for at-least-once mutating operations.
    IdempotencyKey,
    is_opaque_token,
    OPAQUE_PATTERN,
    IdDebug::Redacted
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_accept_valid_and_reject_invalid() {
        assert!(RealmId::parse("work").is_ok());
        assert!(NodeId::parse("build-vm").is_ok());
        assert_eq!(RealmId::parse(""), Err(IdError::Empty));
        assert_eq!(RealmId::parse("Work"), Err(IdError::BadShape));
        assert_eq!(WorkloadId::parse("-bad"), Err(IdError::BadShape));
        assert_eq!(NodeId::parse("a_b"), Err(IdError::BadShape));
    }

    #[test]
    fn opaque_tokens_reject_unsafe_shapes() {
        assert!(ExecutionId::parse("exec-abc123").is_ok());
        assert!(ExecutionId::parse("exec_ABC123.4").is_ok());
        assert_eq!(ExecutionId::parse("a b"), Err(IdError::BadShape));
        assert_eq!(ExecutionId::parse("/etc/passwd"), Err(IdError::BadShape));
        assert_eq!(ExecutionId::parse("path..child"), Err(IdError::BadShape));
        assert_eq!(ExecutionId::parse("secret-abc"), Err(IdError::BadShape));
        assert_eq!(
            ExecutionId::parse("bearer.token.abc"),
            Err(IdError::BadShape)
        );
        assert_eq!(ExecutionId::parse("-bad"), Err(IdError::BadShape));
        assert_eq!(StreamId::parse(""), Err(IdError::Empty));
        assert_eq!(
            PrincipalId::parse("x".repeat(MAX_ID_LEN + 1)),
            Err(IdError::TooLong)
        );
    }

    #[test]
    fn deserialize_is_fail_closed() {
        // Valid tokens round-trip.
        assert!(serde_json::from_str::<RealmId>("\"work\"").is_ok());
        assert!(serde_json::from_str::<ExecutionId>("\"exec-1\"").is_ok());
        // Malformed tokens are rejected at decode, not silently accepted.
        assert!(serde_json::from_str::<RealmId>("\"Work\"").is_err());
        assert!(serde_json::from_str::<RealmId>("\"\"").is_err());
        assert!(serde_json::from_str::<NodeId>("\"a_b\"").is_err());
        assert!(serde_json::from_str::<ExecutionId>("\"a b\"").is_err());
        assert!(serde_json::from_str::<ExecutionId>("\"../secret\"").is_err());
        let overlong = format!("\"{}\"", "x".repeat(MAX_ID_LEN + 1));
        assert!(serde_json::from_str::<PrincipalId>(&overlong).is_err());
    }

    #[test]
    fn serialize_is_transparent_string() {
        let id = WorkloadId::parse("build-vm").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"build-vm\"");
    }

    #[test]
    fn debug_redacts_opaque_ids_but_not_labels() {
        let principal = PrincipalId::parse("principal-1").unwrap();
        let principal_debug = format!("{principal:?}");
        assert!(principal_debug.contains("PrincipalId(<11 bytes>)"));
        assert!(!principal_debug.contains("principal-1"));

        let node = NodeId::parse("gateway").unwrap();
        assert_eq!(format!("{node:?}"), "NodeId(\"gateway\")");
    }
}
