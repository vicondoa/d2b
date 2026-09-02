use d2b_provider_runtime_cloud_hypervisor::adoption::{
    AdoptionOutcome, ProcessIdentity, ProcessObservation, adopt_exact,
};

fn identity(generation: u64, seed: u8) -> ProcessIdentity {
    ProcessIdentity {
        pid: 100 + u32::from(seed),
        start_time_ticks: 10_000 + u64::from(seed),
        cgroup_digest: [seed; 32],
        executable_digest: [seed.wrapping_add(1); 32],
        template_digest: [seed.wrapping_add(2); 32],
        generation,
    }
}

#[test]
fn adoption_requires_one_exact_candidate_and_never_selects_ambiguity() {
    let expected = identity(7, 1);
    assert_eq!(
        adopt_exact(Some(&expected), &ProcessObservation::Absent),
        AdoptionOutcome::Absent
    );
    assert_eq!(
        adopt_exact(
            Some(&expected),
            &ProcessObservation::Candidates(vec![expected])
        ),
        AdoptionOutcome::Adopted
    );
    assert_eq!(
        adopt_exact(
            Some(&expected),
            &ProcessObservation::Candidates(vec![identity(8, 1)])
        ),
        AdoptionOutcome::Quarantined
    );
    assert_eq!(
        adopt_exact(
            Some(&expected),
            &ProcessObservation::Candidates(vec![expected, identity(7, 2)])
        ),
        AdoptionOutcome::Quarantined
    );
    assert_eq!(
        adopt_exact(None, &ProcessObservation::Candidates(vec![expected])),
        AdoptionOutcome::Quarantined
    );
}

#[test]
fn adoption_observation_is_bounded_and_redacted() {
    let candidates = (0..8).map(|seed| identity(7, seed)).collect::<Vec<_>>();
    let observation = ProcessObservation::candidates(candidates).unwrap();
    assert!(!format!("{observation:?}").contains("100"));
    assert!(ProcessObservation::candidates((0..9).map(|seed| identity(7, seed))).is_err());
}
