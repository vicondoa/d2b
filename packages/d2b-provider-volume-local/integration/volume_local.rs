//! integration-target: host-integration
//!
//! Reserved for real Host filesystem provisioning, external marker posture,
//! restart verification, quota enforcement, and cross-process domain
//! isolation. The scenario remains intentionally non-executable until the
//! neutral v3 Volume effect contract and its core/broker adapter are wired;
//! driving the Provider through a local fake would not validate that boundary.

/// Name of the production adapter prerequisite for this scenario.
pub const REQUIRED_ADAPTER: &str = "core-volume-effect-adapter";
