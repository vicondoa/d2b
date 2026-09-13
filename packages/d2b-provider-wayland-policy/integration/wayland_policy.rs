//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the WaylandPolicy boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, commit a display policy row beside the session that references it, and prove the policy row admits, reconciles, and deletes through its own declaration while the session's readiness follows the policy's status.
