//! integration-target: container
//! coverage-status: declaration-only
//!
//! Scenario contract for the Linux process identity boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits container-scenario orchestration
//! and must not be cited as test evidence. The future scenario must spawn a
//! real non-daemonizing child, obtain a pidfd, record process start time, prove
//! pidfd readability after exit, and reject a candidate whose later start-time
//! observation differs.
