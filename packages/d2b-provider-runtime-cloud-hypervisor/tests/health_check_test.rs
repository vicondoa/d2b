use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceGeneration, ResourcePhase, ResourceRef,
    ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision, identity::ReconnectGeneration,
};
use d2b_provider_runtime_cloud_hypervisor::{
    GuestControlEndpoint, GuestGenerationSet, GuestLocalError, GuestLocalResourceStatus,
    GuestLocalSeedBatch, GuestLocalSeedResult, GuestLocalSession, GuestLocalSessionBinding,
    GuestLocalSessionExpectation, GuestLocalStatus, GuestLocalWatch, GuestSessionHealth,
    GuestStatusObservation, GuestStatusPhase,
};
use d2b_session::TransportDescriptor;

const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const ENDPOINT_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const SEED_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const DESCRIPTOR_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SCHEMA_DIGEST: &str =
    "sha256:2222222222222222222222222222222222222222222222222222222222222222";
const BOOT_DIGEST: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

fn guest_ref() -> ResourceRef {
    ResourceRef::parse("Guest/gateway").unwrap()
}

fn endpoint_ref() -> ResourceRef {
    ResourceRef::parse("Endpoint/gateway-guest-control").unwrap()
}

fn endpoint() -> GuestControlEndpoint {
    GuestControlEndpoint::new(
        endpoint_ref(),
        guest_ref(),
        ZoneId::parse("work").unwrap(),
        ResourceUid::parse(ENDPOINT_UID).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        SchemaFingerprint::parse(SCHEMA_DIGEST).unwrap(),
        true,
    )
    .unwrap()
}

fn expectation() -> GuestLocalSessionExpectation {
    GuestLocalSessionExpectation::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        ZoneId::parse("work").unwrap(),
        endpoint_ref(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        SchemaFingerprint::parse(SCHEMA_DIGEST).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ControllerGeneration::new(1).unwrap(),
        ReconnectGeneration::new(1).unwrap(),
        BOOT_DIGEST,
        GuestGenerationSet::all(1),
    )
    .unwrap()
}

