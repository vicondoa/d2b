//! Test-only fake backends.
//!
//! Gated behind the `fake-backends` feature plus `cfg(test)` so they
//! never compile into the production daemon/broker. Per-module fakes live
//! here for the L1c canary matrix.

#![cfg(any(test, feature = "fake-backends"))]

// TODO: per-module fake backends (cgroup, netlink, nft, devices,
// modules) used by the L1c canary matrix tests.

/// Placeholder marker exported so the module compiles as part of the
/// integrator prep workspace build.
#[derive(Debug, Default, Clone, Copy)]
pub struct Placeholder;
