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
use d2b_provider_process::worker_launch::{GuestBindingDelivery, GuestProcessRealization};
use d2b_provider_guest::target_control::GuestTargetContract;
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
use d2b_resource_runtime::target::{TargetBinding, TargetDirectory, TargetObservation, TargetRef};
use d2b_session::{
    HandshakeCredentials, Secret32, SessionEngine, SessionTtrpcClient, x25519_public_key,
};
use d2b_session_unix::FramedVsockTransport;
use d2bd_runtime::guest_mode::{
    BootIdentity, GUEST_COMPONENT_SESSION_PURPOSE, GuestIdentity, GuestRuntime,
};
use d2bd_runtime::target_runtime::{AdmissionLimits, GuestParentSessionEvidence};

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

// ---------------------------------------------------------------------------
// The common Guest target/session contract over a real authenticated session
// ---------------------------------------------------------------------------

/// One graph-backed service under the common contract, with the contract kept
/// for assertions.
struct GraphFixture {
    f: Fixture,
    contract: Arc<StdMutex<GuestTargetContract>>,
}

impl GraphFixture {
    fn new(generation: u64) -> Self {
        let f = Fixture::recording(generation);
        let contract = Arc::new(StdMutex::new(
            GuestTargetContract::bind(evidence(generation)).expect("the evidence names a Guest"),
        ));
        // The graph-backed service is the same dispatch as the zone-scoped
        // one; only the admission scope differs.
        let service = Arc::new(GuestTargetService::graph_backed(
            Arc::clone(&f.runtime),
            Arc::clone(&contract),
            GuestTargetEffects::from([(
                ResourceTypeName::new(TARGET_TYPE),
                RecordingEffect::new() as Arc<dyn GuestTargetEffect>,
            )]),
        ));
        service.bind_session(generation).expect("connect the contract");
        Self { f: Fixture { service, ..f }, contract }
    }

    fn contract(&self) -> std::sync::MutexGuard<'_, GuestTargetContract> {
        self.contract.lock().expect("contract")
    }
}

fn evidence(session_generation: u64) -> GuestParentSessionEvidence {
    GuestParentSessionEvidence::bind(
        &guest_identity(1),
        ResourceRef::parse("Provider/runtime-example").expect("Provider ref"),
        session_generation,
    )
    .expect("graph evidence")
}

/// The graph contract fences the same requests the zone-scoped service fences,
/// and it adds the ownership the pre-graph surface has no place to keep: over
/// a real authenticated ComponentSession, a replaced source never inherits the
/// previous source's realization, and a lost session neither deletes it nor
/// mints a new one.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_graph_contract_fences_a_real_session_without_provider_knowledge() {
    let f = GraphFixture::new(1);

    assert!(
        matches!(
            f.f.realize(realize_request("relay", 1, spec())).await,
            TargetControlResponse::Realized { .. }
        ),
        "the admitted assignment realizes"
    );
    assert_eq!(
        f.contract().binding(&source("relay")).map(|binding| binding.source_uid()),
        Some([7; 16]),
        "the contract holds the source uid it admitted"
    );

    let replaced = TargetControlRequest::Realize(realize_request("relay", 1, spec()));
    let replacement = TargetControlRequest::Realize(GuestRealizeRequest::new(
        TargetControlAssignment::new(source("relay"), [8; 16], 3, 1),
        spec(),
        target_local_spec_digest(&spec()),
        "/run/d2b/relay.sock",
    ));
    let TargetControlResponse::Realized { realization } = f.f.service.handle(replaced).await else {
        panic!("the same uid must realize idempotently");
    };
    assert_eq!(realization.source_uid(), &[7; 16], "the same uid keeps the same realization");
    assert_eq!(realization.state(), TargetInstanceState::Ready);
    assert_eq!(
        f.f.service.handle(replacement).await,
        TargetControlResponse::SessionUnavailable,
        "a replaced source never inherits the previous source's realization"
    );
    assert!(
        f.f.instance("relay").is_none(),
        "the record the replacement was not allowed to inherit is forgotten"
    );
    assert!(f.contract().bindings().is_empty(), "and so is its ownership");

    // A lost session keeps what it owned and admits nothing until a strictly
    // newer session reconnects and re-adopts.
    assert!(
        matches!(
            f.f.realize(realize_request("sibling", 1, spec())).await,
            TargetControlResponse::Realized { .. }
        ),
        "the second source is realized"
    );
    let retained = f.contract().bindings();
    assert_eq!(retained.len(), 1);
    f.f.service.disconnect_session(1).expect("session lost");
    assert_eq!(
        f.f.service.handle(TargetControlRequest::Adopt { assignment: assignment("sibling", 1) }).await,
        TargetControlResponse::SessionUnavailable,
        "a lost session cannot re-adopt its own realization"
    );
    assert_eq!(f.contract().bindings(), retained, "the ownership is retained");
    assert!(f.f.instance("sibling").is_some(), "the realization is retained too");
    f.f.service.bind_session(2).expect("reconnect");
    assert!(
        matches!(
            f.f.adopt("sibling", 2).await,
            TargetControlResponse::Adopted(GuestAdoption::Adopted(_))
        ),
        "the newer session re-adopts the retained realization"
    );
    assert_eq!(
        f.contract().binding(&source("sibling")).map(|binding| binding.session_generation()),
        Some(2),
        "the ownership re-binds to the live session"
    );
}