fn seed_batch_with(operation_id: &str, name: &str) -> GuestLocalSeedBatch {
    let raw = format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"createdAt":"2026-08-29T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"controller","name":"{name}","ownerRef":"Guest/gateway","revision":1,"updatedAt":"2026-08-29T00:00:00.000Z","zone":"work"}},"spec":{{"executionRef":"Guest/gateway"}},"status":{{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{{}},"startedAt":null,"update":{{"dependencies":{{"count":0,"refs":[]}},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{{"count":0,"refs":[]}},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}},"type":"Process"}}"#
    );
    let payload = CanonicalJsonValue::parse(raw.as_bytes())
        .unwrap()
        .to_canonical_bytes();
    GuestLocalSeedBatch::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        operation_id,
        vec![
            d2b_provider_runtime_cloud_hypervisor::GuestLocalSeedResource::new(
                ResourceRef::parse(&format!("Process/{name}")).unwrap(),
                guest_ref(),
                payload,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn seed_batch() -> GuestLocalSeedBatch {
    seed_batch_with("seed-1", "gateway-agent")
}

fn binding(session_generation: u64) -> GuestLocalSessionBinding {
    GuestLocalSessionBinding::new(
        guest_ref(),
        ResourceUid::parse(GUEST_UID).unwrap(),
        ZoneId::parse("work").unwrap(),
        endpoint_ref(),
        ResourceUid::parse(ENDPOINT_UID).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
        SchemaFingerprint::parse(SCHEMA_DIGEST).unwrap(),
        ResourceGeneration::new(1).unwrap(),
        ControllerGeneration::new(1).unwrap(),
        ReconnectGeneration::new(session_generation).unwrap(),
        ReconnectGeneration::new(session_generation).unwrap(),
        BOOT_DIGEST,
    )
    .unwrap()
}

#[derive(Clone)]
struct FakeSession {
    binding: GuestLocalSessionBinding,
    commits: Arc<AtomicUsize>,
    watches: Arc<AtomicUsize>,
    ready: Arc<Mutex<bool>>,
    live: Arc<Mutex<bool>>,
    denied: bool,
}

#[async_trait]
impl GuestLocalSession for FakeSession {
    fn binding(&self) -> &GuestLocalSessionBinding {
        &self.binding
    }

    fn transport_descriptor(&self) -> TransportDescriptor {
        d2b_session_unix::guest_control_transport_descriptor()
    }

    fn is_live(&self) -> bool {
        *self.live.lock().unwrap()
    }

    async fn commit_seed_batch(
        &self,
        batch: &GuestLocalSeedBatch,
    ) -> Result<GuestLocalSeedResult, GuestLocalError> {
        if self.denied {
            return Err(GuestLocalError::AuthorizationDenied);
        }
        self.commits.fetch_add(1, Ordering::SeqCst);
        let resource_ref = batch.resources()[0].resource_ref().clone();
        Ok(GuestLocalSeedResult::new(
            batch.operation_id().to_owned(),
            batch.guest_uid().clone(),
            batch.descriptor_digest().clone(),
            ZoneRevision::new(2),
            ResourceGeneration::new(1).unwrap(),
            vec![
                GuestLocalResourceStatus::new(
                    resource_ref,
                    ResourceUid::parse(SEED_UID).unwrap(),
                    ResourceUid::parse(GUEST_UID).unwrap(),
                    ResourceGeneration::new(1).unwrap(),
                    ZoneRevision::new(2),
                    if *self.ready.lock().unwrap() {
                        ResourcePhase::Ready
                    } else {
                        ResourcePhase::Pending
                    },
                    true,
                )
                .unwrap(),
            ],
        )
        .unwrap()
        .with_ready(*self.ready.lock().unwrap()))
    }

    async fn resume_seed_watch(
        &self,
        after_revision: ZoneRevision,
        resources: &[ResourceRef],
    ) -> Result<GuestLocalWatch, GuestLocalError> {
        self.watches.fetch_add(1, Ordering::SeqCst);
        let phase = if *self.ready.lock().unwrap() {
            ResourcePhase::Ready
        } else {
            ResourcePhase::Pending
        };
        let statuses = resources
            .iter()
            .map(|resource_ref| {
                GuestLocalResourceStatus::new(
                    resource_ref.clone(),
                    ResourceUid::parse(SEED_UID).unwrap(),
                    ResourceUid::parse(GUEST_UID).unwrap(),
                    ResourceGeneration::new(1).unwrap(),
                    ZoneRevision::new(after_revision.get() + 1),
                    phase,
                    true,
                )
                .unwrap()
            })
            .collect();
        Ok(GuestLocalWatch::new(
            after_revision,
            ZoneRevision::new(after_revision.get() + 1),
            statuses,
        )
        .unwrap())
    }
}

#[derive(Clone)]
struct FakeResolver {
    endpoint: GuestControlEndpoint,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl d2b_provider_runtime_cloud_hypervisor::GuestControlEndpointResolver for FakeResolver {
    async fn resolve_guest_control_endpoint(
        &self,
        _endpoint_ref: &ResourceRef,
    ) -> Result<GuestControlEndpoint, GuestLocalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.endpoint.clone())
    }
}

#[derive(Clone)]
struct FakeConnector {
    session: FakeSession,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl d2b_provider_runtime_cloud_hypervisor::GuestControlSessionConnector for FakeConnector {
    type Session = FakeSession;

    async fn connect_guest_control(
        &self,
        _endpoint: &GuestControlEndpoint,
        _minimum_generation: ReconnectGeneration,
    ) -> Result<Self::Session, GuestLocalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.session.clone())
    }
}

#[derive(Clone)]
struct SequenceConnector {
    sessions: Arc<Mutex<VecDeque<FakeSession>>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl d2b_provider_runtime_cloud_hypervisor::GuestControlSessionConnector for SequenceConnector {
    type Session = FakeSession;

    async fn connect_guest_control(
        &self,
        _endpoint: &GuestControlEndpoint,
        _minimum_generation: ReconnectGeneration,
    ) -> Result<Self::Session, GuestLocalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.sessions
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(GuestLocalError::SessionAuthentication)
    }
}

#[derive(Clone)]
struct LossSession {
    binding: GuestLocalSessionBinding,
}

#[async_trait]
impl GuestLocalSession for LossSession {
    fn binding(&self) -> &GuestLocalSessionBinding {
        &self.binding
    }

    fn transport_descriptor(&self) -> TransportDescriptor {
        d2b_session_unix::guest_control_transport_descriptor()
    }

    fn is_live(&self) -> bool {
        true
    }

    async fn commit_seed_batch(
        &self,
        _batch: &GuestLocalSeedBatch,
    ) -> Result<GuestLocalSeedResult, GuestLocalError> {
        Err(GuestLocalError::SessionLost)
    }

    async fn resume_seed_watch(
        &self,
        _after_revision: ZoneRevision,
        _resources: &[ResourceRef],
    ) -> Result<GuestLocalWatch, GuestLocalError> {
        Err(GuestLocalError::SessionLost)
    }
}

#[derive(Clone)]
struct LossConnector;

#[async_trait]
impl d2b_provider_runtime_cloud_hypervisor::GuestControlSessionConnector for LossConnector {
    type Session = LossSession;

