//! Frozen `d2bval/v1` canonical-JSON value frame.

use d2b_contracts::v3::CanonicalJsonValue;

/// `d2bval/v1` format version.
pub const VALUE_FORMAT_VERSION: u8 = 1;
/// Maximum bytes in one complete encoded value frame.
pub const MAX_ENCODED_VALUE_BYTES: usize = 1024 * 1024;
const VALUE_HEADER_BYTES: usize = 7;
/// Maximum canonical JSON payload bytes after the frame header.
pub const MAX_VALUE_PAYLOAD_BYTES: usize = MAX_ENCODED_VALUE_BYTES - VALUE_HEADER_BYTES;

/// Closed table value-kind discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum ValueKind {
    StoreMetaScalar = 0x0001,
    ApiSchemaRecord = 0x0002,
    ResourceRecord = 0x0003,
    TypeIndexRecord = 0x0004,
    OwnerIndexRecord = 0x0005,
    ProducerIndexRecord = 0x0006,
    ControllerIndexRecord = 0x0007,
    ChangeBatch = 0x0008,
    OperationRecord = 0x0009,
    ZoneLinkCursor = 0x000a,
}

impl ValueKind {
    pub const fn from_discriminant(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::StoreMetaScalar),
            0x0002 => Some(Self::ApiSchemaRecord),
            0x0003 => Some(Self::ResourceRecord),
            0x0004 => Some(Self::TypeIndexRecord),
            0x0005 => Some(Self::OwnerIndexRecord),
            0x0006 => Some(Self::ProducerIndexRecord),
            0x0007 => Some(Self::ControllerIndexRecord),
            0x0008 => Some(Self::ChangeBatch),
            0x0009 => Some(Self::OperationRecord),
            0x000a => Some(Self::ZoneLinkCursor),
            _ => None,
        }
    }

    pub const fn discriminant(self) -> u16 {
        self as u16
    }
}

/// Validated encoded value frame.
#[derive(Clone, PartialEq, Eq)]
pub struct EncodedValue(Vec<u8>);

impl EncodedValue {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl core::fmt::Debug for EncodedValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = self
            .0
            .get(1..3)
            .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
            .and_then(|bytes| ValueKind::from_discriminant(u16::from_be_bytes(bytes)));
        let mut debug = f.debug_struct("EncodedValue");
        match kind {
            Some(kind) => debug.field("kind", &kind),
            None => debug.field("kind", &"<invalid>"),
        };
        debug
            .field(
                "payload_byte_length",
                &self.0.len().saturating_sub(VALUE_HEADER_BYTES),
            )
            .finish()
    }
}

/// Validated decoded value frame.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedValue {
    kind: ValueKind,
    canonical_json: Vec<u8>,
}

impl DecodedValue {
    pub const fn kind(&self) -> ValueKind {
        self.kind
    }

    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ValueCodecError> {
        if bytes.len() < VALUE_HEADER_BYTES {
            return Err(ValueCodecError::Truncated);
        }
        if bytes[0] != VALUE_FORMAT_VERSION {
            return Err(ValueCodecError::UnknownVersion);
        }
        let kind = ValueKind::from_discriminant(u16::from_be_bytes([bytes[1], bytes[2]]))
            .ok_or(ValueCodecError::UnknownValueKind)?;
        let declared_length = u32::from_be_bytes(
            bytes[3..7]
                .try_into()
                .map_err(|_| ValueCodecError::Truncated)?,
        );
        let declared_length =
            usize::try_from(declared_length).map_err(|_| ValueCodecError::ValueTooLong)?;
        if declared_length > MAX_VALUE_PAYLOAD_BYTES {
            return Err(ValueCodecError::ValueTooLong);
        }
        let payload = &bytes[VALUE_HEADER_BYTES..];
        if payload.len() != declared_length {
            return Err(ValueCodecError::LengthMismatch);
        }
        validate_canonical_payload(payload)?;
        Ok(Self {
            kind,
            canonical_json: payload.to_vec(),
        })
    }
}

impl core::fmt::Debug for DecodedValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodedValue")
            .field("kind", &self.kind)
            .field("payload_byte_length", &self.canonical_json.len())
            .finish()
    }
}

/// Encode one canonical JSON payload in its table-specific value frame.
pub fn encode_value(
    kind: ValueKind,
    canonical_json: &[u8],
) -> Result<EncodedValue, ValueCodecError> {
    if canonical_json.len() > MAX_VALUE_PAYLOAD_BYTES {
        return Err(ValueCodecError::ValueTooLong);
    }
    validate_canonical_payload(canonical_json)?;
    let length = u32::try_from(canonical_json.len()).map_err(|_| ValueCodecError::ValueTooLong)?;
    let mut encoded = Vec::with_capacity(VALUE_HEADER_BYTES + canonical_json.len());
    encoded.push(VALUE_FORMAT_VERSION);
    encoded.extend_from_slice(&kind.discriminant().to_be_bytes());
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(canonical_json);
    Ok(EncodedValue(encoded))
}

fn validate_canonical_payload(payload: &[u8]) -> Result<(), ValueCodecError> {
    let parsed =
        CanonicalJsonValue::parse(payload).map_err(|_| ValueCodecError::NonCanonicalPayload)?;
    if parsed.to_canonical_bytes() != payload {
        return Err(ValueCodecError::NonCanonicalPayload);
    }
    Ok(())
}

