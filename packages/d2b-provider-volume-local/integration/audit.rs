//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The pending scenario requires real controller actions to enter the live Zone
//! audit stream and export metrics through a running observability Provider.
//! Hermetic serialization and catalogue tests do not prove either connection.

/// Makes the declaration-only status machine-readable to integration inventory.
pub const DECLARATION_ONLY: bool = true;

/// Production surfaces required before this scenario can become executable.
pub const REQUIRED_SURFACES: [&str; 3] = [
    "live-zone-audit-stream",
    "running-observability-provider",
    "controller-transition-emission",
];
