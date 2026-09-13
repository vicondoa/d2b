//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Operation provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! the seed commits Operation rows, the envelope resolves one row per dispatch and refuses an uncommitted operation, and a payload the row's schema rejects is refused before any handler runs.
