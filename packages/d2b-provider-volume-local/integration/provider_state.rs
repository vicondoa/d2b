//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The pending scenario requires a live Zone daemon, a real Host-mounted
//! Volume, a cross-process worker subview, restart adoption, and the complete
//! served-Volume lifecycle. A fake runtime cannot establish those properties.

/// Makes the declaration-only status machine-readable to integration inventory.
pub const DECLARATION_ONLY: bool = true;

/// Production surfaces required before this scenario can become executable.
pub const REQUIRED_SURFACES: [&str; 5] = [
    "live-zone-daemon",
    "host-volume-mount",
    "cross-process-worker-subview",
    "provider-deployment-lifecycle",
    "restart-marker-reverification",
];
