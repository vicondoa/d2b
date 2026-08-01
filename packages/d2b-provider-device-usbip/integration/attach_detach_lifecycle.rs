//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The scenario requires the framework-owned Core adapter, Provider injection
//! through a production ResourceClient and store-watch dispatcher, a real
//! Network relay Endpoint, broker-backed projection apply and remove, and a
//! Guest supervisor effect channel for attach and detach. The Provider crate is
//! forbidden from importing those framework and broker internals, and none of
//! the required composition points is wired today.
