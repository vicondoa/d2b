//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the VolumeBinding provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a binding row whose owning Volume row is live, and
//! prove: the worker Process child commits before the Endpoint child that
//! names it as producer, the derived worker plan re-derives across a daemon
//! restart, the fenced readiness projection reflects the serving socket, a
//! present guest mount blocks the teardown, and the delete leg retires the
//! Endpoint before the worker.
