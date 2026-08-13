//! Stable, opaque operation identity shared by audit durability domains.

use crate::hash_chain::{AuditHash, is_canonical_digest};
use serde::ser::SerializeStruct;

const MAX_OPERATION_ID_BYTES: usize = 256;
const MAX_ZONE_ID_BYTES: usize = 256;
// These domains are also used by the broker wire's authoritative join
// derivation.  Keeping the derivation here identical is what lets a store
// outbox, broker envelope, and terminal evidence use one key.
const OPERATION_ID_DOMAIN: &[u8] = b"d2b:broker-operation:v2";
const ZONE_ID_DOMAIN: &[u8] = b"d2b:broker-zone:v2";
const OPAQUE_DIGEST_DOMAIN: &[u8] = b"d2b:opaque-digest:v1:";

/// A durable operation identity.
///
/// Callers may use any bounded request token, but the value that crosses an
/// audit boundary is always a canonical digest. This keeps attacker-controlled
/// operation text out of audit output while allowing independent durability
/// domains to join the same operation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationIdentity(String);

/// A canonical, opaque Zone identity used by cross-domain joins.
///
/// Zone names are never retained in a join key. A caller may provide an
/// already canonical digest (for example, one read from a redacted audit
/// record), in which case it is preserved.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(String);

/// The single join key shared by broker records and resource outboxes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneOperationKey {
    zone: ZoneId,
    operation: OperationIdentity,
}

impl serde::Serialize for OperationIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for OperationIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl OperationIdentity {
    /// Derive an identity from the request token.
    pub fn derive(value: &str) -> Result<Self, OperationIdentityError> {
        if value.is_empty()
            || value.len() > MAX_OPERATION_ID_BYTES
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(OperationIdentityError::Invalid);
        }
        if let Ok(identity) = Self::parse(value) {
            return Ok(identity);
        }
        let mut bytes = Vec::with_capacity(OPERATION_ID_DOMAIN.len() + 1 + value.len());
        bytes.extend_from_slice(OPERATION_ID_DOMAIN);
        bytes.push(0);
        bytes.extend_from_slice(value.as_bytes());
        Ok(Self(AuditHash::from_bytes(&bytes).as_str().to_owned()))
    }

    /// Parse a previously derived canonical identity.
    pub fn parse(value: &str) -> Result<Self, OperationIdentityError> {
        AuditHash::parse(value.to_owned())
            .map(|hash| Self(hash.as_str().to_owned()))
            .map_err(|_| OperationIdentityError::Invalid)
    }

    /// Borrow the canonical digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ZoneId {
    /// Derive an opaque Zone identity from a trusted Zone name.
    pub fn derive(value: &str) -> Result<Self, OperationIdentityError> {
        if value.is_empty()
            || value.len() > MAX_ZONE_ID_BYTES
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() || matches!(byte, b'/' | b'\\'))
        {
            return Err(OperationIdentityError::Invalid);
        }
        if is_canonical_digest(value) {
            return Ok(Self(value.to_owned()));
        }
        let mut bytes = Vec::with_capacity(ZONE_ID_DOMAIN.len() + 1 + value.len());
        bytes.extend_from_slice(ZONE_ID_DOMAIN);
        bytes.push(0);
        bytes.extend_from_slice(value.as_bytes());
        Ok(Self(AuditHash::from_bytes(&bytes).as_str().to_owned()))
    }

    /// Parse an opaque Zone identity from a durable record.
    pub fn parse(value: &str) -> Result<Self, OperationIdentityError> {
        if !is_canonical_digest(value) {
            return Err(OperationIdentityError::Invalid);
        }
        Ok(Self(value.to_owned()))
    }

    /// Borrow the canonical digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ZoneOperationKey {
    /// Construct a join key from a Zone name and request token.
    pub fn derive(zone: &str, operation: &str) -> Result<Self, OperationIdentityError> {
        Ok(Self {
            zone: ZoneId::derive(zone)?,
            operation: OperationIdentity::derive(operation)?,
        })
    }

    /// Construct a join key from canonical components.
    pub const fn new(zone: ZoneId, operation: OperationIdentity) -> Self {
        Self { zone, operation }
    }

    /// Borrow the Zone component.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the operation component.
    pub const fn operation(&self) -> &OperationIdentity {
        &self.operation
    }
}

