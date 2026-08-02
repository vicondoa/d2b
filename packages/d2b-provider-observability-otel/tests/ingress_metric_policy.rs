use std::collections::BTreeMap;

use d2b_provider_observability_otel::{
    Ingress, IngressOutcome, IngressPolicyGate, MetricFrame, MetricPoint,
};
use d2b_telemetry::{IdentityCanaries, MetricDescriptor, meter_registry::label};

fn invalid_frame() -> MetricFrame {
    MetricFrame::new(
        64,
        [MetricPoint {
            descriptor: MetricDescriptor::new(
                "d2b_test_total",
                [label("outcome", &["ok", "error"])],
            ),
            labels: BTreeMap::from([("vm".to_owned(), "resource-name".to_owned())]),
        }],
        BTreeMap::from([("d2b.zone".to_owned(), "work".to_owned())]),
    )
}

#[test]
fn stream_quarantine_requires_three_policy_violations() {
    let mut gate = IngressPolicyGate::default();
    let frame = invalid_frame();
    assert_eq!(
        gate.admit_for_connection(
            Ingress::ImportStream,
            9,
            &frame,
            &IdentityCanaries::default(),
            true
        )
        .0,
        IngressOutcome::Rejected
    );
    assert_eq!(
        gate.admit_for_connection(
            Ingress::ImportStream,
            9,
            &frame,
            &IdentityCanaries::default(),
            true
        )
        .0,
        IngressOutcome::Rejected
    );
    assert_eq!(
        gate.admit_for_connection(
            Ingress::ImportStream,
            9,
            &frame,
            &IdentityCanaries::default(),
            true
        )
        .0,
        IngressOutcome::Quarantined
    );
    assert_eq!(gate.available_import_credits_for(9), 0);
}
