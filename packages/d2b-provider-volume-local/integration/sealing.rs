//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The scenario requires a live Credential Provider lease, the production
//! policy-bound sealing adapter, durable envelope writes, and crash injection
//! during key-rotation commit. The current crate owns only key-free
//! coordination policy, so an in-process substitute would not prove lease
//! revocation, key custody, or interrupted rotation behavior.
