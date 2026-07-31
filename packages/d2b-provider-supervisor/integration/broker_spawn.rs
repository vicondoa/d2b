//! integration-target: container
//!
//! Scenario contract for the real broker process boundary.
//!
//! The container lane must dispatch a trusted `SpawnRunner` intent, receive one
//! close-on-exec pidfd through `SCM_RIGHTS`, verify process start time after the
//! handoff, observe broker-owned reap, and inspect that the child was born in
//! the declared cgroup and user namespace before its first instruction. This
//! declaration is not hermetic evidence; the orchestrator supplies live probe
//! results to [`required_assertions`].

/// Assert the outcome vector produced by the live broker fixture.
pub fn required_assertions(
    spawn_succeeded: bool,
    pidfd_received: bool,
    start_time_verified: bool,
    broker_reaped: bool,
    placement_verified: bool,
) {
    assert!(spawn_succeeded, "the trusted runner intent must launch");
    assert!(pidfd_received, "the broker must transfer one pidfd");
    assert!(
        start_time_verified,
        "the process start time must match after descriptor handoff"
    );
    assert!(broker_reaped, "the broker must own wait and reap");
    assert!(
        placement_verified,
        "the child must start in its declared isolation and cgroup placement"
    );
}
