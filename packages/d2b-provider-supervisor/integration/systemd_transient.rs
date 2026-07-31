//! integration-target: host-integration
//!
//! Scenario contract for the real systemd process boundary.
//!
//! The booted-host lane must start a non-forking transient system unit and a
//! verified user scope, atomically bind invocation, cgroup, main-process and
//! start-time identity, open and recheck a pidfd, prove systemd owns wait and
//! reap, then remove the unit and prove adoption reports it absent. This file is
//! not hermetic evidence; the orchestrator supplies live outcomes to
//! [`required_assertions`].

/// Assert the outcome vector produced by the booted-host systemd fixture.
pub fn required_assertions(
    system_unit_verified: bool,
    user_scope_verified: bool,
    pidfd_rechecked: bool,
    manager_reaped: bool,
    vanished_unit_absent: bool,
) {
    assert!(
        system_unit_verified,
        "the transient system unit must verify"
    );
    assert!(user_scope_verified, "the transient user scope must verify");
    assert!(
        pidfd_rechecked,
        "descriptor open must recheck full identity"
    );
    assert!(manager_reaped, "systemd must own wait and reap");
    assert!(
        vanished_unit_absent,
        "a removed transient unit must not be adopted"
    );
}
