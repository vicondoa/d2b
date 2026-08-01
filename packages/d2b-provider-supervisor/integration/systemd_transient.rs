//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the real systemd process boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits booted-host scenario orchestration
//! and must not be cited as test evidence. The future scenario must start a
//! non-forking transient system unit and a verified user scope, atomically bind
//! invocation, cgroup, main-process, and start-time identity, recheck a pidfd,
//! prove manager wait and reap ownership, then prove a removed unit is absent.
