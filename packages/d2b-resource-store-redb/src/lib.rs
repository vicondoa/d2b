//! Engine-neutral resource-store contracts prepared for a future redb backend.
//!
//! The production redb engine remains gated on SPIKE-01. This crate currently
//! provides only the frozen physical schema, byte codecs, and ownership rules.

pub mod keys;
pub mod ownership;
pub mod schema;
pub mod values;

pub use keys::{
    DecodedKey, DecodedKeyComponent, EncodedKey, KeyCodecError, KeyComponent, KeySpace,
    MAX_ENCODED_KEY_BYTES, MAX_KEY_COMPONENTS, MAX_TEXT_COMPONENT_BYTES, encode_key,
};
pub use ownership::{
    MAX_OWNER_CHAIN_DEPTH, OwnerBinding, OwnerIndex, OwnerIndexMutation, OwnershipError,
    ReverseOwnerEntry,
};
pub use schema::{TABLE_SCHEMAS, TableSchema};
pub use values::{
    DecodedValue, EncodedValue, MAX_ENCODED_VALUE_BYTES, MAX_VALUE_PAYLOAD_BYTES, ValueCodecError,
    ValueKind, encode_value,
};
