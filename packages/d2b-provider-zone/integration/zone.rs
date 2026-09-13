//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Zone provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! the daemon plane commits a Zone row, the driver converges it without any host effect, and the production status emitter publishes the exact system-core handler pair beside the row's own handler records.
