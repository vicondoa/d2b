//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ShellSession boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, commit a session beside its pool and user, and prove the supervisor Process child is ensured before the Provider attachment effect, the session reaches `Ready`, a killed supervisor re-derives the child, and deletion drains the child before the Provider stage.
