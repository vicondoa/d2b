use d2b_provider_network_local::controller::{
    NetworkEffectError, NetworkMetricLabels, NetworkMetricOperation, NetworkMetricOutcome,
};

fn expected_operation(operation: NetworkMetricOperation) -> &'static str {
    match operation {
        NetworkMetricOperation::Reconcile => "reconcile",
        NetworkMetricOperation::Observe => "observe",
        NetworkMetricOperation::Finalize => "finalize",
        NetworkMetricOperation::VolumeSync => "volume-sync",
        NetworkMetricOperation::AgentReload => "agent-reload",
    }
}

fn expected_outcome(outcome: NetworkMetricOutcome) -> &'static str {
    match outcome {
        NetworkMetricOutcome::Success => "success",
        NetworkMetricOutcome::Retry => "retry",
        NetworkMetricOutcome::Blocked => "blocked",
    }
}

fn expected_error(error: Option<NetworkEffectError>) -> &'static str {
    match error {
        None => "none",
        Some(NetworkEffectError::Transient) => "network-effect-transient",
        Some(NetworkEffectError::BridgeCreate) => "bridge-create-error",
        Some(NetworkEffectError::ConfigVolume) => "config-volume-error",
        Some(NetworkEffectError::HostMemoryBudgetExceeded) => "host-memory-budget-exceeded",
        Some(NetworkEffectError::StaleConfigurationGeneration) => "stale-projection-generation",
        Some(NetworkEffectError::StaleAttachmentGeneration) => "attachment-generation-mismatch",
        Some(NetworkEffectError::ForeignOwnership) => "foreign-nft-rule-preserved",
        Some(NetworkEffectError::CidrConflict) => "cidr-conflict",
        Some(NetworkEffectError::CrossZoneL2) => "external-physical-nic-cross-zone-l2",
        Some(NetworkEffectError::Artifact) => "net-vm-artifact-resolution",
        Some(NetworkEffectError::InvalidState) => "network-controller-invalid-state",
    }
}

#[test]
fn metric_constructor_projects_only_the_closed_schema_and_values() {
    let constructor: fn(
        NetworkMetricOperation,
        NetworkMetricOutcome,
        Option<NetworkEffectError>,
    ) -> NetworkMetricLabels = NetworkMetricLabels::new;
    let operations = [
        NetworkMetricOperation::Reconcile,
        NetworkMetricOperation::Observe,
        NetworkMetricOperation::Finalize,
        NetworkMetricOperation::VolumeSync,
        NetworkMetricOperation::AgentReload,
    ];
    let outcomes = [
        NetworkMetricOutcome::Success,
        NetworkMetricOutcome::Retry,
        NetworkMetricOutcome::Blocked,
    ];
    let errors = [
        None,
        Some(NetworkEffectError::Transient),
        Some(NetworkEffectError::BridgeCreate),
        Some(NetworkEffectError::ConfigVolume),
        Some(NetworkEffectError::HostMemoryBudgetExceeded),
        Some(NetworkEffectError::StaleConfigurationGeneration),
        Some(NetworkEffectError::StaleAttachmentGeneration),
        Some(NetworkEffectError::ForeignOwnership),
        Some(NetworkEffectError::CidrConflict),
        Some(NetworkEffectError::CrossZoneL2),
        Some(NetworkEffectError::Artifact),
        Some(NetworkEffectError::InvalidState),
    ];

    for operation in operations {
        for outcome in outcomes {
            for error in errors {
                let NetworkMetricLabels {
                    operation: actual_operation,
                    outcome: actual_outcome,
                    error: actual_error,
                } = constructor(operation, outcome, error);
                assert_eq!(actual_operation, expected_operation(operation));
                assert_eq!(actual_outcome, expected_outcome(outcome));
                assert_eq!(actual_error, expected_error(error));
            }
        }
    }
}
