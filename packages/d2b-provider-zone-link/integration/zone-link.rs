//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ZoneLink provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! a ZoneLink row is committed beside its gateway composition, the driver converges it, an enrollment pass commits before it releases effects, and a restart re-derives the session state from the durable record.