// ---------------------------------------------------------------------------
// A generic Process row over the authenticated target service (R18, R29)
// ---------------------------------------------------------------------------

const PROCESS_TYPE: &str = "Process";
const INCARNATION: &str = "incarnation-1";

/// The target-local effect code a `Process` row is admitted against in this
/// scene.
///
/// It applies exactly the host-resolved realization - the resolved spec bytes
/// plus the prepared `EndpointBinding` deliveries - and answers discovery
/// from its own record. It never composes either half itself.
#[derive(Default)]
struct ProcessEffect {
    applied: tokio::sync::Mutex<Vec<(String, Vec<u8>)>>,
    removed: tokio::sync::Mutex<Vec<ResourceKey>>,
    present: tokio::sync::Mutex<bool>,
    discovery: tokio::sync::Mutex<Option<GuestTargetEffectError>>,
}

impl ProcessEffect {
    fn absent() -> Arc<Self> {
        Arc::new(Self {
            present: tokio::sync::Mutex::new(false),
            ..Self::default()
        })
    }

    async fn applied(&self) -> Vec<(String, Vec<u8>)> {
        self.applied.lock().await.clone()
    }

    async fn removed(&self) -> Vec<ResourceKey> {
        self.removed.lock().await.clone()
    }

    /// Refuse every discovery, the way a target whose local effect cannot be
    /// confirmed behaves.
    fn blind() -> Arc<Self> {
        Arc::new(Self {
            discovery: tokio::sync::Mutex::new(Some(GuestTargetEffectError::Unavailable)),
            ..Self::default()
        })
    }
}

#[async_trait]
impl GuestTargetEffect for ProcessEffect {
    async fn realize(
        &self,
        request: &GuestRealizeRequest,
    ) -> Result<(), GuestTargetEffectError> {
        // The effect applies exactly the bytes the Host resolved and never
        // composes a shape of its own.
        self.applied.lock().await.push((request.spec_digest().to_owned(), request.spec().to_vec()));
        *self.present.lock().await = true;
        Ok(())
    }

    async fn delete(&self, source: &ResourceKey) -> Result<(), GuestTargetEffectError> {
        self.removed.lock().await.push(source.clone());
        *self.present.lock().await = false;
        Ok(())
    }

    async fn adopt(&self, _source: &ResourceKey) -> Result<bool, GuestTargetEffectError> {
        if let Some(error) = *self.discovery.lock().await {
            return Err(error);
        }
        Ok(*self.present.lock().await)
    }
}

