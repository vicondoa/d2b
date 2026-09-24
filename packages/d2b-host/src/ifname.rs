//! Host-prepare ifname surface.
//!
//! Re-exports the canonical v3 ifname vocabulary
//! (`d2b_contracts_resource::v3::ifname`): the IFNAMSIZ-validated
//! [`IfName`] newtype, the FNV-1a/base32 derivation
//! ([`derive_from_env_vm`]), collision detection, and the
//! `looks_d2b_owned` predicate. The host copy that hand-rolled these
//! was folded onto the contract crate's byte-identical surface.

pub use d2b_contracts_resource::v3::ifname::*;