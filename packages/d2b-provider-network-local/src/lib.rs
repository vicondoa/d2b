//! Host-network policy and observation primitives for `Provider/network-local`.
//!
//! Kernel effects remain behind the injected network effect boundary. This
//! crate computes desired bridge-port policy, validates observations, and
//! produces ownership-scoped firewall projections. It does not open a broker
//! socket or mutate host state directly.

#![deny(missing_docs)]

pub mod bridge_port;
pub mod ifname;
pub mod netlink;
pub mod nftables;
pub mod routes;

pub use d2b_contracts::v3::network::{
    ExternalNicAdmissionError, ExternalNicClaim, MacvtapMode, SharingPolicy,
    admit_external_nic_claims,
};