    async fn connect_guest_control(
        &self,
        _endpoint: &GuestControlEndpoint,
        _minimum_generation: ReconnectGeneration,
    ) -> Result<Self::Session, GuestLocalError> {
        Ok(LossSession {
            binding: binding(1),
        })
    }
}

fn host_ready() -> GuestStatusObservation {
    GuestStatusObservation::ready(1)
}

#[tokio::test]
async fn valid_endpoint_session_and_seed_reach_ready() {
    let commits = Arc::new(AtomicUsize::new(0));
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::clone(&commits),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );

    let status = controller
        .reconcile(&host_ready(), seed_batch())
        .await
        .unwrap();
    assert_eq!(status.phase(), GuestStatusPhase::Ready);
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    let evidence = controller.session_evidence().expect("session evidence");
    assert_eq!(evidence.health(), GuestSessionHealth::Ready);
    assert_eq!(
        evidence.guest_uid().map(ResourceUid::as_str),
        Some(GUEST_UID)
    );
    assert_eq!(
        evidence.descriptor_digest().map(SchemaFingerprint::as_str),
        Some(DESCRIPTOR_DIGEST)
    );
    assert_eq!(evidence.seed_generation(), Some(1));
}

#[tokio::test]
async fn host_pending_gates_endpoint_resolution_and_seed() {
    let resolver_calls = Arc::new(AtomicUsize::new(0));
    let connector_calls = Arc::new(AtomicUsize::new(0));
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::clone(&resolver_calls),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::new(AtomicUsize::new(0)),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::clone(&connector_calls),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    let mut host = host_ready();
    host.process_ready = false;
    let status = controller.reconcile(&host, seed_batch()).await.unwrap();
    assert_eq!(status.phase(), GuestStatusPhase::Pending);
    assert_eq!(resolver_calls.load(Ordering::SeqCst), 0);
    assert_eq!(connector_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pending_target_seed_keeps_parent_pending_until_child_ready() {
    let ready = Arc::new(Mutex::new(false));
    let commits = Arc::new(AtomicUsize::new(0));
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::clone(&commits),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::clone(&ready),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch())
            .await
            .unwrap()
            .phase(),
        GuestStatusPhase::Pending
    );
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        controller.status(&host_ready()).phase(),
        GuestStatusPhase::Pending
    );
    *ready.lock().unwrap() = true;
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch())
            .await
            .unwrap()
            .phase(),
        GuestStatusPhase::Ready
    );
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(controller.last_revision(), ZoneRevision::new(3));
}

#[tokio::test]
async fn session_loss_projects_degraded_until_a_new_generation_recovers() {
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let first_live = Arc::new(Mutex::new(true));
    let commits = Arc::new(AtomicUsize::new(0));
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::clone(&commits),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::clone(&first_live),
            denied: false,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch())
            .await
            .unwrap()
            .phase(),
        GuestStatusPhase::Ready
    );
    *first_live.lock().unwrap() = false;
    controller.mark_session_lost();
    assert_eq!(
        controller.status(&host_ready()).phase(),
        GuestStatusPhase::Degraded
    );
    let evidence = controller.session_evidence().expect("degraded evidence");
    assert_eq!(evidence.health(), GuestSessionHealth::Degraded);
    assert_eq!(evidence.session_generation(), Some(1));
}

