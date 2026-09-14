//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Command provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! the seed commits Command rows, the process controller materializes one spawn operation per command, and a command whose argv placeholder names no declared parameter is refused at seed.
