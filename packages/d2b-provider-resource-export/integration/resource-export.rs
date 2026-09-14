//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ResourceExport provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that
//! a ResourceExport row is committed for an exportable subject, the driver converges it without a host effect, and a non-exportable subject is refused before any row is written.