impl serde::Serialize for ZoneId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ZoneId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl serde::Serialize for ZoneOperationKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut output = serializer.serialize_struct("ZoneOperationKey", 2)?;
        output.serialize_field("zone", &self.zone)?;
        output.serialize_field("operation", &self.operation)?;
        output.end()
    }
}

impl<'de> serde::Deserialize<'de> for ZoneOperationKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            zone: ZoneId,
            operation: OperationIdentity,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(wire.zone, wire.operation))
    }
}

impl core::fmt::Debug for ZoneId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneId(<redacted>)")
    }
}

impl core::fmt::Debug for ZoneOperationKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneOperationKey(<redacted>)")
    }
}

impl core::fmt::Debug for OperationIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("OperationIdentity(<redacted>)")
    }
}

impl core::fmt::Display for OperationIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Operation identity validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationIdentityError {
    /// The token was empty, oversized, malformed, or contained control text.
    Invalid,
}

impl core::fmt::Display for OperationIdentityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("operation-identity-invalid")
    }
}

impl std::error::Error for OperationIdentityError {}

/// Derive a stable opaque digest for an arbitrary bounded identity value.
///
/// This helper is used for fields such as Zone, subject, target, and
/// correlation values that must remain joinable without retaining raw text.
pub fn opaque_identity(value: &str) -> String {
    let mut bytes = Vec::with_capacity(OPAQUE_DIGEST_DOMAIN.len() + value.len());
    bytes.extend_from_slice(OPAQUE_DIGEST_DOMAIN);
    bytes.extend_from_slice(value.as_bytes());
    AuditHash::from_bytes(&bytes).as_str().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_identity_is_stable_and_does_not_echo_input() {
        let first = OperationIdentity::derive("attacker-operation-canary").unwrap();
        let second = OperationIdentity::derive("attacker-operation-canary").unwrap();
        assert_eq!(first, second);
        assert!(first.as_str().starts_with("sha256:"));
        assert!(!first.as_str().contains("attacker-operation-canary"));
        assert_eq!(OperationIdentity::parse(first.as_str()).unwrap(), first);
        assert_eq!(format!("{first:?}"), "OperationIdentity(<redacted>)");
    }

    #[test]
    fn invalid_tokens_fail_closed() {
        assert!(OperationIdentity::derive("").is_err());
        assert!(OperationIdentity::derive("line\nbreak").is_err());
        assert!(OperationIdentity::parse("operation").is_err());
    }

    #[test]
    fn zone_operation_key_is_zone_scoped_and_preserves_canonical_digests() {
        let work = ZoneOperationKey::derive("work", "same-token").unwrap();
        let personal = ZoneOperationKey::derive("personal", "same-token").unwrap();
        assert_ne!(work, personal);
        assert!(work.zone().as_str().starts_with("sha256:"));
        assert_eq!(
            ZoneId::parse(work.zone().as_str()).unwrap().as_str(),
            work.zone().as_str()
        );
        assert_eq!(
            ZoneOperationKey::derive(work.zone().as_str(), work.operation().as_str()).unwrap(),
            work
        );
    }

    #[test]
    fn canonical_digest_shape_is_exact() {
        assert!(ZoneId::parse(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(ZoneId::parse("sha256:ABC").is_err());
        assert!(ZoneId::parse(&format!("sha256:{}", "a".repeat(63))).is_err());
    }

    #[test]
    fn join_derivation_matches_the_broker_wire_domain_vectors() {
        let key = ZoneOperationKey::derive("work", "operation").unwrap();
        assert_eq!(
            key.zone().as_str(),
            "sha256:61526c1fbd92a0f0beb3ec966f196f88ba00643f8266b6a69205038a1d0431ae"
        );
        assert_eq!(
            key.operation().as_str(),
            "sha256:484e31dca7cec89e3bebdd36b2be30259a30de5f909049984ab165c71a2da81b"
        );
    }
}
