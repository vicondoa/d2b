//! Neutral d2b v3 foundation contracts.

pub mod effect_port;
pub mod ifname;

pub use ifname::{
    BRIDGE_TAG, DEFAULT_PREFIX, DerivedRole, HASH_SUFFIX_LEN, IfName, IfNameError, IfNameMapping,
    MAX_IFNAME_BYTES, NetworkIfRole, TAP_TAG, derive_from_env_vm, derive_ifname, detect_collisions,
    derive_network_child_name, derive_network_ifname, derive_network_route_name, looks_d2b_owned,
    validate_prefix,
};
