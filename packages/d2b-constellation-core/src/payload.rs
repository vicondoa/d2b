//! A bounded, redaction-safe opaque byte payload (ADR 0032). Operation
//! bodies and stream-data chunks are opaque to the routing/audit layer;
//! this newtype bounds their length at decode and ensures `Debug` reveals
//! only the length, never the bytes (no payload can leak through a log or
//! audit line).

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{ArrayValidation, InstanceType, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum length of an [`OpaquePayload`]. This is a semantic safety cap
/// (a codec also enforces the negotiated wire frame cap as defense in
/// depth).
pub const MAX_PAYLOAD_LEN: usize = 1 << 20; // 1 MiB

/// A bounded, opaque byte payload. Decoding rejects anything larger than
/// [`MAX_PAYLOAD_LEN`] (fail-closed); `Debug` shows only the length.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OpaquePayload(Vec<u8>);

impl OpaquePayload {
    /// Build a payload, rejecting anything over [`MAX_PAYLOAD_LEN`].
    pub fn new(bytes: Vec<u8>) -> Result<Self, PayloadTooLarge> {
        if bytes.len() > MAX_PAYLOAD_LEN {
            Err(PayloadTooLarge)
        } else {
            Ok(Self(bytes))
        }
    }

    /// An empty payload.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Borrow the bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The payload length.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A payload exceeded [`MAX_PAYLOAD_LEN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadTooLarge;

impl core::fmt::Display for PayloadTooLarge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "payload exceeds {MAX_PAYLOAD_LEN} bytes")
    }
}

impl std::error::Error for PayloadTooLarge {}

// Redacting Debug: never print the bytes.
impl core::fmt::Debug for OpaquePayload {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OpaquePayload(<{} bytes>)", self.0.len())
    }
}

// Fail-closed decode: a frame larger than the cap is rejected before it is
// retained.
impl<'de> Deserialize<'de> for OpaquePayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Self::new(bytes).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for OpaquePayload {
    fn schema_name() -> String {
        "OpaquePayload".to_owned()
    }

    fn json_schema(r#gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Array))),
            array: Some(Box::new(ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(r#gen.subschema_for::<u8>()))),
                max_items: Some(MAX_PAYLOAD_LEN as u32),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_payload() {
        assert!(OpaquePayload::new(vec![0u8; 10]).is_ok());
        assert!(OpaquePayload::new(vec![0u8; MAX_PAYLOAD_LEN + 1]).is_err());
    }

    #[test]
    fn debug_redacts_bytes() {
        let p = OpaquePayload::new(b"secret-bytes".to_vec()).unwrap();
        let dbg = format!("{p:?}");
        assert!(dbg.contains("12 bytes"));
        assert!(!dbg.contains("secret"));
    }

    #[test]
    fn deserialize_is_fail_closed() {
        // a small array round-trips
        assert!(serde_json::from_str::<OpaquePayload>("[1,2,3]").is_ok());
    }

    #[test]
    fn deserialize_rejects_oversized_payload() {
        // An over-cap byte array is rejected at decode, before retention.
        let oversized = serde_json::to_value(vec![0u8; MAX_PAYLOAD_LEN + 1]).unwrap();
        assert!(serde_json::from_value::<OpaquePayload>(oversized).is_err());
    }
}
