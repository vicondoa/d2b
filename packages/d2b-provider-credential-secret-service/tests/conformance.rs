//! Conformance lane for the Secret Service Credential Provider.
//!
//! The descriptor, schema, and method-matrix conformance arms this file used
//! to assert belong to `d2b-provider-toolkit` and `d2b-contracts-provider`,
//! which own and test those checks. The crate's own behavior is covered by
//! `tests/lifecycle.rs`, `tests/faults.rs`, `tests/delivery.rs`,
//! `tests/canary.rs`, `tests/session.rs`, and `tests/placement.rs`; this path
//! stays present because the Provider dossier pins the crate's test layout.

mod common;
