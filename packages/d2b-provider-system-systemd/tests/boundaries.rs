use d2b_provider_system_systemd::SystemdManifest;
use d2b_provider_system_systemd::drain::{DrainError, DrainProof, DrainStage, validate};
use d2b_provider_system_systemd::metrics::validate_labels;

#[test]
fn drain_requires_exact_stop_manager_terminal_and_empty_leaf() {
    assert_eq!(
        validate(DrainProof::default()),
        Err(DrainError::TerminalTransitionMissing)
    );
    assert_eq!(
        validate(DrainProof {
            exact_main_stopped: true,
            manager_terminal: true,
            cgroup_empty: false,
        }),
        Err(DrainError::LeafNotEmpty)
    );
    assert_eq!(
        validate(DrainProof {
            exact_main_stopped: true,
            manager_terminal: true,
            cgroup_empty: true,
        }),
        Ok(DrainStage::Complete)
    );
}

#[test]
fn metrics_reject_unknown_high_cardinality_or_path_labels() {
    assert!(validate_labels(&[
        ("operation".to_owned(), "start".to_owned()),
        ("domain".to_owned(), "system".to_owned()),
    ]));
    assert!(!validate_labels(&[(
        "resource".to_owned(),
        "host".to_owned()
    )]));
    assert!(!validate_labels(&[(
        "operation".to_owned(),
        "Process/host".to_owned()
    )]));
    assert!(!validate_labels(&[(
        "operation".to_owned(),
        "x".repeat(33)
    )]));
}

#[test]
fn canonical_manifest_keeps_transient_process_contract() {
    let manifest = SystemdManifest::canonical();
    assert_eq!(manifest.artifact_id, "system-systemd");
    assert_eq!(manifest.resource_types, ["Process", "EphemeralProcess"]);
    assert_eq!(manifest.component, "systemd-controller");
    assert!(!manifest.declares_state_volume);
}
