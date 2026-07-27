//! Frozen `d2bkey/v1` table-key codec.

/// `d2bkey/v1` format version.
pub const KEY_FORMAT_VERSION: u8 = 1;
/// Maximum components after the key-space discriminant.
pub const MAX_KEY_COMPONENTS: usize = 4;
/// Maximum bytes in one text component.
pub const MAX_TEXT_COMPONENT_BYTES: usize = 512;
/// Maximum bytes in one complete encoded key.
pub const MAX_ENCODED_KEY_BYTES: usize = 1024;

/// Closed table key-space discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum KeySpace {
    StoreMeta = 0x01,
    ApiSchemas = 0x02,
    Resources = 0x03,
    TypeIndex = 0x04,
    OwnerIndex = 0x05,
    ProducerIndex = 0x06,
    ControllerIndex = 0x07,
    RevisionLog = 0x08,
    Operations = 0x09,
    ZoneLinkCursors = 0x0a,
}

impl KeySpace {
    pub const fn from_discriminant(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::StoreMeta),
            0x02 => Some(Self::ApiSchemas),
            0x03 => Some(Self::Resources),
            0x04 => Some(Self::TypeIndex),
            0x05 => Some(Self::OwnerIndex),
            0x06 => Some(Self::ProducerIndex),
            0x07 => Some(Self::ControllerIndex),
            0x08 => Some(Self::RevisionLog),
            0x09 => Some(Self::Operations),
            0x0a => Some(Self::ZoneLinkCursors),
            _ => None,
        }
    }

    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    const fn component_shape(self) -> &'static [KeyComponentKind] {
        match self {
            Self::StoreMeta | Self::ApiSchemas | Self::Operations | Self::ZoneLinkCursors => {
                &[KeyComponentKind::Text]
            }
            Self::Resources | Self::TypeIndex | Self::OwnerIndex | Self::ProducerIndex => {
                &[KeyComponentKind::Text, KeyComponentKind::Text]
            }
            Self::ControllerIndex => &[
                KeyComponentKind::Text,
                KeyComponentKind::Text,
                KeyComponentKind::Text,
            ],
            Self::RevisionLog => &[KeyComponentKind::U64],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyComponentKind {
    Text,
    U64,
}

/// Borrowed key component accepted by the encoder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyComponent<'a> {
    Text(&'a str),
    U64(u64),
}

impl core::fmt::Debug for KeyComponent<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (kind, byte_length) = match self {
            Self::Text(text) => ("Text", text.len()),
            Self::U64(_) => ("U64", core::mem::size_of::<u64>()),
        };
        f.debug_struct("KeyComponent")
            .field("kind", &kind)
            .field("byte_length", &byte_length)
            .finish()
    }
}

/// Owned component returned by the decoder.
#[derive(Clone, PartialEq, Eq)]
pub enum DecodedKeyComponent {
    Text(String),
    U64(u64),
}

impl core::fmt::Debug for DecodedKeyComponent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (kind, byte_length) = match self {
            Self::Text(text) => ("Text", text.len()),
            Self::U64(_) => ("U64", core::mem::size_of::<u64>()),
        };
        f.debug_struct("DecodedKeyComponent")
            .field("kind", &kind)
            .field("byte_length", &byte_length)
            .finish()
    }
}

/// Validated encoded key.
#[derive(Clone, PartialEq, Eq)]
pub struct EncodedKey(Vec<u8>);

impl EncodedKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl core::fmt::Debug for EncodedKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match DecodedKey::decode(&self.0) {
            Ok(decoded) => f
                .debug_struct("EncodedKey")
                .field("key_space", &decoded.key_space)
                .field("components", &decoded.components)
                .finish(),
            Err(_) => f
                .debug_struct("EncodedKey")
                .field("key_space", &"<invalid>")
                .field("components", &"<invalid>")
                .finish(),
        }
    }
}

/// Validated decoded key.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedKey {
    key_space: KeySpace,
    components: Vec<DecodedKeyComponent>,
}

impl DecodedKey {
    pub const fn key_space(&self) -> KeySpace {
        self.key_space
    }

