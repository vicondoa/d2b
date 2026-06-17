//! A bounded, low-cardinality protocol token (ADR 0032) — used for the
//! handshake `codec_id` and the peer-context `auth_mechanism`. Both are
//! peer-controlled, so they are bounded + shape-checked at decode and
//! their `Debug` only ever prints the bounded token (never unbounded peer
//! input).

use schemars::{
    gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
    JsonSchema,
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum length of a [`ProtocolToken`].
pub const MAX_PROTOCOL_TOKEN_LEN: usize = 64;

/// ECMA-regex for a bounded printable-ASCII token (no spaces).
const TOKEN_PATTERN: &str = "^[\\x21-\\x7e]+$";

/// A bounded, non-empty, printable-ASCII protocol token.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProtocolToken(String);

/// Reason a [`ProtocolToken`] failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    /// Empty token.
    Empty,
    /// Token exceeded [`MAX_PROTOCOL_TOKEN_LEN`].
    TooLong,
    /// Token contained a space or non-printable byte.
    BadShape,
}

impl core::fmt::Display for TokenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TokenError::Empty => write!(f, "protocol token is empty"),
            TokenError::TooLong => {
                write!(f, "protocol token exceeds {MAX_PROTOCOL_TOKEN_LEN} bytes")
            }
            TokenError::BadShape => write!(f, "protocol token has an invalid shape"),
        }
    }
}

impl std::error::Error for TokenError {}

impl ProtocolToken {
    /// Validate and construct (fail-closed).
    pub fn parse(raw: impl Into<String>) -> Result<Self, TokenError> {
        let s = raw.into();
        if s.is_empty() {
            return Err(TokenError::Empty);
        }
        if s.len() > MAX_PROTOCOL_TOKEN_LEN {
            return Err(TokenError::TooLong);
        }
        if !s.chars().all(|c| c.is_ascii_graphic() && c != ' ') {
            return Err(TokenError::BadShape);
        }
        Ok(Self(s))
    }

    /// Borrow the token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ProtocolToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProtocolToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ProtocolToken {
    fn schema_name() -> String {
        "ProtocolToken".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_PROTOCOL_TOKEN_LEN as u32),
                min_length: Some(1),
                pattern: Some(TOKEN_PATTERN.to_owned()),
            })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_decode_are_fail_closed() {
        assert!(ProtocolToken::parse("protobuf.v1").is_ok());
        assert_eq!(ProtocolToken::parse(""), Err(TokenError::Empty));
        assert_eq!(
            ProtocolToken::parse("x".repeat(MAX_PROTOCOL_TOKEN_LEN + 1)),
            Err(TokenError::TooLong)
        );
        assert_eq!(ProtocolToken::parse("a b"), Err(TokenError::BadShape));
        // decode path is bounded too
        assert!(serde_json::from_str::<ProtocolToken>("\"protobuf.v1\"").is_ok());
        let overlong = format!("\"{}\"", "x".repeat(MAX_PROTOCOL_TOKEN_LEN + 1));
        assert!(serde_json::from_str::<ProtocolToken>(&overlong).is_err());
    }
}
