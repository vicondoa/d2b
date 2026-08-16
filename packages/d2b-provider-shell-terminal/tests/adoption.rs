use d2b_provider_shell_terminal::{
    AdoptionDecision, SupervisorCandidate, SupervisorIdentity, adopt_supervisor,
};

fn identity(seed: u8, generation: u64) -> SupervisorIdentity {
    SupervisorIdentity::new([seed; 32], [seed.wrapping_add(1); 32], generation).unwrap()
}

#[test]
fn restart_adoption_requires_one_owned_matching_supervisor() {
    let expected = identity(4, 7);
    let candidate = SupervisorCandidate::new("session-a", expected.clone());
    assert_eq!(
        adopt_supervisor("session-a", &expected, &[candidate]),
        AdoptionDecision::Adopted
    );

    let stale = SupervisorCandidate::new("session-a", identity(4, 6));
    assert_eq!(
        adopt_supervisor("session-a", &expected, &[stale]),
        AdoptionDecision::StaleGeneration
    );
    let duplicate = SupervisorCandidate::new("session-a", expected.clone());
    assert_eq!(
        adopt_supervisor(
            "session-a",
            &expected,
            &[
                SupervisorCandidate::new("session-a", expected.clone()),
                duplicate,
            ],
        ),
        AdoptionDecision::Ambiguous
    );
}
