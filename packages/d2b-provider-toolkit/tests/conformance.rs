//! Focused Provider toolkit conformance over the v3 registry and RPC seam.

use std::time::Duration;

use d2b_provider::{
    AdmissionOptions, CancellationToken, ProviderClass, ProviderMethodName,
    ProviderRegistryBuilder, ProviderRuntimeError, rpc::RpcProviderProxy,
};
use d2b_provider_toolkit::{
    FakeProvider, Fixture, GeneratedProviderServiceServer, ProviderValues, ServerError,
};

#[tokio::test]
async fn fake_provider_round_trip_uses_the_exact_placement_binding() {
    let first = Fixture::new(ProviderClass::Runtime, 0).expect("first fixture");
    let second = Fixture::new(ProviderClass::Runtime, 1).expect("second fixture");
    let mut builder =
        ProviderRegistryBuilder::new(first.zone().clone(), first.descriptor.registry_generation());
    builder
        .register_instance(first.descriptor.clone(), ())
        .expect("first descriptor");
    builder
        .register_instance(second.descriptor.clone(), ())
        .expect("second descriptor");
    let registry = builder.finish().expect("registry");
    let method = ProviderMethodName::parse("health").expect("health method");
    let first_admitted = registry
        .admit(AdmissionOptions {
            identity: first.session_identity().expect("first identity"),
            expected_method: method.clone(),
            deadline_after: Duration::from_secs(1),
            caller_cancellation: CancellationToken::new(),
        })
        .expect("first admission");
    let second_admitted = registry
        .admit(AdmissionOptions {
            identity: second.session_identity().expect("second identity"),
            expected_method: method.clone(),
            deadline_after: Duration::from_secs(1),
            caller_cancellation: CancellationToken::new(),
        })
        .expect("second admission");

    let provider = FakeProvider::new(first.clone());
    let proxy = RpcProviderProxy::new_with_descriptor(first.descriptor.clone(), provider.clone())
        .expect("placement-bound proxy");
    let response = proxy
        .dispatch(
            &first_admitted.context,
            first.call(method.clone()).expect("call"),
        )
        .await
        .expect("round trip");
    assert_eq!(response.payload().get("state").is_some(), true);
    assert_eq!(
        proxy
            .dispatch(&second_admitted.context, first.call(method).expect("call"),)
            .await
            .expect_err("foreign placement must fail"),
        ProviderRuntimeError::SessionIdentityMismatch
    );
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn fake_provider_conformance_keeps_health_inspection_and_observability_closed() {
    let fixture = Fixture::new(ProviderClass::Observability, 0).expect("fixture");
    let values = ProviderValues::new(&fixture.descriptor, fixture.now_unix_ms).expect("values");
    assert_eq!(
        values.observability().sequence(),
        &["health", "inspect", "observability"]
    );
    FakeProvider::new(fixture)
        .conformance_sequence()
        .expect("closed sequence");
}

#[tokio::test]
async fn generated_server_shutdown_refuses_new_work_after_drain() {
    let fixture = Fixture::new(ProviderClass::Runtime, 0).expect("fixture");
    let server = GeneratedProviderServiceServer::new(FakeProvider::new(fixture));
    let permit = server.admit_request().expect("request permit");
    let drain = server.shutdown(Duration::from_millis(1)).await;
    assert!(!drain);
    drop(permit);
    assert!(server.shutdown(Duration::from_millis(100)).await);
    assert_eq!(
        server.admit_request().expect_err("server is retired"),
        ServerError::NotAccepting
    );
}