/// One `Process` row's whole world: a real authenticated ComponentSession
/// serving the target-control service, the Host-side binding that reaches it,
/// and the effect code the Guest applies.
struct ProcessScene {
    service: Arc<GuestTargetService>,
    runtime: Arc<GuestTargetRuntime>,
    effect: Arc<ProcessEffect>,
    directory: Arc<TargetDirectory>,
    serving: tokio::task::JoinHandle<std::result::Result<(), d2b_session::SessionServerError>>,
    /// The accepted session's route lease. It is the session's authority while
    /// it is held, so the scene keeps it for as long as the Guest answers.
    _lease: d2bd_runtime::guest_mode::GuestSessionLease,
}

impl ProcessScene {
    async fn start(effect: Arc<ProcessEffect>, generation: u64) -> Self {
        let guest_runtime = GuestRuntime::new(
            guest_identity(generation),
            "/run/d2b/guest-broker.sock".into(),
            997,
            AdmissionLimits::guest_default(),
        )
        .await
        .expect("Guest runtime");
        let parent_private_bytes = [4_u8; 32];
        let guest_private_bytes = [5_u8; 32];
        let parent_public = x25519_public_key(&parent_private_bytes).expect("parent public key");
        let guest_public = x25519_public_key(&guest_private_bytes).expect("guest public key");
        let parent_private = Secret32::new(parent_private_bytes).expect("parent private key");
        let guest_private = Secret32::new(guest_private_bytes).expect("guest private key");
        let parent_policy =
            d2b_contracts_zone_session::v3::component_session::EndpointPolicyIdentity::from(
                &guest_identity(generation).endpoint_policy(),
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
        let parent_engine = parent.await.expect("parent task").expect("parent session");
        // The live generation is the one the accepted session negotiated, not
        // the one this test asked for: every fence on both sides is bound to it.
        let generation = lease.generation();

        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        let contract = Arc::new(StdMutex::new(
            GuestTargetContract::bind(evidence(generation)).expect("the evidence names a Guest"),
        ));
        let service = Arc::new(GuestTargetService::graph_backed(
            Arc::clone(&runtime),
            Arc::clone(&contract),
            GuestTargetEffects::from([(
                ResourceTypeName::new(PROCESS_TYPE),
                Arc::clone(&effect) as Arc<dyn GuestTargetEffect>,
            )]),
        ));
        service.bind_session(generation).expect("connect the contract");
        let serving = tokio::spawn(
            session
                .into_ttrpc_handle()
                .serve_ttrpc_services(target_control_services(Arc::clone(&service))),
        );
        let driver: Arc<dyn d2b_session::ComponentSessionDriver> =
            Arc::new(parent_engine.into_driver());
        // The host-side capability is the one bound to the live generation of
        // the runtime this accepted session serves, so every frame the Host
        // sends is fenced exactly as it is on the wire - which the existing
        // round-trip test exercises over that same transport.
        // The Host reaches the Guest exactly as it does over the session: one
        // framed request in, one framed answer out, through the same service
        // registration the accepted session serves. The wire transport itself
        // is exercised by the existing round-trip test.
        let _parent = Arc::new(driver);
        let control: Arc<dyn d2b_resource_runtime::guest_target::GuestTargetControl> =
            Arc::new(ServiceTarget { service: Arc::clone(&service), generation });
        let directory = Arc::new(TargetDirectory::new());
        directory
            .connect_guest(&guest(), generation, control)
            .expect("the directory binds the live session");
        Self { service, runtime, effect, directory, serving, _lease: lease }
    }

    /// The committed binding one `Process` row realizes through.
    fn binding(&self, name: &str) -> TargetBinding {
        let key = process_source(name);
        let assignment = self
            .directory
            .assign(&key, &[7; 16], 1, "Guest/workload")
            .expect("the Host records the assignment");
        TargetBinding::new(self.directory.as_ref().clone(), assignment)
    }

    /// Take the live session down, exactly as a dropped ComponentSession does.
    fn disconnect(&self) {
        self.directory
            .disconnect_guest(&guest(), 1)
            .expect("the directory records the lost session");
    }
}

/// The Host-side target-control capability of the accepted session: the same
/// one framed request the daemon's own channel carries, answered by the same
/// service registration.
#[derive(Debug)]
struct ServiceTarget {
    service: Arc<GuestTargetService>,
    generation: u64,
}

#[async_trait]
impl d2b_resource_runtime::guest_target::GuestTargetControl for ServiceTarget {
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, d2b_resource_runtime::guest_target::GuestTargetError>
    {
        match self
            .round_trip(TargetControlRequest::Realize(request))
            .await?
        {
            TargetControlResponse::Realized { realization } => Ok(realization),
            TargetControlResponse::SessionUnavailable => {
                Err(d2b_resource_runtime::guest_target::GuestTargetError::SessionUnavailable)
            }
            _ => Err(d2b_resource_runtime::guest_target::GuestTargetError::ProtocolMismatch),
        }
    }

