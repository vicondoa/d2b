//! Guest-side target-control service tests.
//!
//! The service lives in `d2b-provider-guest`; the scene these tests build is
//! the daemon's - a real authenticated ComponentSession over a framed vsock
//! transport - which is why they run from the daemon crate that owns the
//! session runtime.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
    identity::{ReconnectGeneration, SessionPurpose},
};
use d2b_provider_guest::target_service::{
    GuestTargetEffect, GuestTargetEffectError, GuestTargetEffects, GuestTargetService,
    target_control_services,
};
use d2b_resource_runtime::guest_target::{
    GuestAdoption, GuestRealizeRequest, GuestTargetRuntime, TargetControlAssignment,
    TargetControlFrame, TargetControlRequest, TargetControlResponse, TargetResourceInstance,
    TargetInstanceState, TARGET_CONTROL_METHOD, TARGET_CONTROL_SERVICE, target_local_spec_digest,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::target::{TargetObservation, TargetRef};
use d2b_session::{
    HandshakeCredentials, Secret32, SessionEngine, SessionTtrpcClient, x25519_public_key,
};
use d2b_session_unix::FramedVsockTransport;
use d2bd_runtime::guest_mode::{
    BootIdentity, GUEST_COMPONENT_SESSION_PURPOSE, GuestIdentity, GuestRuntime,
};
use d2bd_runtime::target_runtime::AdmissionLimits;

const AUTHORITY_ZONE: &str = "work";
const TARGET_TYPE: &str = "Endpoint";

/// One target-local effect under test: records the exact spec bytes it
/// was asked to apply and answers the discovery question.
#[derive(Default)]
struct RecordingEffect {
    realized: StdMutex<Vec<(ResourceKey, Vec<u8>, String)>>,
    deleted: StdMutex<Vec<ResourceKey>>,
    adopted: StdMutex<Vec<ResourceKey>>,
    present: StdMutex<bool>,
    outcome: StdMutex<Option<GuestTargetEffectError>>,
    discovery: StdMutex<Option<GuestTargetEffectError>>,
}

impl RecordingEffect {
    fn new() -> Arc<Self> {
        Arc::new(Self { present: StdMutex::new(true), ..Self::default() })
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn absent() -> Arc<Self> {
        let effect = Self::new();
        *effect.present.lock().expect("present") = false;
        effect
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn failing(error: GuestTargetEffectError) -> Arc<Self> {
        let effect = Self::new();
        *effect.outcome.lock().expect("outcome") = Some(error);
        effect
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn realized(&self) -> Vec<(ResourceKey, Vec<u8>, String)> {
        self.realized.lock().expect("realized").clone()
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn deleted(&self) -> Vec<ResourceKey> {
        self.deleted.lock().expect("deleted").clone()
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn adopted(&self) -> Vec<ResourceKey> {
        self.adopted.lock().expect("adopted").clone()
    }
}

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[async_trait]
impl GuestTargetEffect for RecordingEffect {
    async fn realize(
        &self,
        request: &GuestRealizeRequest,
    ) -> Result<(), GuestTargetEffectError> {
        if let Some(error) = *self.outcome.lock().expect("outcome") { // async-gate-allow: test-support recorder lock
            return Err(error);
        }
        self.realized.lock().expect("realized").push(( // async-gate-allow: test-support recorder lock
            request.source().clone(),
            request.spec().to_vec(),
            request.spec_digest().to_owned(),
        ));
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn delete(&self, source: &ResourceKey) -> Result<(), GuestTargetEffectError> {
        self.deleted.lock().expect("deleted").push(source.clone()); // async-gate-allow: test-support recorder lock
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn adopt(&self, source: &ResourceKey) -> Result<bool, GuestTargetEffectError> {
        self.adopted.lock().expect("adopted").push(source.clone()); // async-gate-allow: test-support recorder lock
        if let Some(error) = *self.discovery.lock().expect("discovery") { // async-gate-allow: test-support recorder lock
            return Err(error);
        }
        Ok(*self.present.lock().expect("present")) // async-gate-allow: test-support recorder lock
    }
}

/// The service under test with its runtime kept for assertions.
struct Fixture {
    runtime: Arc<GuestTargetRuntime>,
    effect: Arc<RecordingEffect>,
    service: Arc<GuestTargetService>,
}

impl Fixture {
    fn new(effect: Arc<RecordingEffect>, generation: u64) -> Self {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        let effects = GuestTargetEffects::from([(
            ResourceTypeName::new(TARGET_TYPE),
            Arc::clone(&effect) as Arc<dyn GuestTargetEffect>,
        )]);
        let service = Arc::new(GuestTargetService::new(
            Arc::clone(&runtime),
            zone(),
            effects,
        ));
        service.bind_session(generation).expect("bind session");
        Self { runtime, effect, service }
    }

    fn recording(generation: u64) -> Self {
        Self::new(RecordingEffect::new(), generation)
    }

    async fn realize(&self, request: GuestRealizeRequest) -> TargetControlResponse {
        self.service
            .handle(TargetControlRequest::Realize(request))
            .await
    }

    async fn observe(&self, name: &str, generation: u64) -> TargetControlResponse {
        self.service
            .handle(TargetControlRequest::Observe {
                assignment: assignment(name, generation),
            })
            .await
    }

    async fn delete(&self, name: &str, generation: u64) -> TargetControlResponse {
        self.service
            .handle(TargetControlRequest::Delete {
                assignment: assignment(name, generation),
            })
            .await
    }

    async fn adopt(&self, name: &str, generation: u64) -> TargetControlResponse {
        self.service
            .handle(TargetControlRequest::Adopt {
                assignment: assignment(name, generation),
            })
            .await
    }

    fn instance(&self, name: &str) -> Option<TargetResourceInstance> {
        self.runtime.instance(&source(name))
    }
}

fn guest() -> TargetRef {
    TargetRef::guest("workload").expect("guest ref")
}

fn zone() -> ZoneId {
    ZoneId::parse(AUTHORITY_ZONE).expect("zone")
}

fn source(name: &str) -> ResourceKey {
    ResourceKey::new(AUTHORITY_ZONE, TARGET_TYPE, name)
}

fn spec() -> Vec<u8> {
    br#"{"endpoint":{"kind":"relay"}}"#.to_vec()
}

fn assignment(name: &str, generation: u64) -> TargetControlAssignment {
    TargetControlAssignment::new(source(name), [7; 16], 3, generation)
}

fn realize_request(name: &str, generation: u64, spec: Vec<u8>) -> GuestRealizeRequest {
    GuestRealizeRequest::new(
        assignment(name, generation),
        spec.clone(),
        target_local_spec_digest(&spec),
        format!("/run/d2b/{name}.sock"),
    )
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn realize_records_once_applies_the_exact_spec_and_reports_ready() {
    let f = Fixture::recording(1);
    let request = realize_request("relay", 1, spec());

    let first = f.realize(request.clone()).await;
    let TargetControlResponse::Realized { realization } = &first else {
        panic!("first realize: {first:?}");
    };
    assert_eq!(realization.state(), TargetInstanceState::Ready, "the effect is serving");
    assert_eq!(realization.local_handle(), "/run/d2b/relay.sock");
    assert_eq!(realization.spec_digest(), request.spec_digest());

    let second = f.realize(realize_request("relay", 1, spec())).await;
    let TargetControlResponse::Realized { realization } = &second else {
        panic!("second realize: {second:?}");
    };
    assert_eq!(realization.source(), &source("relay"));
    assert_eq!(f.runtime.instances().len(), 1, "one realization per host-owned source");
    assert_eq!(f.effect.realized().len(), 2, "the effect is re-applied idempotently");
    assert_eq!(f.effect.realized()[0].1, spec(), "the exact bytes reach the effect");
    assert_eq!(f.effect.realized()[0].2, request.spec_digest());
    assert_eq!(
        f.observe("relay", 1).await,
        TargetControlResponse::Observed(TargetObservation::Ready { session_generation: 1 })
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_stale_generation_performs_no_effect_and_answers_session_unavailable() {
    let f = Fixture::recording(2);

    assert_eq!(
        f.realize(realize_request("relay", 1, spec())).await,
        TargetControlResponse::SessionUnavailable
    );
    assert_eq!(
        f.observe("relay", 1).await,
        TargetControlResponse::SessionUnavailable
    );
    assert_eq!(f.delete("relay", 1).await, TargetControlResponse::SessionUnavailable);
    assert_eq!(f.adopt("relay", 1).await, TargetControlResponse::SessionUnavailable);
    assert!(f.runtime.instances().is_empty(), "no stale request created state");
    assert!(f.effect.realized().is_empty(), "the effect never saw the stale spec");
    assert!(f.effect.deleted().is_empty());
    assert!(f.effect.adopted().is_empty());
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_live_realization_survives_the_reconnect_and_the_old_generation_stops_working() {
    let f = Fixture::recording(1);
    f.realize(realize_request("relay", 1, spec())).await;

    f.service.bind_session(2).expect("reconnect generation");
    assert_eq!(
        f.observe("relay", 1).await,
        TargetControlResponse::SessionUnavailable,
        "the lost generation cannot observe its old realization"
    );
    assert!(f.instance("relay").is_some(), "desired realization state is untouched");
    assert_eq!(
        f.observe("relay", 2).await,
        TargetControlResponse::Observed(TargetObservation::Ready { session_generation: 1 }),
        "the live generation sees the stored realization; only adoption re-binds it"
    );
    let adopted = f.adopt("relay", 2).await;
    let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &adopted else {
        panic!("adopt: {adopted:?}");
    };
    assert_eq!(instance.session_generation(), 2, "adoption re-binds to the live generation");
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn delete_removes_only_its_own_instance_and_its_own_effect() {
    let f = Fixture::recording(1);
    f.realize(realize_request("relay", 1, spec())).await;
    f.realize(realize_request("other", 1, spec())).await;

    assert_eq!(f.delete("relay", 1).await, TargetControlResponse::Deleted);
    assert_eq!(f.effect.deleted(), vec![source("relay")]);
    assert!(f.instance("relay").is_none());
    assert!(f.instance("other").is_some(), "the sibling realization stays");
    assert_eq!(f.runtime.instances().len(), 1);

    // Deleting an absent source is still `deleted` and runs no effect.
    assert_eq!(f.delete("relay", 1).await, TargetControlResponse::Deleted);
    assert_eq!(f.effect.deleted(), vec![source("relay")], "an absent delete runs no effect");
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_foreign_zone_key_is_refused_without_creating_state() {
    let f = Fixture::recording(1);
    let foreign = GuestRealizeRequest::new(
        TargetControlAssignment::new(
            ResourceKey::new("other-zone", TARGET_TYPE, "relay"),
            [7; 16],
            3,
            1,
        ),
        spec(),
        target_local_spec_digest(&spec()),
        "/run/d2b/relay.sock",
    );
    assert_eq!(
        f.service.handle(TargetControlRequest::Realize(foreign)).await,
        TargetControlResponse::SessionUnavailable
    );
    assert!(f.runtime.instances().is_empty());
    assert!(f.effect.realized().is_empty());
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_key_with_no_target_local_effect_is_refused_without_creating_state() {
    let f = Fixture::recording(1);
    let unknown = GuestRealizeRequest::new(
        TargetControlAssignment::new(
            ResourceKey::new(AUTHORITY_ZONE, "Role", "admin"),
            [7; 16],
            3,
            1,
        ),
        spec(),
        target_local_spec_digest(&spec()),
        "/run/d2b/admin.sock",
    );
    assert_eq!(
        f.service.handle(TargetControlRequest::Realize(unknown)).await,
        TargetControlResponse::SessionUnavailable
    );
    assert!(f.runtime.instances().is_empty(), "an unknown type never becomes a realization");
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_substituted_spec_is_refused_before_the_effect_sees_it() {
    let f = Fixture::recording(1);
    let request = realize_request("relay", 1, spec());
    let substituted = GuestRealizeRequest::new(
        request.assignment().clone(),
        br#"{"endpoint":{"kind":"hostile"}}"#.to_vec(),
        request.spec_digest().to_owned(),
        request.local_handle().to_owned(),
    );
    assert_eq!(
        f.service.handle(TargetControlRequest::Realize(substituted)).await,
        TargetControlResponse::SessionUnavailable
    );
    assert!(f.runtime.instances().is_empty(), "nothing is recorded for a mismatched spec");
    assert!(f.effect.realized().is_empty(), "the effect never sees a mismatched spec");

    // The same bytes with the right commitment are accepted: the refusal
    // is the commitment, not the transport.
    assert!(matches!(
        f.realize(request).await,
        TargetControlResponse::Realized { .. }
    ));
    assert_eq!(f.effect.realized().len(), 1);
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_failed_effect_leaves_the_realization_realizing_instead_of_ready() {
    let f = Fixture::new(RecordingEffect::failing(GuestTargetEffectError::Unavailable), 1);
    let response = f.realize(realize_request("relay", 1, spec())).await;
    let TargetControlResponse::Realized { realization } = &response else {
        panic!("realize: {response:?}");
    };
    assert_eq!(realization.state(), TargetInstanceState::Realizing);
    assert_eq!(
        f.observe("relay", 1).await,
        TargetControlResponse::Observed(TargetObservation::Realizing { session_generation: 1 })
    );
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn adoption_rebinds_and_recovers_the_target_local_effect() {
    let f = Fixture::recording(1);
    f.realize(realize_request("relay", 1, spec())).await;
    f.service.bind_session(2).expect("reconnect generation");

    let response = f.adopt("relay", 2).await;
    let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &response else {
        panic!("adopt: {response:?}");
    };
    assert_eq!(instance.session_generation(), 2, "adoption re-binds, never inherits");
    assert_eq!(instance.state(), TargetInstanceState::Ready, "the present effect re-serves");
    assert_eq!(f.effect.adopted(), vec![source("relay")]);
    assert_eq!(f.runtime.instances().len(), 1);
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn adoption_reports_missing_when_the_target_local_effect_is_gone() {
    let f = Fixture::new(RecordingEffect::absent(), 1);
    f.realize(realize_request("relay", 1, spec())).await;
    f.service.bind_session(2).expect("reconnect generation");

    assert_eq!(
        f.adopt("relay", 2).await,
        TargetControlResponse::Adopted(GuestAdoption::Missing),
        "an absent effect is never reported as adopted"
    );
    assert!(f.instance("relay").is_none(), "the stale record is forgotten");
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn adoption_reports_missing_when_the_effect_cannot_confirm_the_realization() {
    let effect = RecordingEffect::new();
    *effect.discovery.lock().expect("discovery") = Some(GuestTargetEffectError::Unavailable); // async-gate-allow: test-support recorder lock
    let f = Fixture::new(Arc::clone(&effect), 1);
    f.realize(realize_request("relay", 1, spec())).await;
    f.service.bind_session(2).expect("reconnect generation");

    assert_eq!(
        f.adopt("relay", 2).await,
        TargetControlResponse::Adopted(GuestAdoption::Missing),
        "an unconfirmed realization is never reported as adopted"
    );
    assert!(f.instance("relay").is_none(), "the unconfirmed record is forgotten");
}

#[test]
fn the_registered_service_publishes_exactly_the_protocol_method() {
    let f = Fixture::recording(1);
    let services = target_control_services(Arc::clone(&f.service));
    assert_eq!(services.len(), 1);
    let surface = services.get(TARGET_CONTROL_SERVICE).expect("registered service");
    assert_eq!(
        surface.methods.keys().collect::<Vec<_>>(),
        vec![TARGET_CONTROL_METHOD],
        "the service publishes exactly the one protocol method"
    );
    assert!(surface.streams.is_empty());
}

#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn frames_round_trip_over_a_real_authenticated_session() {
    let guest_runtime = GuestRuntime::new(
        guest_identity(1),
        "/run/d2b/guest-broker.sock".into(),
        997,
        AdmissionLimits::guest_default(),
    )
    .await
    .expect("Guest runtime");
    let parent_private_bytes = [2_u8; 32];
    let guest_private_bytes = [3_u8; 32];
    let parent_public = x25519_public_key(&parent_private_bytes).expect("parent public key");
    let guest_public = x25519_public_key(&guest_private_bytes).expect("guest public key");
    let parent_private = Secret32::new(parent_private_bytes).expect("parent private key");
    let guest_private = Secret32::new(guest_private_bytes).expect("guest private key");
    let parent_policy =
        d2b_contracts_zone_session::v3::component_session::EndpointPolicyIdentity::from(
            &guest_identity(1).endpoint_policy(),
        );
    let (left, right) = tokio::io::duplex(64 * 1024);
    let parent = tokio::spawn(async move {
        SessionEngine::establish_initiator_with_generation_discovery(
            FramedVsockTransport::new(left),
            parent_policy,
            HandshakeCredentials::Kk {
                local_private: parent_private,
                remote_public: guest_public,
            },
            std::time::Instant::now(),
        )
        .await
    });
    let (session, lease) = guest_runtime
        .establish_component_session(
            FramedVsockTransport::new(right),
            guest_private,
            parent_public,
        )
        .await
        .expect("guest accepts the parent session");
    let generation = lease.generation();
    let parent_engine = parent.await.expect("parent task").expect("parent session");

    let f = Fixture::new(RecordingEffect::new(), generation);
    let serving = tokio::spawn(
        session
            .into_ttrpc_handle()
            .serve_ttrpc_services(target_control_services(Arc::clone(&f.service))),
    );
    let driver: Arc<dyn d2b_session::ComponentSessionDriver> =
        Arc::new(parent_engine.into_driver());
    let transport = SessionTtrpcClient::new(driver);
    let client = transport.client();

    // Realize over the wire: the service applies the exact spec and the
    // reply carries the ready realization back.
    let request = realize_request("relay", generation, spec());
    let response = call(&client, TargetControlRequest::Realize(request.clone())).await;
    let TargetControlResponse::Realized { realization } = &response else {
        panic!("wire realize: {response:?}");
    };
    assert_eq!(realization.state(), TargetInstanceState::Ready);
    assert_eq!(realization.source(), &source("relay"));
    assert_eq!(f.effect.realized()[0].1, spec());

    // A stale generation over the same wire is fenced by the daemon's
    // live generation: SessionUnavailable, no effect, no state.
    let stale = TargetControlRequest::Realize(GuestRealizeRequest::new(
        assignment("stale", generation + 7),
        spec(),
        target_local_spec_digest(&spec()),
        "/run/d2b/stale.sock",
    ));
    assert_eq!(call(&client, stale).await, TargetControlResponse::SessionUnavailable);
    assert!(f.instance("stale").is_none());

    // Observe, adopt and delete round-trip on the same registration.
    assert_eq!(
        call(
            &client,
            TargetControlRequest::Observe { assignment: assignment("relay", generation) }
        )
        .await,
        TargetControlResponse::Observed(TargetObservation::Ready {
            session_generation: generation
        })
    );
    assert!(matches!(
        call(
            &client,
            TargetControlRequest::Adopt { assignment: assignment("relay", generation) }
        )
        .await,
        TargetControlResponse::Adopted(GuestAdoption::Adopted(_))
    ));
    assert_eq!(
        call(
            &client,
            TargetControlRequest::Delete { assignment: assignment("relay", generation) }
        )
        .await,
        TargetControlResponse::Deleted
    );
    assert_eq!(f.effect.deleted(), vec![source("relay")]);

    // A payload that is not a frame, and a frame with a foreign protocol
    // token, are both refused without touching the service.
    assert!(raw_call(&client, b"not-a-frame".to_vec()).await.is_err());
    let mut foreign: serde_json::Value = serde_json::from_slice(
        &TargetControlFrame::new(TargetControlRequest::Observe {
            assignment: assignment("relay", generation),
        })
        .encode(),
    )
    .expect("frame json");
    foreign["protocol"] = serde_json::Value::String("d2b.target-control.v0".to_owned());
    assert!(
        raw_call(&client, serde_json::to_vec(&foreign).expect("foreign frame"))
            .await
            .is_err()
    );

    serving.abort();
    drop(lease);
    let _ = serving.await;
}

/// One encoded round trip over the real service registration.
async fn call(
    client: &ttrpc::r#async::Client,
    request: TargetControlRequest,
) -> TargetControlResponse {
    let payload = TargetControlFrame::new(request).encode();
    let response = raw_call(client, payload).await.expect("target-control reply");
    TargetControlResponse::decode(&response).expect("decodable reply")
}

async fn raw_call(
    client: &ttrpc::r#async::Client,
    payload: Vec<u8>,
) -> ttrpc::Result<Vec<u8>> {
    let response = client
        .request(ttrpc::Request {
            service: TARGET_CONTROL_SERVICE.to_owned(),
            method: TARGET_CONTROL_METHOD.to_owned(),
            payload,
            ..Default::default()
        })
        .await?;
    Ok(response.payload)
}

fn guest_identity(generation: u64) -> GuestIdentity {
    GuestIdentity::new(
        ResourceRef::parse("Guest/workload").expect("Guest ref"),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("Guest UID"),
        zone(),
        BootIdentity::from_kernel_boot_id("guest-target-service-test").expect("boot identity"),
        SessionPurpose::parse(GUEST_COMPONENT_SESSION_PURPOSE).expect("purpose"),
        SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).expect("schema"),
        ReconnectGeneration::new(generation).expect("generation"),
        1,
        1,
        1,
    )
    .expect("Guest identity")
}
