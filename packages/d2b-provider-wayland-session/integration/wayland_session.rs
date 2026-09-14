//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the WaylandSession boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, admit a display session against a committed policy, and prove the two worker Process children and their Endpoint children are ensured before the Provider effect, the session reaches `Ready`, and deletion drains the children before the Provider teardown stage.