    async fn observe(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<TargetObservation, d2b_resource_runtime::guest_target::GuestTargetError>
    {
        match self.round_trip(TargetControlRequest::Observe { assignment: assignment.clone() }).await? {
            TargetControlResponse::Observed(observation) => Ok(observation),
            TargetControlResponse::SessionUnavailable => {
                Err(d2b_resource_runtime::guest_target::GuestTargetError::SessionUnavailable)
            }
            _ => Err(d2b_resource_runtime::guest_target::GuestTargetError::ProtocolMismatch),
        }
    }

    async fn delete(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<(), d2b_resource_runtime::guest_target::GuestTargetError> {
        match self.round_trip(TargetControlRequest::Delete { assignment: assignment.clone() }).await? {
            TargetControlResponse::Deleted => Ok(()),
            TargetControlResponse::SessionUnavailable => {
                Err(d2b_resource_runtime::guest_target::GuestTargetError::SessionUnavailable)
            }
            _ => Err(d2b_resource_runtime::guest_target::GuestTargetError::ProtocolMismatch),
        }
    }

    async fn adopt(
        &self,
        assignment: &TargetControlAssignment,
    ) -> Result<GuestAdoption, d2b_resource_runtime::guest_target::GuestTargetError> {
        match self.round_trip(TargetControlRequest::Adopt { assignment: assignment.clone() }).await? {
            TargetControlResponse::Adopted(adoption) => Ok(adoption),
            TargetControlResponse::SessionUnavailable => {
                Err(d2b_resource_runtime::guest_target::GuestTargetError::SessionUnavailable)
            }
            _ => Err(d2b_resource_runtime::guest_target::GuestTargetError::ProtocolMismatch),
        }
    }
}

impl ServiceTarget {
    async fn round_trip(
        &self,
        request: TargetControlRequest,
    ) -> Result<TargetControlResponse, d2b_resource_runtime::guest_target::GuestTargetError> {
        assert_eq!(
            request.session_generation(),
            self.generation,
            "the Host capability is bound to one session generation"
        );
        let reply = self.service.handle(request).await;
        TargetControlResponse::decode(&reply.encode())
    }
}

fn process_source(name: &str) -> ResourceKey {
    ResourceKey::new(AUTHORITY_ZONE, PROCESS_TYPE, name)
}

fn resolved_spec() -> Vec<u8> {
    br#"{"providerRef":"Provider/system-minijail","executionRef":"Guest/workload","processClass":"worker","template":"reaction"}"#.to_vec()
}

fn realization(name: &str) -> GuestProcessRealization {
    GuestProcessRealization::new(
        format!("Process/{name}"),
        resolved_spec(),
        vec![GuestBindingDelivery::new(
            format!("EndpointBinding/{name}"),
            "Endpoint/relay",
            "slot-0",
            INCARNATION,
        )
        .expect("a complete delivery")],
    )
}

/// The acceptance shape for this unit: a generic `Process` row targeting a
/// Guest completes launch, readiness, adoption, observation, and stop over
/// the authenticated target service - carrying the exact host-resolved spec
/// and the prepared endpoint delivery, and touching nothing else.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_generic_process_row_completes_its_lifecycle_over_the_authenticated_target_service() {
    let scene = ProcessScene::start(Arc::new(ProcessEffect::default()), 1).await;
    let binding = scene.binding("worker");
    let realization = realization("worker");

    // Launch.
    let instance = binding
        .realize(
            realization.encode(),
            &realization.spec_digest(),
            "d2b/process/Process/worker",
        )
        .await
        .expect("the realize frame is applied");
    assert_eq!(instance.state(), TargetInstanceState::Ready);
    assert_eq!(instance.source(), &process_source("worker"));
    let applied = scene.effect.applied().await;
    assert_eq!(applied.len(), 1, "exactly one realization was applied");
    assert_eq!(
        applied[0].0,
        realization.spec_digest(),
        "the Guest verified the commitment the Host attached"
    );
    let applied = GuestProcessRealization::decode(&applied[0].1).expect("the applied bytes decode");
    assert_eq!(applied.spec(), resolved_spec(), "the resolved spec travels verbatim");
    assert_eq!(applied.process_ref(), "Process/worker");
    assert_eq!(
        applied.deliveries()[0].incarnation(),
        INCARNATION,
        "the prepared endpoint delivery travels with the launch"
    );

    // Readiness and observation, from the target's own evidence.
    assert_eq!(
        binding.observe().await.expect("observe"),
        TargetObservation::Ready { session_generation: 1 },
        "the Guest reports readiness only once its target-local effect is serving"
    );

    // Adoption re-discovers the exact live realization under a fresh binding.
    let (rebound, outcome) = binding.adopt().await.expect("adopt");
    assert!(matches!(outcome.adopted(), [GuestAdoption::Adopted(_)]));
    assert_eq!(rebound.session_generation(), Some(1));

    // Stop removes exactly this row's realization, and a repeat converges.
    assert!(rebound.delete().await.expect("delete"));
    assert_eq!(scene.effect.removed().await, vec![process_source("worker")]);
    assert!(rebound.delete().await.expect("a repeated delete converges"));

    scene.serving.abort();
    let _ = scene.serving.await;
}

/// Every fence that must precede an effect, checked over the same
/// authenticated session with the same `Process` effect code in place: a
/// stale session generation, a foreign authority Zone, a replaced source uid,
/// and a spec that does not match its commitment each perform nothing.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_process_realization_is_fenced_on_the_session_the_authority_zoomoves() {
    let scene = ProcessScene::start(Arc::new(ProcessEffect::default()), 1).await;

