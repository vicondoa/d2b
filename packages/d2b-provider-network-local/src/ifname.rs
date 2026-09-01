//! Deterministic interface-name derivation used during Network admission.
//!
//! The canonical implementation lives in the provider-neutral contract crate.
//! Re-exporting it here keeps collision validation and the private core adapter
//! on one byte-for-byte derivation without giving this Provider a second naming
//! implementation.

use d2b_contracts_resource::v3::ResourceUid;

pub use d2b_contracts_resource::v3::{
    BRIDGE_TAG, DEFAULT_PREFIX, DerivedRole, HASH_SUFFIX_LEN, IfName, IfNameError, IfNameMapping,
    MAX_IFNAME_BYTES, NetworkIfRole, TAP_TAG, derive_ifname, derive_network_ifname,
    derive_network_ownership_marker, detect_collisions, looks_d2b_owned, validate_prefix,
};

/// Derive a bounded private route identity from immutable Network identity.
pub fn derive_network_route_name(network_uid: &ResourceUid, index: usize) -> String {
    d2b_contracts_resource::v3::derive_network_route_name(network_uid, network_uid, index)
}

/// Derive a bounded private route identity from the complete Network identity.
pub fn derive_network_route_name_for(
    zone_uid: &ResourceUid,
    network_uid: &ResourceUid,
    index: usize,
) -> String {
    d2b_contracts_resource::v3::derive_network_route_name(zone_uid, network_uid, index)
}

/// Derive a stable Zone-local child resource name from Network identity.
pub fn derive_network_child_name(network_uid: &ResourceUid, kind: &str) -> String {
    d2b_contracts_resource::v3::derive_network_child_name(network_uid, kind)
}
