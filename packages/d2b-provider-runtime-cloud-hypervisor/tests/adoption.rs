use d2b_provider_runtime_cloud_hypervisor::adoption::{
    AdoptionOutcome, ProcessIdentity, verify_identity,
};

fn identity(generation: u64) -> ProcessIdentity {
    ProcessIdentity {
        pid: 9,
        start_time_ticks: 12,
        cgroup_digest: [1; 32],
        executable_digest: [2; 32],
        template_digest: [3; 32],
        generation,
    }
}

#[test]
fn stale_start_time_or_generation_is_quarantined_before_pidfd() {
    assert_eq!(
        verify_identity(&identity(1), &identity(1)),
        AdoptionOutcome::Adopted
    );
    assert_eq!(
        verify_identity(&identity(1), &identity(2)),
        AdoptionOutcome::Quarantined
    );
    assert!(!format!("{:?}", identity(1)).contains("12"));
}