#[tokio::test]
async fn mismatched_session_identity_is_rejected_before_seed() {
    let commits = Arc::new(AtomicUsize::new(0));
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: GuestLocalSessionBinding::new(
                ResourceRef::parse("Guest/other").unwrap(),
                ResourceUid::parse(GUEST_UID).unwrap(),
                ZoneId::parse("work").unwrap(),
                endpoint_ref(),
                ResourceUid::parse(ENDPOINT_UID).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                SchemaFingerprint::parse(DESCRIPTOR_DIGEST).unwrap(),
                SchemaFingerprint::parse(SCHEMA_DIGEST).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ControllerGeneration::new(1).unwrap(),
                ReconnectGeneration::new(1).unwrap(),
                ReconnectGeneration::new(1).unwrap(),
                BOOT_DIGEST,
            )
            .unwrap(),
            commits: Arc::clone(&commits),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    assert_eq!(
        controller.reconcile(&host_ready(), seed_batch()).await,
        Err(GuestLocalError::SessionBindingMismatch)
    );
    assert_eq!(commits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn endpoint_uid_and_generation_mismatch_is_rejected_before_connect() {
    let bad_endpoint = GuestControlEndpoint::new(
        endpoint_ref(),
        guest_ref(),
        ZoneId::parse("work").unwrap(),
        ResourceUid::parse(ENDPOINT_UID).unwrap(),
        ResourceGeneration::new(2).unwrap(),
        ResourceGeneration::new(3).unwrap(),
        ResourceGeneration::new(2).unwrap(),
        SchemaFingerprint::parse(SCHEMA_DIGEST).unwrap(),
        true,
    )
    .unwrap();
    let connector_calls = Arc::new(AtomicUsize::new(0));
    let resolver = FakeResolver {
        endpoint: bad_endpoint,
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::new(AtomicUsize::new(0)),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::clone(&connector_calls),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    assert_eq!(
        controller.reconcile(&host_ready(), seed_batch()).await,
        Err(GuestLocalError::EndpointMismatch)
    );
    assert_eq!(connector_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn reconnect_resumes_from_revision_without_duplicate_seed() {
    let commits = Arc::new(AtomicUsize::new(0));
    let watches = Arc::new(AtomicUsize::new(0));
    let first_live = Arc::new(Mutex::new(true));
    let second_live = Arc::new(Mutex::new(true));
    let connector = SequenceConnector {
        sessions: Arc::new(Mutex::new(VecDeque::from([
            FakeSession {
                binding: binding(1),
                commits: Arc::clone(&commits),
                watches: Arc::clone(&watches),
                ready: Arc::new(Mutex::new(true)),
                live: Arc::clone(&first_live),
                denied: false,
            },
            FakeSession {
                binding: binding(2),
                commits: Arc::clone(&commits),
                watches: Arc::clone(&watches),
                ready: Arc::new(Mutex::new(true)),
                live: Arc::clone(&second_live),
                denied: false,
            },
        ]))),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    controller
        .reconcile(&host_ready(), seed_batch())
        .await
        .unwrap();
    *first_live.lock().unwrap() = false;
    controller.mark_session_lost();
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch())
            .await
            .unwrap()
            .phase(),
        GuestStatusPhase::Ready
    );
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(watches.load(Ordering::SeqCst), 1);
    assert_eq!(
        controller
            .session_evidence()
            .expect("reconnected evidence")
            .session_generation(),
        Some(2)
    );
}

#[tokio::test]
async fn operation_id_reuse_with_changed_seed_is_rejected() {
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::new(AtomicUsize::new(0)),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: false,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    controller
        .reconcile(&host_ready(), seed_batch())
        .await
        .unwrap();
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch_with("seed-1", "gateway-other"))
            .await,
        Err(GuestLocalError::OperationReused)
    );
}

#[tokio::test]
async fn authorization_denial_is_observed_before_any_ready_status() {
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let connector = FakeConnector {
        session: FakeSession {
            binding: binding(1),
            commits: Arc::new(AtomicUsize::new(0)),
            watches: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(Mutex::new(true)),
            live: Arc::new(Mutex::new(true)),
            denied: true,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        connector,
    );
    assert_eq!(
        controller.reconcile(&host_ready(), seed_batch()).await,
        Err(GuestLocalError::AuthorizationDenied)
    );
}

#[tokio::test]
async fn session_loss_before_ready_remains_pending() {
    let resolver = FakeResolver {
        endpoint: endpoint(),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut controller = d2b_provider_runtime_cloud_hypervisor::GuestLocalController::new(
        expectation(),
        resolver,
        LossConnector,
    );
    assert_eq!(
        controller
            .reconcile(&host_ready(), seed_batch())
            .await
            .unwrap()
            .phase(),
        GuestStatusPhase::Pending
    );
}

#[test]
fn health_projection_and_debug_output_are_redacted() {
    let status = GuestLocalStatus::new(GuestStatusPhase::Ready, true, true, true);
    let rendered = format!("{status:?}");
    assert!(!rendered.contains("secret-path"));
    assert!(!rendered.contains(GUEST_UID));
    assert!(!rendered.contains("/nix/store"));
}

#[test]
fn session_admission_names_only_commit_batch_for_guest_seeding() {
    let service =
        d2b_contracts_resource::v3::identity::ServiceName::parse("d2b.resource.v3").unwrap();
    let zone = ZoneId::parse("work").unwrap();
    let commit = d2b_session::SessionAuthorizationRequest::new(
        d2b_session::SessionVerb::Invoke,
        service.clone(),
        "ResourceService/CommitBatch",
        zone.clone(),
        None,
    )
    .unwrap();
    assert!(commit.is_guest_resource_commit_batch());
    let get = d2b_session::SessionAuthorizationRequest::new(
        d2b_session::SessionVerb::Invoke,
        service,
        "ResourceService/Get",
        zone,
        Some(guest_ref()),
    )
    .unwrap();
    assert!(!get.is_guest_resource_commit_batch());
}