    pub fn components(&self) -> &[DecodedKeyComponent] {
        &self.components
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, KeyCodecError> {
        if bytes.len() > MAX_ENCODED_KEY_BYTES {
            return Err(KeyCodecError::KeyTooLong);
        }
        let (&version, remainder) = bytes.split_first().ok_or(KeyCodecError::Truncated)?;
        if version != KEY_FORMAT_VERSION {
            return Err(KeyCodecError::UnknownVersion);
        }
        let (&discriminant, mut remainder) =
            remainder.split_first().ok_or(KeyCodecError::Truncated)?;
        let key_space =
            KeySpace::from_discriminant(discriminant).ok_or(KeyCodecError::UnknownKeySpace)?;
        let shape = key_space.component_shape();
        let mut components = Vec::with_capacity(shape.len());

        for kind in shape {
            match kind {
                KeyComponentKind::Text => {
                    if remainder.len() < 2 {
                        return Err(KeyCodecError::Truncated);
                    }
                    let length = usize::from(u16::from_be_bytes([remainder[0], remainder[1]]));
                    remainder = &remainder[2..];
                    if length == 0 {
                        return Err(KeyCodecError::EmptyTextComponent);
                    }
                    if length > MAX_TEXT_COMPONENT_BYTES {
                        return Err(KeyCodecError::TextComponentTooLong);
                    }
                    if remainder.len() < length {
                        return Err(KeyCodecError::Truncated);
                    }
                    let (text, next) = remainder.split_at(length);
                    let text =
                        core::str::from_utf8(text).map_err(|_| KeyCodecError::InvalidUtf8)?;
                    components.push(DecodedKeyComponent::Text(text.to_owned()));
                    remainder = next;
                }
                KeyComponentKind::U64 => {
                    if remainder.len() < 8 {
                        return Err(KeyCodecError::Truncated);
                    }
                    let bytes: [u8; 8] = remainder[..8]
                        .try_into()
                        .map_err(|_| KeyCodecError::Truncated)?;
                    components.push(DecodedKeyComponent::U64(u64::from_be_bytes(bytes)));
                    remainder = &remainder[8..];
                }
            }
        }

        if !remainder.is_empty() {
            return Err(KeyCodecError::TrailingBytes);
        }
        Ok(Self {
            key_space,
            components,
        })
    }
}

impl core::fmt::Debug for DecodedKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecodedKey")
            .field("key_space", &self.key_space)
            .field("components", &self.components)
            .finish()
    }
}

/// Encode one key using the table's exact component shape.
pub fn encode_key(
    key_space: KeySpace,
    components: &[KeyComponent<'_>],
) -> Result<EncodedKey, KeyCodecError> {
    let shape = key_space.component_shape();
    if components.len() != shape.len() || components.len() > MAX_KEY_COMPONENTS {
        return Err(KeyCodecError::WrongComponentCount);
    }

    let mut encoded = Vec::with_capacity(32);
    encoded.push(KEY_FORMAT_VERSION);
    encoded.push(key_space.discriminant());
    for (component, kind) in components.iter().zip(shape) {
        match (component, kind) {
            (KeyComponent::Text(text), KeyComponentKind::Text) => {
                if text.is_empty() {
                    return Err(KeyCodecError::EmptyTextComponent);
                }
                if text.len() > MAX_TEXT_COMPONENT_BYTES {
                    return Err(KeyCodecError::TextComponentTooLong);
                }
                let length =
                    u16::try_from(text.len()).map_err(|_| KeyCodecError::TextComponentTooLong)?;
                encoded.extend_from_slice(&length.to_be_bytes());
                encoded.extend_from_slice(text.as_bytes());
            }
            (KeyComponent::U64(value), KeyComponentKind::U64) => {
                encoded.extend_from_slice(&value.to_be_bytes());
            }
            _ => return Err(KeyCodecError::WrongComponentKind),
        }
    }
    if encoded.len() > MAX_ENCODED_KEY_BYTES {
        return Err(KeyCodecError::KeyTooLong);
    }
    Ok(EncodedKey(encoded))
}

/// Fail-closed key decode or encode reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCodecError {
    UnknownVersion,
    UnknownKeySpace,
    WrongComponentCount,
    WrongComponentKind,
    EmptyTextComponent,
    TextComponentTooLong,
    KeyTooLong,
    InvalidUtf8,
    Truncated,
    TrailingBytes,
}