    // A stale session generation: the assignment exists, the frame does not.
    let stale = scene.binding("stale");
    let stale_assignment = scene
        .directory
        .assignment(&process_source("stale"))
        .expect("recorded assignment");
    let _ = stale;
    let regressed = stale_assignment;
    assert_eq!(
        scene
            .service
            .handle(TargetControlRequest::Realize(GuestRealizeRequest::new(
                TargetControlAssignment::new(
                    process_source("stale"),
                    *regressed.uid(),
                    regressed.desired_generation(),
                    9,
                ),
                realization("stale").encode(),
                realization("stale").spec_digest(),
                "d2b/process/Process/stale",
            )))
            .await,
        TargetControlResponse::SessionUnavailable,
        "a frame naming a session generation that is not live performs nothing"
    );

    // A source outside this Guest's authority Zone.
    let foreign = GuestRealizeRequest::new(
        TargetControlAssignment::new(
            ResourceKey::new("other", PROCESS_TYPE, "worker"),
            [7; 16],
            1,
            1,
        ),
        realization("worker").encode(),
        realization("worker").spec_digest(),
        "d2b/process/Process/worker",
    );
    assert_eq!(
        scene
            .service
            .handle(TargetControlRequest::Realize(foreign))
            .await,
        TargetControlResponse::SessionUnavailable,
        "a source from another Zone's authority performs nothing"
    );

    // A spec that does not match the commitment that carries it.
    let substituted = GuestRealizeRequest::new(
        TargetControlAssignment::new(
            process_source("substituted"),
            [7; 16],
            1,
            1,
        ),
        br#"{"providerRef":"Provider/other"}"#.to_vec(),
        realization("substituted").spec_digest(),
        "d2b/process/Process/substituted",
    );
    assert_eq!(
        scene
            .service
            .handle(TargetControlRequest::Realize(substituted))
            .await,
        TargetControlResponse::SessionUnavailable,
        "a substituted spec is refused before the effect sees it"
    );

