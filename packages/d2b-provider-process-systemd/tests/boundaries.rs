use d2b_provider_process_systemd::drain::{DrainError, DrainProof, DrainStage, validate};
use d2b_provider_process_systemd::metrics::{MetricLabelKey, validate_labels};

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
fn metrics_reject_high_cardinality_or_path_labels() {
    assert!(validate_labels(&[
        (MetricLabelKey::Operation, "start".to_owned()),
        (MetricLabelKey::Domain, "system".to_owned()),
    ]));
    assert!(!validate_labels(&[(
        MetricLabelKey::Operation,
        "Process/host".to_owned()
    )]));
    assert!(!validate_labels(&[(
        MetricLabelKey::Operation,
        "x".repeat(33)
    )]));
}
