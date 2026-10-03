//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ExecutionPolicy provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must prove that an
//! ExecutionPolicy row is committed, an execution instance selects it under
//! granted authorization, a backend missing one of the policy's mandatory
//! confinement facets is refused instead of launched without it, and the
//! compiled seccomp filter the broker loads matches the selected
//! SeccompProfile row.
