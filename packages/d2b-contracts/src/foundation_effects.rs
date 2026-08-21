//! Neutral credential effect-seam contracts.
//!
//! These values are shared only by the foundation ACA effect adapter and
//! provider credential contracts. They contain no provider implementation
//! behavior beyond the frozen opaque-value validation and redaction rules.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

/// Maximum bytes accepted as an opaque non-secret cloud reference.
pub const MAX_AZURE_REF_BYTES: usize = 128;
/// Maximum bytes accepted as a Provider lease handle before one-way encoding.
pub const MAX_CREDENTIAL_LEASE_HANDLE_BYTES: usize = 256;

const OPAQUE_DIGEST_BYTES: usize = 71;
const OPAQUE_DIGEST_PREFIX: &str = "sha256:";

/// Validation failure for a Credential base contract.
///
/// The variants deliberately carry no caller-controlled data, resource identity,
/// or credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialContractError {
    /// An opaque value was empty, over its bound, or used a rejected character.
    InvalidOpaqueValue,
    /// Status timestamps or state fields conflict.
    InvalidStatus,
}

impl core::fmt::Display for CredentialContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidOpaqueValue => "credential opaque value is invalid",
            Self::InvalidStatus => "credential status is invalid",
        })
    }
}

impl std::error::Error for CredentialContractError {}

fn validate_opaque_source(value: &str, max_bytes: usize) -> Result<(), CredentialContractError> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
        return Err(CredentialContractError::InvalidOpaqueValue);
    }
    Ok(())
}

fn opaque_digest(domain: &[u8], value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    hasher.update(value.as_bytes());
    format!("{OPAQUE_DIGEST_PREFIX}{:x}", hasher.finalize())
}

fn validate_opaque_digest(value: &str) -> Result<(), CredentialContractError> {
    let Some(hex) = value.strip_prefix(OPAQUE_DIGEST_PREFIX) else {
        return Err(CredentialContractError::InvalidOpaqueValue);
    };
    if value.len() != OPAQUE_DIGEST_BYTES
        || hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(CredentialContractError::InvalidOpaqueValue);
    }
    Ok(())
}

fn opaque_digest_schema() -> schemars::schema::Schema {
    let mut schema = schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::String,
        ))),
        ..Default::default()
    };
    schema.string().min_length = Some(OPAQUE_DIGEST_BYTES as u32);
    schema.string().max_length = Some(OPAQUE_DIGEST_BYTES as u32);
    schema.string().pattern = Some("^sha256:[0-9a-f]{64}$".to_owned());
    schemars::schema::Schema::Object(schema)
}

macro_rules! opaque_credential_value {
    ($name:ident, $max:expr, $domain:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate a raw identifier and retain only its domain-separated digest.
            pub fn parse(value: impl AsRef<str>) -> Result<Self, CredentialContractError> {
                let value = value.as_ref();
                validate_opaque_source(value, $max)?;
                Ok(Self(opaque_digest($domain, value)))
            }

            /// Borrow the non-reversible representation used on authorized wires.
            pub fn as_opaque_str(&self) -> &str {
                &self.0
            }

            /// Reconstruct a value from its authorized one-way wire representation.
            pub fn from_opaque_digest(
                value: impl Into<String>,
            ) -> Result<Self, CredentialContractError> {
                let value = value.into();
                validate_opaque_digest(&value)?;
                Ok(Self(value))
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::from_opaque_digest(String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(
                _gen: &mut schemars::r#gen::SchemaGenerator,
            ) -> schemars::schema::Schema {
                opaque_digest_schema()
            }
        }
    };
}

/// A bounded non-secret cloud identifier whose diagnostics are always redacted.
///
/// Providers need the validated tenant, client, or region value when calling
/// their backing service, so serialization preserves it instead of hashing it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OpaqueAzureRef(String);

impl OpaqueAzureRef {
    /// Validate and preserve a bare non-secret cloud identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialContractError> {
        let value = value.into();
        validate_opaque_source(&value, MAX_AZURE_REF_BYTES)?;
        Ok(Self(value))
    }

    /// Borrow the validated identifier for Provider use.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for OpaqueAzureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OpaqueAzureRef(<redacted>)")
    }
}

impl core::fmt::Display for OpaqueAzureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OpaqueAzureRef(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for OpaqueAzureRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for OpaqueAzureRef {
    fn schema_name() -> String {
        "OpaqueAzureRef".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().min_length = Some(1);
        schema.string().max_length = Some(MAX_AZURE_REF_BYTES as u32);
        schema.string().pattern = Some("^[A-Za-z0-9._-]+$".to_owned());
        schemars::schema::Schema::Object(schema)
    }
}

opaque_credential_value!(
    CredentialLeaseHandle,
    MAX_CREDENTIAL_LEASE_HANDLE_BYTES,
    b"d2b:v3:credential-lease-handle",
    "A bounded non-authorizing lease handle represented only by a one-way digest."
);
