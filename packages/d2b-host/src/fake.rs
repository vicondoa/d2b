//! Test-only fake backends.
//!
//! Gated behind the `fake-backends` feature plus `cfg(test)` so they
//! never compile into the production daemon/broker. Per-module fakes live
//! here for the L1c canary matrix.

#![cfg(any(test, feature = "fake-backends"))]

// Per-module fake backends (cgroup, netlink, nft, devices, modules) are added
// here as the L1 canary matrix grows.

/// Placeholder marker exported so the module compiles as part of the
/// integrator prep workspace build.
#[derive(Debug, Default, Clone, Copy)]
pub struct Placeholder;
