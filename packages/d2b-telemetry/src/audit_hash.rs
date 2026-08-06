//! Hash-chain primitives used by the authoritative audit writer.

use sha2::{Digest, Sha256};

const PREFIX: &str = "sha256:";
const HEX_BYTES: usize = 64;

/// A validated SHA-256 digest string.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct AuditHash(String);

impl AuditHash {
    /// Parse the canonical `sha256:<lowercase hex>` representation.
    pub fn parse(value: impl Into<String>) -> Result<Self, AuditHashError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix(PREFIX) else {
            return Err(AuditHashError::BadShape);
        };
        if hex.len() != HEX_BYTES
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(AuditHashError::BadShape);
        }
        Ok(Self(value))
    }

    /// Construct a digest from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(format!("{PREFIX}{}", hex_lower(&Sha256::digest(bytes))))
    }

    /// Borrow the canonical digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for AuditHash {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditHash(<redacted>)")
    }
}

impl<'de> serde::Deserialize<'de> for AuditHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Audit hash parser failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditHashError {
    /// The value did not have the required shape.
    BadShape,
}

impl core::fmt::Display for AuditHashError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("audit-hash-invalid")
    }
}

impl std::error::Error for AuditHashError {}

/// The three hashes that bind one record to its predecessor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditChainLink {
    /// Sequence within the segment.
    pub sequence: u64,
    /// Hash of the previous record.
    pub previous_hash: AuditHash,
    /// Hash of the canonical class payload.
    pub payload_hash: AuditHash,
    /// Hash of the canonical record envelope.
    pub record_hash: AuditHash,
}

impl AuditChainLink {
    /// Construct a chain link from trusted digests.
    pub fn new(
        sequence: u64,
        previous_hash: AuditHash,
        payload_hash: AuditHash,
        record_hash: AuditHash,
    ) -> Self {
        Self {
            sequence,
            previous_hash,
            payload_hash,
            record_hash,
        }
    }

    /// Verify a link against recomputed values.
    pub fn verify(
        &self,
        previous_hash: &AuditHash,
        payload_hash: &AuditHash,
        record_hash: &AuditHash,
    ) -> Result<(), ChainVerificationError> {
        if &self.previous_hash != previous_hash {
            return Err(ChainVerificationError::PreviousHashMismatch);
        }
        if &self.payload_hash != payload_hash {
            return Err(ChainVerificationError::PayloadHashMismatch);
        }
        if &self.record_hash != record_hash {
            return Err(ChainVerificationError::RecordHashMismatch);
        }
        Ok(())
    }

    /// Verify a link, including its expected sequence number.
    ///
    /// The shorter [`Self::verify`] method intentionally remains available
    /// for callers that do not have a segment sequence. Export and replay
    /// paths should use this method when they do.
    pub fn verify_at(
        &self,
        expected_sequence: u64,
        previous_hash: &AuditHash,
        payload_hash: &AuditHash,
        record_hash: &AuditHash,
    ) -> Result<(), ChainVerificationError> {
        if self.sequence != expected_sequence {
            return Err(ChainVerificationError::SequenceMismatch);
        }
        self.verify(previous_hash, payload_hash, record_hash)
    }
}

/// A chain verification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainVerificationError {
    /// The predecessor link changed.
    PreviousHashMismatch,
    /// The class payload changed.
    PayloadHashMismatch,
    /// The complete envelope changed.
    RecordHashMismatch,
    /// The sequence was not the next sequence.
    SequenceMismatch,
}

impl core::fmt::Display for ChainVerificationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::PreviousHashMismatch => "audit-chain-previous-hash-mismatch",
            Self::PayloadHashMismatch => "audit-chain-payload-hash-mismatch",
            Self::RecordHashMismatch => "audit-chain-record-hash-mismatch",
            Self::SequenceMismatch => "audit-chain-sequence-mismatch",
        })
    }
}

impl std::error::Error for ChainVerificationError {}

/// Compute a payload digest.
pub fn payload_hash(bytes: &[u8]) -> AuditHash {
    AuditHash::from_bytes(bytes)
}

/// Compute the envelope digest from the predecessor and canonical envelope.
pub fn record_hash(previous: &AuditHash, canonical_envelope: &[u8]) -> AuditHash {
    let mut hasher = Sha256::new();
    hasher.update(previous.as_str().as_bytes());
    hasher.update(canonical_envelope);
    AuditHash(format!("{PREFIX}{}", hex_lower(&hasher.finalize())))
}

/// A deterministic genesis marker.
pub fn genesis_hash() -> AuditHash {
    AuditHash::from_bytes(b"d2b-audit-v3-genesis")
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_have_one_canonical_shape() {
        let hash = AuditHash::from_bytes(b"value");
        assert!(AuditHash::parse(hash.as_str()).is_ok());
        assert_eq!(format!("{hash:?}"), "AuditHash(<redacted>)");
        assert!(AuditHash::parse("sha256:ABC").is_err());
    }

    #[test]
    fn record_hash_changes_when_predecessor_changes() {
        let first = genesis_hash();
        let second = AuditHash::from_bytes(b"other");
        assert_ne!(
            record_hash(&first, b"payload"),
            record_hash(&second, b"payload")
        );
    }

    #[test]
    fn chain_link_verification_checks_sequence_when_supplied() {
        let previous = genesis_hash();
        let payload = payload_hash(b"payload");
        let record = record_hash(&previous, b"record");
        let link = AuditChainLink::new(2, previous.clone(), payload.clone(), record.clone());
        assert_eq!(
            link.verify_at(1, &previous, &payload, &record),
            Err(ChainVerificationError::SequenceMismatch)
        );
        assert!(link.verify_at(2, &previous, &payload, &record).is_ok());
    }
}