/// Fail-closed value decode or encode reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueCodecError {
    UnknownVersion,
    UnknownValueKind,
    ValueTooLong,
    LengthMismatch,
    NonCanonicalPayload,
    Truncated,
}

impl core::fmt::Display for ValueCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnknownVersion => "unknown d2bval format version",
            Self::UnknownValueKind => "unknown d2bval value kind",
            Self::ValueTooLong => "d2bval payload exceeds its bound",
            Self::LengthMismatch => "d2bval payload length mismatch",
            Self::NonCanonicalPayload => "d2bval payload is not canonical JSON",
            Self::Truncated => "truncated d2bval frame",
        })
    }
}

impl std::error::Error for ValueCodecError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_literal_golden_vectors_pin_every_value_kind() {
        let vectors: &[(ValueKind, &[u8])] = &[
            (
                ValueKind::StoreMetaScalar,
                b"\x01\x00\x01\x00\x00\x00\x02{}",
            ),
            (
                ValueKind::ApiSchemaRecord,
                b"\x01\x00\x02\x00\x00\x00\x02{}",
            ),
            (ValueKind::ResourceRecord, b"\x01\x00\x03\x00\x00\x00\x02{}"),
            (
                ValueKind::TypeIndexRecord,
                b"\x01\x00\x04\x00\x00\x00\x02{}",
            ),
            (
                ValueKind::OwnerIndexRecord,
                b"\x01\x00\x05\x00\x00\x00\x02{}",
            ),
            (
                ValueKind::ProducerIndexRecord,
                b"\x01\x00\x06\x00\x00\x00\x02{}",
            ),
            (
                ValueKind::ControllerIndexRecord,
                b"\x01\x00\x07\x00\x00\x00\x02{}",
            ),
            (ValueKind::ChangeBatch, b"\x01\x00\x08\x00\x00\x00\x02{}"),
            (
                ValueKind::OperationRecord,
                b"\x01\x00\x09\x00\x00\x00\x02{}",
            ),
            (ValueKind::ZoneLinkCursor, b"\x01\x00\x0a\x00\x00\x00\x02{}"),
        ];

        for (kind, literal) in vectors {
            assert_eq!(encode_value(*kind, b"{}").unwrap().as_bytes(), *literal);
            let decoded = DecodedValue::decode(literal).unwrap();
            assert_eq!(decoded.kind(), *kind);
            assert_eq!(decoded.canonical_json(), b"{}");
        }
    }

    #[test]
    fn decode_rejects_unknown_truncated_mismatched_and_noncanonical_frames() {
        assert_eq!(DecodedValue::decode(&[]), Err(ValueCodecError::Truncated));
        assert_eq!(
            DecodedValue::decode(b"\x02\x00\x01\x00\x00\x00\x02{}"),
            Err(ValueCodecError::UnknownVersion)
        );
        assert_eq!(
            DecodedValue::decode(b"\x01\x00\x0b\x00\x00\x00\x02{}"),
            Err(ValueCodecError::UnknownValueKind)
        );
        assert_eq!(
            DecodedValue::decode(b"\x01\x00\x01\x00\x00\x00\x03{}"),
            Err(ValueCodecError::LengthMismatch)
        );
        assert_eq!(
            DecodedValue::decode(b"\x01\x00\x01\x00\x00\x00\x02[]"),
            Ok(DecodedValue {
                kind: ValueKind::StoreMetaScalar,
                canonical_json: b"[]".to_vec(),
            })
        );
        assert_eq!(
            DecodedValue::decode(b"\x01\x00\x01\x00\x00\x00\x09{\"b\":0} \n"),
            Err(ValueCodecError::NonCanonicalPayload)
        );
        assert_eq!(
            DecodedValue::decode(b"\x01\x00\x01\x00\x10\x00\x01"),
            Err(ValueCodecError::ValueTooLong)
        );
    }

    #[test]
    fn encode_rejects_derived_or_oversized_payloads() {
        assert_eq!(
            encode_value(ValueKind::ResourceRecord, br#"{ "a": 1 }"#),
            Err(ValueCodecError::NonCanonicalPayload)
        );
        assert_eq!(
            encode_value(
                ValueKind::ResourceRecord,
                &vec![b'x'; MAX_VALUE_PAYLOAD_BYTES + 1]
            ),
            Err(ValueCodecError::ValueTooLong)
        );
    }

    #[test]
    fn value_debug_redacts_canonical_json() {
        const MARKER: &str = "debug-leak-sentinel-value";
        let payload = format!(r#"{{"marker":"{MARKER}"}}"#);
        let encoded = encode_value(ValueKind::ResourceRecord, payload.as_bytes()).unwrap();
        let decoded = DecodedValue::decode(encoded.as_bytes()).unwrap();
        let encoded_debug = format!("{encoded:?}");
        let decoded_debug = format!("{decoded:?}");

        assert!(
            !encoded_debug.contains(MARKER),
            "encoded value Debug exposed canonical JSON"
        );
        assert!(
            !decoded_debug.contains(MARKER),
            "decoded value Debug exposed canonical JSON"
        );
        for diagnostic in [encoded_debug, decoded_debug] {
            assert!(diagnostic.contains("ResourceRecord"));
            assert!(diagnostic.contains(&payload.len().to_string()));
        }
    }
}
