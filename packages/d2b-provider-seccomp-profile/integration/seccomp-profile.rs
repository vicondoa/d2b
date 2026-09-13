//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the SeccompProfile provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! the seed commits SeccompProfile rows, a role referencing an uncommitted profile is refused, and the compiled posture the broker applies matches the committed row.
