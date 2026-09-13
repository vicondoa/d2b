//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Role provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! a Role row is committed, the driver converges it without a host effect, and the decision cache admits exactly the positive decisions whose policy revisions match the committed row.
