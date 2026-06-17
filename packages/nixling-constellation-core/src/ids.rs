//! Strongly-typed identifiers for the constellation model (ADR 0032).
//!
//! Two families:
//!
//! - **Label-shaped** ids (`RealmId` labels, [`NodeId`], [`WorkloadId`],
//!   [`ProviderId`]) reuse the nixling lowercase label shape
//!   `^[a-z][a-z0-9-]*$`.
//! - **Opaque** ids ([`GatewayId`], [`ExecutionId`], [`StreamId`],
//!   [`PrincipalId`], [`OperationId`], [`IdempotencyKey`]) are bounded,
//!   non-empty, printable-ASCII tokens.
//!
//! All constructors validate; malformed input is rejected with
//! [`IdError`] rather than silently accepted (fail-closed).

use serde::{Deserialize, Serialize};

/// Maximum length for any identifier token.
pub const MAX_ID_LEN: usize = 128;

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

/// True for the nixling lowercase label shape `^[a-z][a-z0-9-]*$`.
pub fn is_label(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// True for a bounded, non-empty printable-ASCII opaque token (no spaces
/// or control characters).
fn is_opaque_token(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_graphic() && c != ' ')
}

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident, $validate:expr) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
            schemars::JsonSchema,
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
    };
}

id_newtype!(
    /// A single realm label. A full realm path is a `.`-joined sequence
    /// of these; see [`crate::realm::RealmPath`].
    RealmId,
    is_label
);
id_newtype!(
    /// A node within a realm (a host, gateway, or provider-managed node).
    NodeId,
    is_label
);
id_newtype!(
    /// A workload (VM, session, or sandbox) on a node.
    WorkloadId,
    is_label
);
id_newtype!(
    /// A provider implementation id.
    ProviderId,
    is_label
);
id_newtype!(
    /// A realm gateway guest identity. Opaque (not operator-typed).
    GatewayId,
    is_opaque_token
);
id_newtype!(
    /// A durable execution id.
    ExecutionId,
    is_opaque_token
);
id_newtype!(
    /// A multiplexed stream id within a peer session.
    StreamId,
    is_opaque_token
);
id_newtype!(
    /// An authenticated principal (never a relay credential).
    PrincipalId,
    is_opaque_token
);
id_newtype!(
    /// Audit/correlation id for a single operation.
    OperationId,
    is_opaque_token
);
id_newtype!(
    /// Caller-generated key for at-least-once mutating operations.
    IdempotencyKey,
    is_opaque_token
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
    fn opaque_tokens_reject_spaces_and_control() {
        assert!(ExecutionId::parse("exec-abc123").is_ok());
        assert_eq!(ExecutionId::parse("a b"), Err(IdError::BadShape));
        assert_eq!(StreamId::parse(""), Err(IdError::Empty));
        assert_eq!(
            PrincipalId::parse("x".repeat(MAX_ID_LEN + 1)),
            Err(IdError::TooLong)
        );
    }
}
