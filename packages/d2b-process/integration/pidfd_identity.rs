//! integration-target: container
//!
//! Scenario contract for the Linux process identity boundary.
//!
//! The container lane must spawn a real non-daemonizing child, obtain a pidfd,
//! record process start time, prove pidfd readability after exit, and reject a
//! candidate whose later start-time observation differs. This file is a tier
//! declaration, not hermetic evidence; the container orchestrator must call
//! [`required_assertions`] and supply the live observations.

/// Assert the outcome vector produced by the container's live pidfd probe.
pub fn required_assertions(
    pidfd_opened: bool,
    readable_after_exit: bool,
    reused_identity_rejected: bool,
) {
    assert!(pidfd_opened, "the live child must yield a pidfd");
    assert!(
        readable_after_exit,
        "the pidfd must become readable after child exit"
    );
    assert!(
        reused_identity_rejected,
        "start-time drift must reject process adoption"
    );
}
