//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ResourceImport provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! a ResourceImport row is committed beside the zone link it arrives over, the driver converges it without a host effect, and a row whose link is not Ready stays Pending.