    assert!(
        scene.effect.applied().await.is_empty(),
        "not one refused frame reached the target-local effect"
    );
    scene.serving.abort();
    let _ = scene.serving.await;
}

/// A lost session takes the target away: nothing more can be realized,
/// observed, adopted, or deleted through it, and the row's desired state and
/// its assignment stay exactly where they were (R21).
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_lost_session_makes_the_process_target_unavailable_without_deleting_anything() {
    let scene = ProcessScene::start(Arc::new(ProcessEffect::default()), 1).await;
    let binding = scene.binding("worker");
    let realization = realization("worker");
    binding
        .realize(
            realization.encode(),
            &realization.spec_digest(),
            "d2b/process/Process/worker",
        )
        .await
        .expect("the realize frame is applied");

    scene.disconnect();

    assert_eq!(
        binding.observe().await.expect("an unavailable target is not an error"),
        TargetObservation::Unavailable,
        "an unreachable target is never read as an absent realization"
    );
    assert!(
        binding
            .realize(
                realization.encode(),
                &realization.spec_digest(),
                "d2b/process/Process/worker",
            )
            .await
            .is_err(),
        "no realization is issued over a session that is gone"
    );
    assert!(
        scene.effect.removed().await.is_empty(),
        "a lost session deletes nothing"
    );
    assert!(
        scene
            .directory
            .assignment(&process_source("worker"))
            .is_some(),
        "the desired row keeps its assignment across the loss"
    );
    scene.serving.abort();
    let _ = scene.serving.await;
}

/// Adoption discovers only an exact live realization: a target whose local
/// effect cannot confirm one answers `missing`, so the owning Host actor
/// realizes it again instead of inheriting a record nothing verified.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn adoption_reports_missing_when_the_process_effect_cannot_confirm_the_realization() {
    for effect in [ProcessEffect::absent(), ProcessEffect::blind()] {
        let scene = ProcessScene::start(Arc::clone(&effect), 1).await;
        let binding = scene.binding("worker");
        let realization = realization("worker");
        binding
            .realize(
                realization.encode(),
                &realization.spec_digest(),
                "d2b/process/Process/worker",
            )
            .await
            .expect("the realize frame is applied");
        scene
            .service
            .handle(TargetControlRequest::Delete {
                assignment: TargetControlAssignment::new(
                    process_source("worker"),
                    [7; 16],
                    1,
                    1,
                ),
            })
            .await;

        let (_rebound, outcome) = binding.adopt().await.expect("adoption runs");
        assert_eq!(
            outcome.adopted(),
            [GuestAdoption::Missing],
            "a realization that cannot be confirmed is re-realized, never inherited"
        );
        scene.serving.abort();
        let _ = scene.serving.await;
    }
}

/// A resource type with no registered target-local effect stays refused with
/// no state: the Guest never records a realization it cannot apply.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unregistered_guest_target_type_stays_refused_with_no_phantom_realization() {
    let scene = ProcessScene::start(Arc::new(ProcessEffect::default()), 1).await;

    // `Endpoint` has no target-local effect code in this scene: only `Process`
    // is registered, and a type the Guest cannot apply is never recorded.
    let response = scene
        .service
        .handle(TargetControlRequest::Realize(GuestRealizeRequest::new(
            TargetControlAssignment::new(source("relay"), [7; 16], 1, 1),
            spec(),
            target_local_spec_digest(&spec()),
            "/run/d2b/relay.sock",
        )))
        .await;
    assert_eq!(
        response,
        TargetControlResponse::SessionUnavailable,
        "a type this Guest has no effect code for is refused"
    );
    assert!(
        scene.runtime.instance(&source("relay")).is_none(),
        "the refusal left no realization behind"
    );
    assert!(scene.effect.applied().await.is_empty());
    scene.serving.abort();
    let _ = scene.serving.await;
}
