use d2b_provider_network_local::controller::{
    NetworkEffectError, NetworkMetricLabels, NetworkMetricOperation, NetworkMetricOutcome,
};

#[test]
fn metric_keys_and_values_are_closed_and_identity_free() {
    let labels = NetworkMetricLabels::new(
        NetworkMetricOperation::Reconcile,
        NetworkMetricOutcome::Retry,
        Some(NetworkEffectError::Transient),
    );
    let keys = ["operation", "outcome", "error"];
    for forbidden_key in ["vm", "zone", "zone_id", "zone_uid", "network", "resource"] {
        assert!(!keys.contains(&forbidden_key));
    }
    let values = [labels.operation, labels.outcome, labels.error];
    for forbidden in [
        "work-net-canary",
        "198.51.100.77",
        "/run/private",
        "d2b-tsecret",
    ] {
        assert!(!values.iter().any(|value| value.contains(forbidden)));
    }
}
