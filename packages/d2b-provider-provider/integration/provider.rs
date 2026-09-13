//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Provider provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! the daemon plane commits a Provider row beside its controller Process and state Volume children, the driver publishes Pending while the controller session evidence is absent, publishes Ready once the evidence is live, and gates the row's retirement on the controller Process child.
