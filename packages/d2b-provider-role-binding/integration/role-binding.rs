//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the RoleBinding provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! a RoleBinding row is committed beside the role it references, the driver converges it without a host effect, and the binding's declared subject set resolves against the committed role.