impl core::fmt::Display for KeyCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnknownVersion => "unknown d2bkey format version",
            Self::UnknownKeySpace => "unknown d2bkey key space",
            Self::WrongComponentCount => "wrong d2bkey component count",
            Self::WrongComponentKind => "wrong d2bkey component kind",
            Self::EmptyTextComponent => "empty d2bkey text component",
            Self::TextComponentTooLong => "d2bkey text component exceeds its bound",
            Self::KeyTooLong => "d2bkey exceeds its encoded bound",
            Self::InvalidUtf8 => "d2bkey text component is not UTF-8",
            Self::Truncated => "truncated d2bkey",
            Self::TrailingBytes => "d2bkey has trailing bytes",
        })
    }
}

impl std::error::Error for KeyCodecError {}

#[cfg(test)]
mod tests {
    use super::*;

    const UID_A: &str = "123e4567-e89b-42d3-a456-426614174000";
    const UID_B: &str = "123e4567-e89b-42d3-a456-426614174001";

    #[test]
    fn independent_literal_golden_vectors_pin_every_key_space() {
        let vectors: Vec<(
            KeySpace,
            &[KeyComponent<'_>],
            &[u8],
            Vec<DecodedKeyComponent>,
        )> = vec![
            (
                KeySpace::StoreMeta,
                &[KeyComponent::Text("zone_name")],
                b"\x01\x01\x00\x09zone_name",
                vec![DecodedKeyComponent::Text("zone_name".to_owned())],
            ),
            (
                KeySpace::ApiSchemas,
                &[KeyComponent::Text("sha256:00")],
                b"\x01\x02\x00\x09sha256:00",
                vec![DecodedKeyComponent::Text("sha256:00".to_owned())],
            ),
            (
                KeySpace::Resources,
                &[KeyComponent::Text("Host"), KeyComponent::Text("local")],
                b"\x01\x03\x00\x04Host\x00\x05local",
                vec![
                    DecodedKeyComponent::Text("Host".to_owned()),
                    DecodedKeyComponent::Text("local".to_owned()),
                ],
            ),
            (
                KeySpace::TypeIndex,
                &[KeyComponent::Text("Guest"), KeyComponent::Text("work")],
                b"\x01\x04\x00\x05Guest\x00\x04work",
                vec![
                    DecodedKeyComponent::Text("Guest".to_owned()),
                    DecodedKeyComponent::Text("work".to_owned()),
                ],
            ),
            (
                KeySpace::OwnerIndex,
                &[KeyComponent::Text(UID_A), KeyComponent::Text(UID_B)],
                b"\x01\x05\x00\x24123e4567-e89b-42d3-a456-426614174000\
                  \x00\x24123e4567-e89b-42d3-a456-426614174001",
                vec![
                    DecodedKeyComponent::Text("123e4567-e89b-42d3-a456-426614174000".to_owned()),
                    DecodedKeyComponent::Text("123e4567-e89b-42d3-a456-426614174001".to_owned()),
                ],
            ),
            (
                KeySpace::ProducerIndex,
                &[KeyComponent::Text(UID_A), KeyComponent::Text(UID_B)],
                b"\x01\x06\x00\x24123e4567-e89b-42d3-a456-426614174000\
                  \x00\x24123e4567-e89b-42d3-a456-426614174001",
                vec![
                    DecodedKeyComponent::Text("123e4567-e89b-42d3-a456-426614174000".to_owned()),
                    DecodedKeyComponent::Text("123e4567-e89b-42d3-a456-426614174001".to_owned()),
                ],
            ),
            (
                KeySpace::ControllerIndex,
                &[
                    KeyComponent::Text("core"),
                    KeyComponent::Text("Host"),
                    KeyComponent::Text("local"),
                ],
                b"\x01\x07\x00\x04core\x00\x04Host\x00\x05local",
                vec![
                    DecodedKeyComponent::Text("core".to_owned()),
                    DecodedKeyComponent::Text("Host".to_owned()),
                    DecodedKeyComponent::Text("local".to_owned()),
                ],
            ),
            (
                KeySpace::RevisionLog,
                &[KeyComponent::U64(42)],
                b"\x01\x08\x00\x00\x00\x00\x00\x00\x00\x2a",
                vec![DecodedKeyComponent::U64(42)],
            ),
            (
                KeySpace::Operations,
                &[KeyComponent::Text("op-1")],
                b"\x01\x09\x00\x04op-1",
                vec![DecodedKeyComponent::Text("op-1".to_owned())],
            ),
            (
                KeySpace::ZoneLinkCursors,
                &[KeyComponent::Text(UID_A)],
                b"\x01\x0a\x00\x24123e4567-e89b-42d3-a456-426614174000",
                vec![DecodedKeyComponent::Text(
                    "123e4567-e89b-42d3-a456-426614174000".to_owned(),
                )],
            ),
        ];

        for (key_space, components, literal, expected_decoded) in vectors {
            assert_eq!(
                encode_key(key_space, components).unwrap().as_bytes(),
                literal
            );
            let decoded = DecodedKey::decode(literal).unwrap();
            assert_eq!(decoded.key_space(), key_space);
            assert_eq!(decoded.components(), expected_decoded);
        }
    }

    #[test]
    fn decode_rejects_unknown_truncated_overlong_and_noncanonical_shapes() {
        assert_eq!(DecodedKey::decode(&[]), Err(KeyCodecError::Truncated));
        assert_eq!(
            DecodedKey::decode(b"\x02\x01\x00\x01x"),
            Err(KeyCodecError::UnknownVersion)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x0b\x00\x01x"),
            Err(KeyCodecError::UnknownKeySpace)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x01\x00\x00"),
            Err(KeyCodecError::EmptyTextComponent)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x01\x02\x01"),
            Err(KeyCodecError::TextComponentTooLong)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x01\x00\x01\xff"),
            Err(KeyCodecError::InvalidUtf8)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x08\x00"),
            Err(KeyCodecError::Truncated)
        );
        assert_eq!(
            DecodedKey::decode(b"\x01\x08\x00\x00\x00\x00\x00\x00\x00\x01x"),
            Err(KeyCodecError::TrailingBytes)
        );
        assert_eq!(
            DecodedKey::decode(&vec![0; MAX_ENCODED_KEY_BYTES + 1]),
            Err(KeyCodecError::KeyTooLong)
        );
    }

    #[test]
    fn encode_rejects_wrong_shapes_and_bounds() {
        assert_eq!(
            encode_key(KeySpace::RevisionLog, &[KeyComponent::Text("1")]),
            Err(KeyCodecError::WrongComponentKind)
        );
        assert_eq!(
            encode_key(KeySpace::Resources, &[KeyComponent::Text("Host")]),
            Err(KeyCodecError::WrongComponentCount)
        );
        assert_eq!(
            encode_key(KeySpace::StoreMeta, &[KeyComponent::Text("")]),
            Err(KeyCodecError::EmptyTextComponent)
        );
        let overlong = "x".repeat(MAX_TEXT_COMPONENT_BYTES + 1);
        assert_eq!(
            encode_key(KeySpace::StoreMeta, &[KeyComponent::Text(&overlong)]),
            Err(KeyCodecError::TextComponentTooLong)
        );
    }

    #[test]
    fn key_debug_redacts_component_material() {
        const MARKER: &str = "debug-leak-sentinel-key";
        let components = [KeyComponent::Text(MARKER)];
        let encoded = encode_key(KeySpace::StoreMeta, &components).unwrap();
        let decoded = DecodedKey::decode(encoded.as_bytes()).unwrap();
        let rendered = [
            format!("{:?}", components[0]),
            format!("{encoded:?}"),
            format!("{decoded:?}"),
            format!("{:?}", decoded.components()[0]),
        ];

        for diagnostic in &rendered {
            assert!(
                !diagnostic.contains(MARKER),
                "key Debug exposed component material"
            );
        }
        assert!(rendered[1].contains("StoreMeta"));
        assert!(rendered[2].contains("Text"));
        assert!(rendered[2].contains(&MARKER.len().to_string()));
    }
}
