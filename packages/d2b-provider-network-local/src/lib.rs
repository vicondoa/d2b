//! Host-network policy and observation primitives for `Provider/network-local`.
//!
//! Kernel effects remain behind the injected network effect boundary. This
//! crate computes desired bridge-port policy, validates observations, and
//! produces ownership-scoped firewall projections. It does not open a broker
//! socket or mutate host state directly.

#![deny(missing_docs)]

pub mod artifact;
pub mod bridge_port;
pub mod broker;
pub mod controller;
pub mod diagnostics;
pub mod driver;
pub mod ifname;
pub mod netlink;
pub mod nftables;
pub mod observe;
pub mod operations;
pub mod plan;
pub mod routes;

// `test_support` is needed both by external crates (which opt in via the
// `test-support` feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically
// when compiling this crate's tests, so `cargo test -p d2b-provider-network-local`
// works without enabling the feature.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    NETWORK_CONTROLLER_REF, NETWORK_CREATIONS, NETWORK_FAMILY_NAME, NETWORK_PROVIDER_REF, NETWORK_REGISTRATIONS,
    NETWORK_RESYNC, NETWORK_TYPE_NAME, NetworkComponent, NetworkDriverArgs, NetworkDriverEffects,
    declared_dependency_refs, network_descriptor,
};
pub use operations::{
    APPLY_NFTABLES, APPLY_NFTABLES_PROJECTION, APPLY_NM_UNMANAGED, APPLY_ROUTE, APPLY_SYSCTL,
    CREATE_BRIDGE, CREATE_PERSISTENT_TAP, CREATE_TAP_FD, DELETE_BRIDGE, DELETE_PERSISTENT_TAP,
    KERNEL_APPLY_NFTABLES_PROJECTION, KERNEL_SEED_DNSMASQ_LEASE, SEED_DNSMASQ_LEASE,
    SET_BRIDGE_PORT_FLAGS, UPDATE_HOSTS_FILE, network_family_operations,
};

pub use d2b_contracts_resource::v3::network::{
    ExternalNicAdmissionError, ExternalNicClaim, MacvtapMode, SharingPolicy,
    admit_external_nic_claims,
};
