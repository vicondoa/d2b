//! integration-target: host-integration
//!
//! Declaration only. This file is not executable test coverage.
//! The scenario requires the production ResourceClient and store-watch
//! dispatcher to create a staging Volume, launch the signed migration
//! EphemeralProcess, coordinate multiple component writers, and inject a Host
//! crash at each durable prepare, commit, and rollback boundary. Those
//! composition surfaces are not wired, so a local state-machine test here
//! would not exercise the required Host migration path.
