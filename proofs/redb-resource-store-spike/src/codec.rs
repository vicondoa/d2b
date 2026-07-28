use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model::{StoreError, StoreResult};

const KEY_HEADER: &[u8] = b"d2bkey/v1";
const VALUE_HEADER: &[u8] = b"d2bval/v1";

pub enum KeyPart<'a> {
    Text(&'a str),
    Revision(u64),
}

pub fn key(key_space: u8, parts: &[KeyPart<'_>]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(64);
    encoded.extend_from_slice(KEY_HEADER);
    encoded.push(key_space);
    for part in parts {
        match part {
            KeyPart::Text(value) => {
                encoded.push(1);
                encoded.extend_from_slice(
                    &u32::try_from(value.len())
                        .expect("bounded synthetic key")
                        .to_be_bytes(),
                );
                encoded.extend_from_slice(value.as_bytes());
            }
            KeyPart::Revision(value) => {
                encoded.push(2);
                encoded.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    encoded
}

pub fn value<T: Serialize>(kind: u16, value: &T) -> StoreResult<Vec<u8>> {
    let mut encoded = Vec::with_capacity(128);
    encoded.extend_from_slice(VALUE_HEADER);
    encoded.extend_from_slice(&kind.to_be_bytes());
    encoded.extend_from_slice(
        &serde_json::to_vec(value)
            .map_err(|error| StoreError::Integrity(format!("encode:{error}")))?,
    );
    Ok(encoded)
}

pub fn decode<T: DeserializeOwned>(expected_kind: u16, bytes: &[u8]) -> StoreResult<T> {
    let header_length = VALUE_HEADER.len();
    if bytes.get(..header_length) != Some(VALUE_HEADER) {
        return Err(StoreError::Integrity("unknown-value-header".to_owned()));
    }
    let kind_bytes: [u8; 2] = bytes
        .get(header_length..header_length + 2)
        .ok_or_else(|| StoreError::Integrity("truncated-value-kind".to_owned()))?
        .try_into()
        .map_err(|_| StoreError::Integrity("invalid-value-kind".to_owned()))?;
    let kind = u16::from_be_bytes(kind_bytes);
    if kind != expected_kind {
        return Err(StoreError::Integrity(format!(
            "value-kind:{kind:#06x}!={expected_kind:#06x}"
        )));
    }
    serde_json::from_slice(
        bytes
            .get(header_length + 2..)
            .ok_or_else(|| StoreError::Integrity("missing-value-body".to_owned()))?,
    )
    .map_err(|error| StoreError::Integrity(format!("decode:{error}")))
}
