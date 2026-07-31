//! Deterministic interface-name derivation used during Network admission.
//!
//! The canonical implementation lives in the provider-neutral contract crate.
//! Re-exporting it here keeps collision validation and the private core adapter
//! on one byte-for-byte derivation without giving this Provider a second naming
//! implementation.

pub use d2b_contracts::v3::{
    BRIDGE_TAG, DEFAULT_PREFIX, DerivedRole, HASH_SUFFIX_LEN, IfName, IfNameError, IfNameMapping,
    MAX_IFNAME_BYTES, NetworkIfRole, TAP_TAG, derive_from_env_vm, derive_ifname, detect_collisions,
    looks_d2b_owned, validate_prefix,
};
