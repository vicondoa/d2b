use std::collections::BTreeMap;

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingEvidence, BindingLifecycleState, FreshnessTuple, RefusalReason,
    ResourceRef,
};
use d2b_provider_observability_otel::route_fixtures::{
    admitted_credential_evidence, admitted_endpoint_evidence, admitted_network_evidence,
    binding_key, freshness, observe, observed_for, observed_for_all,
};
use d2b_provider_observability_otel::{
    DeliveryRoute, DeliverySource, IdentityCanaries, Ingress, IngressOutcome, MetricFrame,
    MetricPoint, TelemetryBindingController, TelemetryBindingFrame,
    TelemetryBindingPhase, TelemetryComponentSession, TelemetryControllerError,
    TelemetryServiceController, TelemetryServiceError, TelemetryServicePhase,
    TelemetryServiceRole, TelemetryStreamRequest, TelemetryStreamSignal, canonical_descriptor,
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
            descriptor: canonical_descriptor("d2b_otel_ingress_policy_total").unwrap(),
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

/// A live route over the admitted endpoint relationship, on the vsock
/// transport the endpoint owner admitted it for.
fn live_route() -> (DeliveryRoute, Vec<FreshnessTuple>) {
    let endpoint = admitted_endpoint_evidence();
    let observed = observed_for(&endpoint);
    let route = DeliveryRoute::for_endpoint(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        endpoint,
    )
    .expect("the admitted endpoint relationship is a route");
    (route, observed)
}

#[test]
fn explicit_binding_reconciles_collector_children_and_route_status() {
    let (binding, service, target) = refs();
    let (route, observed) = live_route();
    let frame = frame();
    let mut controller = TelemetryBindingController::new();
    let result = controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
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
    let (route, observed) = live_route();
    let frame = frame();
    let mut controller = TelemetryBindingController::new();
    controller.finalize().unwrap();
    assert!(
        controller
            .reconcile(
                &binding,
                &service,
                &target,
                TelemetryBindingFrame {
                    route: &route,
                    observed: &observed,
                    connection_id: 7,
                    frame: &frame,
                    canaries: &IdentityCanaries::default(),
                    capacity_available: true,
                },
            )
            .is_err()
    );
    assert!(TelemetryBindingController::child_resources(&service, &service, &target).is_err());
}

#[test]
fn service_resource_reconciliation_is_separate_from_stream_admission() {
    let (binding, service, _target) = refs();
    let provider = ResourceRef::parse("Provider/observability-otel").unwrap();
    let endpoint = ResourceRef::parse("Endpoint/ingest").unwrap();
    let mut service_controller = TelemetryServiceController::new();
    let status = service_controller
        .reconcile(
            &service,
            &provider,
            TelemetryServiceRole::Authority,
            &[endpoint],
            true,
            true,
        )
        .unwrap();
    assert_eq!(status.phase, TelemetryServicePhase::Ready);
    assert_eq!(service_controller.phase(), TelemetryServicePhase::Ready);

    let session = TelemetryComponentSession;
    let stream = session
        .open_stream(TelemetryStreamRequest {
            service_ref: service,
            binding_ref: binding,
            signal: TelemetryStreamSignal::Metrics,
        })
        .unwrap();
    assert_eq!(stream.request().signal, TelemetryStreamSignal::Metrics);
    assert_eq!(
        TelemetryComponentSession::resource_mutation_forbidden(),
        TelemetryControllerError::StreamOnly
    );
}

#[test]
fn service_authority_and_stream_target_mismatches_fail_closed() {
    let mut controller = TelemetryServiceController::new();
    let result = controller.reconcile(
        &ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/incorrect").unwrap(),
        &ResourceRef::parse("Provider/observability-otel").unwrap(),
        TelemetryServiceRole::Authority,
        &[ResourceRef::parse("Endpoint/ingest").unwrap()],
        true,
        true,
    );
    assert!(result.is_err());

    let session = TelemetryComponentSession;
    let result = session.open_stream(TelemetryStreamRequest {
        service_ref: ResourceRef::parse("telemetry.d2bus.org.TelemetryService/ingest").unwrap(),
        binding_ref: ResourceRef::parse("Process/not-a-binding").unwrap(),
        signal: TelemetryStreamSignal::Logs,
    });
    assert_eq!(result, Err(TelemetryControllerError::Admission));
}

#[test]
fn projection_readiness_requires_ingest_evidence() {
    let (_binding, service, _target) = refs();
    let provider = ResourceRef::parse("Provider/observability-otel").unwrap();
    let mut controller = TelemetryServiceController::new();

    let status = controller
        .reconcile(
            &service,
            &provider,
            TelemetryServiceRole::Projection,
            &[],
            true,
            false,
        )
        .unwrap();

    assert_eq!(status.phase, TelemetryServicePhase::Pending);
}

#[test]
fn telemetry_children_reject_unsupported_target_types() {
    let (binding, service, _target) = refs();
    let user = ResourceRef::parse("User/alice").unwrap();

    assert_eq!(
        TelemetryBindingController::child_resources(&binding, &service, &user),
        Err(TelemetryControllerError::Admission)
    );
}

#[test]
fn telemetry_authority_rejects_non_endpoint_ingest_rows() {
    let (_binding, service, _target) = refs();
    let provider = ResourceRef::parse("Provider/observability-otel").unwrap();
    let process = ResourceRef::parse("Process/not-an-endpoint").unwrap();
    let mut controller = TelemetryServiceController::new();

    assert_eq!(
        controller.reconcile(
            &service,
            &provider,
            TelemetryServiceRole::Authority,
            &[process],
            true,
            true,
        ),
        Err(TelemetryServiceError::InvalidAuthority)
    );
}

#[test]
fn telemetry_binding_requires_an_identity_scoped_connection() {
    let (binding, service, target) = refs();
    let (route, observed) = live_route();
    let frame = frame();
    let mut controller = TelemetryBindingController::new();

    assert_eq!(
        controller
            .reconcile(
                &binding,
                &service,
                &target,
                TelemetryBindingFrame {
                    route: &route,
                    observed: &observed,
                    connection_id: 0,
                    frame: &frame,
                    canaries: &IdentityCanaries::default(),
                    capacity_available: true,
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Err(TelemetryControllerError::Admission.to_string()),
    );
}

// ---------------------------------------------------------------------------
// Scenario 1: an invalid config cannot publish a self-grant or bypass an
// unsupported provider facet. Proven here over the shared binding contract
// that both the configuration provider and telemetry deliver through.
// ---------------------------------------------------------------------------

#[test]
fn a_route_never_admits_an_authorization_free_delivery() {
    // The contract refuses a request with no authorization evidence, and
    // `BindingEvidence` is only reachable from an admission that went
    // through it, so no observability-side value can stand in for the grant.
    let key = binding_key(
        d2b_contracts_resource::v3::BindingKind::Endpoint,
        &ResourceRef::parse("Endpoint/ingest").unwrap(),
        &ResourceRef::parse("Process/collector").unwrap(),
    );
    let source = d2b_contracts_resource::v3::SourceAdmission::new(
        key.clone(),
        vec![d2b_contracts_resource::v3::RequestedRights::Consume],
        d2b_contracts_resource::v3::BindingArbitration::Shared,
    )
    .unwrap();
    let refusal = d2b_contracts_resource::v3::admit_binding_request(
        &key,
        d2b_contracts_resource::v3::RequestedRights::Consume,
        &[d2b_contracts_resource::v3::BindingRealizationFacet::EndpointDescriptor],
        &d2b_contracts_resource::v3::BindingAuthorization::absent(),
        &source,
        &d2b_provider_observability_otel::route_fixtures::support(&[
            d2b_contracts_resource::v3::BindingRealizationFacet::EndpointDescriptor,
        ]),
        &[freshness(key.source_ref())],
    )
    .expect_err("an unauthorized request is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);
}

#[test]
fn a_route_refuses_a_relationship_whose_kind_does_not_match_its_source() {
    // Presenting a network binding as the endpoint delivery is a
    // self-granting substitution; the route refuses it at the admitting
    // stage rather than accepting the evidence under the wrong name.
    let network = admitted_network_evidence();
    let error = DeliveryRoute::new(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        [(DeliverySource::Endpoint, network)],
    )
    .expect_err("a mismatched relationship is refused");
    assert_eq!(error.stage(), AdmissionStage::Admit);
    assert_eq!(error.reason(), RefusalReason::SourcePolicyRefused);
}

#[test]
fn a_route_requires_an_admitted_endpoint_relationship() {
    // A route with only a credential relationship has no delivery: there is
    // nothing to submit to. The absence is refused at the authorizing stage.
    let credential = admitted_credential_evidence();
    let error = DeliveryRoute::new(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        [(DeliverySource::Credential, credential)],
    )
    .expect_err("a route with no endpoint is refused");
    assert_eq!(error.stage(), AdmissionStage::Authorize);
    assert_eq!(error.reason(), RefusalReason::IdentityNotAuthorized);
}

// ---------------------------------------------------------------------------
// Scenario 2: endpoint/credential revocation stops new delivery, and it
// stops rather than falling back to another channel.
// ---------------------------------------------------------------------------

#[test]
fn revoking_the_endpoint_relationship_stops_new_delivery() {
    let (binding, service, target) = refs();
    let (mut route, observed) = live_route();
    let frame = frame();
    let mut controller = TelemetryBindingController::new();

    // Live: the same frame is accepted.
    controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect("the live relationship delivers");

    route.revoke(DeliverySource::Endpoint);
    let error = controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect_err("a revoked endpoint stops delivery");
    let TelemetryControllerError::DeliveryRefused(refusal) = error else {
        panic!("revocation is a delivery refusal, not a structural rejection");
    };
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
    assert_eq!(refusal.source(), DeliverySource::Endpoint);
    assert_eq!(
        controller.phase(),
        TelemetryBindingPhase::Degraded,
        "the binding reports the stopped route rather than a usable one"
    );
}

#[test]
fn revoking_the_credential_relationship_stops_export_delivery() {
    let (binding, service, target) = refs();
    let endpoint = admitted_endpoint_evidence();
    let credential = admitted_credential_evidence();
    let observed = observed_for_all(&[&endpoint, &credential]);
    let mut route = DeliveryRoute::new(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        [
            (DeliverySource::Endpoint, endpoint),
            (DeliverySource::Credential, credential),
        ],
    )
    .expect("both relationships are admitted");
    let frame = frame();
    let mut controller = TelemetryBindingController::new();

    controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect("the live relationships deliver");

    route.revoke(DeliverySource::Credential);
    let error = controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect_err("a revoked credential stops export delivery");
    assert!(matches!(
        error,
        TelemetryControllerError::DeliveryRefused(_)
    ));
    assert_eq!(controller.phase(), TelemetryBindingPhase::Degraded);
}

#[test]
fn a_revoked_relationship_does_not_resume_on_another_transport() {
    // The route is pinned to the one transport its endpoint owner admitted.
    // Re-presenting the same producer elsewhere is refused rather than
    // silently routed somewhere delivery still works.
    let (mut route, observed) = live_route();
    for ingress in [
        Ingress::EmitterUnix,
        Ingress::OtlpUnix,
        Ingress::ImportStream,
    ] {
        let error = route
            .channel_refusal(ingress)
            .expect("another transport is not this route");
        assert_eq!(error.stage(), AdmissionStage::Admit);
        assert_eq!(error.reason(), RefusalReason::SourcePolicyRefused);
    }
    assert!(
        route.channel_refusal(Ingress::OtlpVsock).is_none(),
        "the admitted transport is the only one this route serves"
    );
    route.revoke(DeliverySource::Endpoint);
    for ingress in [
        Ingress::OtlpVsock,
        Ingress::EmitterUnix,
        Ingress::OtlpUnix,
        Ingress::ImportStream,
    ] {
        assert!(
            route.refusal(&observed).is_some(),
            "{ingress:?} stays refused after revocation"
        );
    }
}

#[test]
fn a_relationship_whose_committed_revision_moved_stops_delivery() {
    // The dependency fence: the admitted freshness no longer matches what
    // the source published, so the earlier admission is stale.
    let (binding, service, target) = refs();
    let (route, observed) = live_route();
    let frame = frame();
    let mut controller = TelemetryBindingController::new();

    // The endpoint row's committed revision advanced, so the observed tuple
    // the admission was fenced against no longer appears.
    let moved: Vec<FreshnessTuple> = Vec::new();
    let error = controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &moved,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect_err("a moved dependency stops delivery");
    let TelemetryControllerError::DeliveryRefused(refusal) = error else {
        panic!("a stale admission is a delivery refusal");
    };
    assert_eq!(refusal.stage(), AdmissionStage::Reserve);
    assert_eq!(refusal.reason(), RefusalReason::UnprovenEffect);
    assert_eq!(observed.len(), 1, "the fixture observed one dependency");
    assert_eq!(controller.phase(), TelemetryBindingPhase::Degraded);
}

#[test]
fn a_relationship_that_left_admitting_use_stops_delivery() {
    // A source that drove its own relationship into `Revoking` has stopped
    // admitting use even though the row still exists.
    let (binding, service, target) = refs();
    let endpoint = observe(&admitted_endpoint_evidence(), BindingLifecycleState::Revoking);
    let observed = observed_for(&endpoint);
    let route = DeliveryRoute::for_endpoint(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        endpoint,
    )
    .expect("the relationship was once admitted");
    let frame = frame();
    let mut controller = TelemetryBindingController::new();

    let error = controller
        .reconcile(
            &binding,
            &service,
            &target,
            TelemetryBindingFrame {
                route: &route,
                observed: &observed,
                connection_id: 7,
                frame: &frame,
                canaries: &IdentityCanaries::default(),
                capacity_available: true,
            },
        )
        .expect_err("a revoking relationship stops delivery");
    let TelemetryControllerError::DeliveryRefused(refusal) = error else {
        panic!("a stopped relationship is a delivery refusal");
    };
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

// ---------------------------------------------------------------------------
// Scenario 3: the diagnostic names the resource and the enforcing stage and
// leaks no private path or secret.
// ---------------------------------------------------------------------------

#[test]
fn a_refusal_diagnostic_names_the_resource_and_the_refusal_stage() {
    let (mut route, endpoint_observed) = live_route();
    let credential = admitted_credential_evidence();
    let observed = observed_for_all(&[&admitted_endpoint_evidence(), &credential]);
    assert!(!endpoint_observed.is_empty());
    route.revoke(DeliverySource::Credential);
    let refusal = route
        .refusal(&observed)
        .expect("the revoked credential is refused");
    assert_eq!(
        refusal.resource(),
        &ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        "the refusal names the route's exact resource"
    );
    assert_eq!(refusal.source(), DeliverySource::Credential);
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
    let rendered = refusal.to_string();
    assert!(rendered.contains("telemetry.d2bus.org.TelemetryBinding/metrics"), "{rendered}");
    assert!(rendered.contains("revoke"), "the stage is named: {rendered}");
    assert!(rendered.contains("stale-authority"), "{rendered}");
}

#[test]
fn a_refusal_diagnostic_leaks_no_private_path_or_secret() {
    // The relationship is the one whose credential was admitted; its spec
    // and delivery material never reach the refusal, so neither does a
    // private path, a socket path, or a credential byte.
    let (mut route, observed) = live_route();
    let credential = admitted_credential_evidence();
    route.revoke(DeliverySource::Endpoint);
    let refusal = route
        .refusal(&observed)
        .expect("the revoked endpoint is refused");
    let rendered = format!("{refusal:?} {refusal}");
    for forbidden in [
        "/run/d2b",
        "/var/lib/d2b",
        "emitter.sock",
        "otlp.sock",
        "host-egress",
        "OTEL_EXPORTER_OTLP_HEADERS",
        "Authorization",
        "Bearer",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "the diagnostic leaked {forbidden}: {rendered}"
        );
    }
    // The credential relationship is not even named in the refusal: the
    // endpoint relationship is what stopped, and only that is reported.
    assert!(!rendered.contains("Credential/otlp"), "{rendered}");
    let _ = credential;
}

#[test]
fn the_status_projection_carries_only_the_closed_redacted_fields() {
    let (mut route, observed) = live_route();
    let status = route.status(&observed);
    assert!(status.admitting);
    assert_eq!(
        status.resource,
        "telemetry.d2bus.org.TelemetryBinding/metrics"
    );
    assert_eq!(status.source, "endpoint");
    assert_eq!(status.stage, None);
    assert_eq!(status.reason, None);
    // The rendered status is exactly the four closed fields.
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        serde_json::json!({
            "resource": "telemetry.d2bus.org.TelemetryBinding/metrics",
            "source": "endpoint",
            "stage": null,
            "reason": null,
            "admitting": true,
        }),
    );

    route.revoke(DeliverySource::Endpoint);
    let stopped = route.status(&observed);
    assert!(!stopped.admitting);
    assert_eq!(stopped.stage, Some(AdmissionStage::Revoke));
    assert_eq!(stopped.reason, Some(RefusalReason::StaleAuthority));
    let rendered = serde_json::to_string(&stopped).unwrap();
    for private in ["/run/", "/var/lib/", ".sock", "emitter.sock", "otlp.sock"] {
        assert!(
            !rendered.contains(private),
            "the status leaked a private path fragment {private}: {rendered}"
        );
    }
}

#[test]
fn a_route_exposes_no_authority_from_a_name_or_a_catalog_row() {
    // A delivery route is only constructible from admitted binding evidence.
    // A resource name, a service-catalog service string, or a runner role
    // cannot stand in for one: there is no constructor that takes them.
    let (route, observed) = live_route();
    assert_eq!(route.ingress(), Ingress::OtlpVsock);
    assert_eq!(
        route
            .endpoint_key()
            .map(|key| key.source_ref().to_canonical_string()),
        Some("Endpoint/ingest".to_owned()),
        "the route is keyed by the exact admitted source"
    );
    assert!(route.network().is_none());
    assert!(route.credential().is_none());
    assert!(route.evidence(DeliverySource::Network).is_none());
    assert!(route.admits_new_use(&observed));
    assert_eq!(DeliverySource::ALL.len(), 3);
    // The provider's own ambient-credential gate and the bridge role constant
    // are not admission inputs to any of this.
    let _: &str = d2b_provider_observability_otel::OTEL_HOST_BRIDGE_ROLE;
    assert!(d2b_provider_observability_otel::reject_ambient_credential_chain(["RUST_LOG"]).is_ok());
}

#[test]
fn the_route_reports_the_exact_admitted_evidence_it_rides() {
    let endpoint: BindingEvidence = admitted_endpoint_evidence();
    let network = admitted_network_evidence();
    let credential = admitted_credential_evidence();
    let observed = observed_for_all(&[&endpoint, &network, &credential]);
    let route = DeliveryRoute::new(
        ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
        Ingress::OtlpVsock,
        [
            (DeliverySource::Endpoint, endpoint.clone()),
            (DeliverySource::Network, network.clone()),
            (DeliverySource::Credential, credential.clone()),
        ],
    )
    .expect("all three relationships are admitted");
    assert_eq!(route.evidence(DeliverySource::Endpoint), Some(&endpoint));
    assert_eq!(route.evidence(DeliverySource::Network), Some(&network));
    assert_eq!(route.evidence(DeliverySource::Credential), Some(&credential));
    assert_eq!(route.network(), Some(&network));
    assert_eq!(route.credential(), Some(&credential));
    assert!(route.refusal(&observed).is_none());
    assert_eq!(route.revoked_sources().count(), 0);

    // Revoking one of the three stops the route; it does not narrow the
    // fence to the two survivors.
    let mut stopped = route.clone();
    stopped.revoke(DeliverySource::Network);
    let refusal = stopped.refusal(&observed).expect("a revoked source stops delivery");
    assert_eq!(refusal.source(), DeliverySource::Network);
    assert_eq!(refusal.stage(), AdmissionStage::Revoke);
    assert_eq!(
        stopped.revoked_sources().collect::<Vec<_>>(),
        vec![DeliverySource::Network],
        "the route keeps naming the source it lost"
    );
}
