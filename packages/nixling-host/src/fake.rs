//! W3 test-only fake backends.
//!
//! Gated behind the `fake-backends` feature plus `cfg(test)` so they
//! never compile into the production daemon/broker. Scope agents s1-s4
//! land their per-module fakes here as they implement the L1c canary
//! matrix in plan.md §"W3 pre-merge canary matrix".

#![cfg(any(test, feature = "fake-backends"))]

// TODO(W3-s1..s4): per-module fake backends (cgroup, netlink, nft,
// devices, modules) used by the L1c canary matrix tests.

/// Placeholder marker exported so the module compiles as part of the
/// integrator prep workspace build.
#[derive(Debug, Default, Clone, Copy)]
pub struct Placeholder;
