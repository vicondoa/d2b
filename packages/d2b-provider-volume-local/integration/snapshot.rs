//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The scenario requires the production ResourceClient to launch the signed
//! snapshot EphemeralProcess against a real Host filesystem, publish snapshot
//! status, expire retained snapshots, and coordinate an automatic snapshot
//! with an interrupted migration. No production worker dispatcher or Host
//! snapshot adapter is wired yet.
