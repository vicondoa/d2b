use std::collections::BTreeMap;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_observability_otel::{
    IdentityCanaries, Ingress, IngressOutcome, MetricFrame, MetricPoint,
    TelemetryBindingController, TelemetryBindingPhase,
};

fn refs() -> (ResourceRef, ResourceRef, ResourceRef) {
    (
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        ResourceRef::parse("telemetry.d2bus.org.TelemetryService/zone").unwrap(),
        ResourceRef::parse("Guest/workload").unwrap(),
    )
}

fn frame() -> MetricFrame {
    MetricFrame::new(
        64,
        [MetricPoint {
            descriptor: d2b_provider_observability_otel::canonical_descriptor(
                "d2b_otel_ingress_policy_total",
            )
            .unwrap(),
            labels: BTreeMap::from([
                ("ingress".to_owned(), "otlp_vsock".to_owned()),
                ("outcome".to_owned(), "accepted".to_owned()),
                ("error_class".to_owned(), "none".to_owned()),
            ]),
            value: 1.0,
        }],
        BTreeMap::from([(
            "d2b.zone".to_owned(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
        )]),
    )
}

#[test]
fn explicit_binding_reconciles_collector_children_and_route_status() {
    let (binding, service, target) = refs();
    let mut controller = TelemetryBindingController::new();
    let result = controller
        .reconcile(
            &binding,
            &service,
            &target,
            Ingress::OtlpVsock,
            7,
            &frame(),
            &IdentityCanaries::default(),
            true,
        )
        .unwrap();
    assert_eq!(controller.phase(), TelemetryBindingPhase::Ready);
    assert_eq!(result.status.outcome, Some(IngressOutcome::Accepted));
    assert_eq!(result.children.iter().count(), 4);
    assert_eq!(
        result
            .children
            .child("ingest-endpoint")
            .unwrap()
            .producer_ref(),
        Some(result.children.child("collector").unwrap().resource_ref())
    );
    assert_eq!(
        result
            .children
            .child("forwarder-endpoint")
            .unwrap()
            .producer_ref(),
        Some(result.children.child("forwarder").unwrap().resource_ref())
    );
}

#[test]
fn finalization_blocks_reconcile_and_service_alone_cannot_create_children() {
    let (binding, service, target) = refs();
    let mut controller = TelemetryBindingController::new();
    controller.finalize().unwrap();
    assert!(
        controller
            .reconcile(
                &binding,
                &service,
                &target,
                Ingress::OtlpVsock,
                7,
                &frame(),
                &IdentityCanaries::default(),
                true,
            )
            .is_err()
    );
    assert!(TelemetryBindingController::child_resources(&service, &service, &target).is_err());
}
