//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the EmergencyPolicy provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! an EmergencyPolicy row is committed, the driver converges it without a host effect, and its exclusive authority scope is admitted once per zone.
