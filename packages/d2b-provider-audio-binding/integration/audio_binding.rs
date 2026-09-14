//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the AudioBinding boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, commit a binding against a ready service and guest, and prove the worker and endpoint children are committed before the Provider lease effect, the binding reaches `Ready`, and deletion finalizes the lease before the children retire.
