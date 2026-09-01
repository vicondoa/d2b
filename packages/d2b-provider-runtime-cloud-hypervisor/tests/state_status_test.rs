use d2b_provider_runtime_cloud_hypervisor::state::{
    GuestGenerationSet, GuestStatusObservation, GuestStatusPhase, finalization_eligible,
    reduce_status,
};

fn ready_observation() -> GuestStatusObservation {
    GuestStatusObservation {
        generations: GuestGenerationSet::all(7),
        dependencies_ready: true,
        process_ready: true,
        endpoint_ready: true,
        session_ready: true,
        seed_ready: true,
        session_healthy: true,
        required_children_healthy: true,
        deletion_requested: false,
        session_active: true,
        descendants_present: true,
        process_stopped: false,
    }
}

#[test]
fn status_precedence_is_table_driven() {
    let ready = ready_observation();
    let cases = [
        (
            "missing dependency",
            GuestStatusObservation {
                dependencies_ready: false,
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "endpoint not ready",
            GuestStatusObservation {
                endpoint_ready: false,
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "session not ready",
            GuestStatusObservation {
                session_ready: false,
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "seed not ready",
            GuestStatusObservation {
                seed_ready: false,
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        ("same-generation graph", ready, GuestStatusPhase::Ready),
        (
            "session health loss",
            GuestStatusObservation {
                session_healthy: false,
                ..ready
            },
            GuestStatusPhase::Degraded,
        ),
        (
            "required-child health loss",
            GuestStatusObservation {
                required_children_healthy: false,
                ..ready
            },
            GuestStatusPhase::Degraded,
        ),
        (
            "provider generation mismatch",
            GuestStatusObservation {
                generations: GuestGenerationSet {
                    provider: 8,
                    ..ready.generations
                },
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "descriptor generation mismatch",
            GuestStatusObservation {
                generations: GuestGenerationSet {
                    descriptor: 8,
                    ..ready.generations
                },
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "controller generation mismatch",
            GuestStatusObservation {
                generations: GuestGenerationSet {
                    controller: 8,
                    ..ready.generations
                },
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "child generation mismatch",
            GuestStatusObservation {
                generations: GuestGenerationSet {
                    child: 8,
                    ..ready.generations
                },
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "session generation mismatch",
            GuestStatusObservation {
                generations: GuestGenerationSet {
                    session: 8,
                    ..ready.generations
                },
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "missing generation",
            GuestStatusObservation {
                generations: GuestGenerationSet::all(0),
                ..ready
            },
            GuestStatusPhase::Pending,
        ),
        (
            "deletion takes precedence",
            GuestStatusObservation {
                deletion_requested: true,
                ..ready
            },
            GuestStatusPhase::Draining,
        ),
    ];

    for (name, observation, expected) in cases {
        assert_eq!(reduce_status(&observation).phase, expected, "{name}");
    }
}

#[test]
fn finalization_requires_a_complete_current_generation_drain() {
    let ready = ready_observation();
    let cases = [
        ("not deleting", ready, false),
        (
            "active session",
            GuestStatusObservation {
                deletion_requested: true,
                ..ready
            },
            false,
        ),
        (
            "remaining descendants",
            GuestStatusObservation {
                deletion_requested: true,
                session_active: false,
                descendants_present: true,
                process_stopped: true,
                ..ready
            },
            false,
        ),
        (
            "running process",
            GuestStatusObservation {
                deletion_requested: true,
                session_active: false,
                descendants_present: false,
                process_stopped: false,
                ..ready
            },
            false,
        ),
        (
            "complete drain",
            GuestStatusObservation {
                deletion_requested: true,
                session_active: false,
                descendants_present: false,
                process_stopped: true,
                ..ready
            },
            true,
        ),
        (
            "stale generation",
            GuestStatusObservation {
                deletion_requested: true,
                session_active: false,
                descendants_present: false,
                process_stopped: true,
                generations: GuestGenerationSet {
                    session: 8,
                    ..ready.generations
                },
                ..ready
            },
            false,
        ),
    ];

    for (name, observation, expected) in cases {
        assert_eq!(finalization_eligible(&observation), expected, "{name}");
    }
}

#[test]
fn draining_uses_provider_deleting_phase_on_the_wire() {
    let status = reduce_status(&GuestStatusObservation {
        deletion_requested: true,
        ..ready_observation()
    });
    assert_eq!(
        serde_json::to_value(status).unwrap()["phase"],
        serde_json::json!("Deleting")
    );
}

#[test]
fn status_debug_output_is_identity_free() {
    let status = reduce_status(&ready_observation());
    let debug = format!("{status:?}").to_ascii_lowercase();
    let public = serde_json::to_string(&status).unwrap().to_ascii_lowercase();
    for forbidden in ["pid", "uid", "path", "argv", "credential", "cid", "cgroup"] {
        assert!(!debug.contains(forbidden), "{forbidden} leaked in {debug}");
        assert!(
            !public.contains(forbidden),
            "{forbidden} leaked in {public}"
        );
    }
    assert!(debug.contains("ready"));
}
