//! integration-target: container
//! coverage-status: declaration-only
//!
//! Scenario contract for the real broker process boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits container-scenario orchestration
//! and must not be cited as test evidence. The future scenario must dispatch a
//! trusted `SpawnRunner` intent, receive one close-on-exec pidfd through
//! `SCM_RIGHTS`, verify process start time after handoff, observe broker-owned
//! reap, and inspect final cgroup and user-namespace placement.
