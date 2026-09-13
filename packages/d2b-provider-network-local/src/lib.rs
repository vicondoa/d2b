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
pub mod plan;
pub mod routes;

pub use driver::{
    NETWORK_CONTROLLER_REF, NETWORK_CREATIONS, NETWORK_PROVIDER_REF, NETWORK_REGISTRATIONS,
    NETWORK_RESYNC, NETWORK_TYPE_NAME, NetworkComponent, NetworkDriverArgs, NetworkDriverEffects,
    declared_dependency_refs, network_descriptor,
};

pub use d2b_contracts_resource::v3::network::{
    ExternalNicAdmissionError, ExternalNicClaim, MacvtapMode, SharingPolicy,
    admit_external_nic_claims,
};
