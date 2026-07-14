use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use d2b_client::{
    CallOptions, CancellationToken, Client, ClientError, ComponentSession,
    ComponentSessionConnector, ConnectedSession, MetadataInput, RemoteErrorKind, Response,
    RetryClass, RetryPolicy, RouteRecord, RouteTable, ServiceKind, ServiceOwner, SessionAttachment,
    SessionCall, SessionFailure, SessionReply, StreamId, TargetInput, TransportKind,
    TransportSelection, WallClock,
};
use d2b_contracts::{
    v2_component_session::KernelObjectType,
    v2_identity::{ProviderId, RealmId, WorkloadId},
    v2_services::{SERVICE_INVENTORY, common},
};
use protobuf::{EnumOrUnknown, MessageField};
use tokio::sync::Notify;
use ttrpc::r#async::transport::Socket;

const NOW: u64 = 1_800_000_000_000;

#[derive(Debug)]
struct FixedClock;

impl WallClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        NOW
    }
}

#[derive(Default)]
struct FakeSession {
    calls: AtomicUsize,
    cancellations: AtomicUsize,
    failures: Mutex<VecDeque<SessionFailure>>,
    seen: Mutex<Vec<SeenCall>>,
    attachments: Mutex<Vec<SessionAttachment>>,
    response_indexes: Mutex<Vec<u32>>,
    response_override: Mutex<Option<common::ServiceResponse>>,
    stream_id: Mutex<Option<String>>,
    stream_sent: Mutex<Vec<Vec<u8>>>,
    stream_received: Mutex<VecDeque<Vec<u8>>>,
    detached: AtomicUsize,
    closed: AtomicUsize,
    stream_cancelled: AtomicUsize,
    block: AtomicBool,
    blocker: Notify,
}

#[derive(Debug)]
struct SeenCall {
    generation: u64,
    request_id: Vec<u8>,
    idempotency_key: Vec<u8>,
    timeout_nanos: u64,
}

impl FakeSession {
    fn fail_once(&self, failure: SessionFailure) {
        self.failures.lock().unwrap().push_back(failure);
    }

    fn reply(&self) -> SessionReply {
        if let Some(response) = self.response_override.lock().unwrap().clone() {
            return SessionReply {
                response,
                attachments: self.attachments.lock().unwrap().clone(),
            };
        }
        let mut response = common::ServiceResponse::new();
        response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
        response.attachment_indexes = self.response_indexes.lock().unwrap().clone();
        response.stream_id = self.stream_id.lock().unwrap().clone().unwrap_or_default();
        SessionReply {
            response,
            attachments: self.attachments.lock().unwrap().clone(),
        }
    }
}

#[async_trait]
impl ComponentSession for FakeSession {
    fn generation(&self) -> u64 {
        17
    }

    async fn invoke(&self, call: SessionCall) -> Result<SessionReply, SessionFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let metadata = call.request.metadata.as_ref().unwrap();
        self.seen.lock().unwrap().push(SeenCall {
            generation: metadata.session_generation,
            request_id: metadata.request_id.clone(),
            idempotency_key: metadata.idempotency_key.clone(),
            timeout_nanos: call.relative_timeout_nanos,
        });
        if self.block.load(Ordering::Acquire) {
            self.blocker.notified().await;
        }
        if let Some(failure) = self.failures.lock().unwrap().pop_front() {
            return Err(failure);
        }
        Ok(self.reply())
    }

    async fn cancel(&self, generation: u64, request_id: [u8; 16]) -> Result<(), SessionFailure> {
        assert_eq!(generation, 17);
        assert_eq!(request_id, [7; 16]);
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stream_send(&self, _stream: &StreamId, message: &[u8]) -> Result<(), SessionFailure> {
        self.stream_sent.lock().unwrap().push(message.to_vec());
        Ok(())
    }

    async fn stream_receive(&self, _stream: &StreamId) -> Result<Vec<u8>, SessionFailure> {
        Ok(self
            .stream_received
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default())
    }

    async fn stream_detach(&self, _stream: &StreamId) -> Result<(), SessionFailure> {
        self.detached.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stream_close(&self, _stream: &StreamId) -> Result<(), SessionFailure> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stream_cancel(&self, _stream: &StreamId) -> Result<(), SessionFailure> {
        self.stream_cancelled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeConnector {
    session: Arc<FakeSession>,
    attempts: AtomicUsize,
    seen: Mutex<Vec<(TransportKind, ServiceKind)>>,
    fail: AtomicBool,
}

#[derive(Clone)]
struct SharedConnector(Arc<FakeConnector>);

impl FakeConnector {
    fn new(session: Arc<FakeSession>) -> Self {
        Self {
            session,
            attempts: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl ComponentSessionConnector for SharedConnector {
    async fn connect(
        &self,
        target: &d2b_client::ResolvedTarget,
        service: ServiceKind,
    ) -> Result<ConnectedSession, ClientError> {
        self.0.attempts.fetch_add(1, Ordering::SeqCst);
        self.0
            .seen
            .lock()
            .unwrap()
            .push((target.transport(), service));
        if self.0.fail.load(Ordering::Acquire) {
            return Err(ClientError::ConnectFailed);
        }
        let (client, peer) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _peer = peer;
            std::future::pending::<()>().await;
        });
        Ok(ConnectedSession {
            control: self.0.session.clone(),
            ttrpc_socket: Socket::new(client),
        })
    }
}

fn ids() -> (RealmId, WorkloadId, ProviderId) {
    (
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        ProviderId::parse("ccccccccccccccccccca").unwrap(),
    )
}

fn routes() -> RouteTable {
    let (realm, workload, provider) = ids();
    RouteTable::new(vec![
        RouteRecord {
            owner: ServiceOwner::LocalRoot(realm.clone()),
            transport: TransportKind::LocalUnix,
        },
        RouteRecord {
            owner: ServiceOwner::Realm(realm.clone()),
            transport: TransportKind::Provider,
        },
        RouteRecord {
            owner: ServiceOwner::Workload {
                realm: realm.clone(),
                workload,
            },
            transport: TransportKind::NativeVsock,
        },
        RouteRecord {
            owner: ServiceOwner::Provider { realm, provider },
            transport: TransportKind::InheritedSocket,
        },
    ])
}

fn options(mutating: bool) -> CallOptions {
    let metadata = MetadataInput::new([7; 16], NOW, NOW + 30_000)
        .unwrap()
        .with_correlation("correlation-1")
        .unwrap()
        .with_trace([8; 16])
        .unwrap();
    let metadata = if mutating {
        metadata.with_idempotency(vec![9; 32]).unwrap()
    } else {
        metadata
    };
    CallOptions {
        metadata,
        retry: RetryPolicy::new(3).unwrap(),
    }
}

async fn daemon_client(
    session: Arc<FakeSession>,
) -> (Arc<FakeConnector>, d2b_client::ConnectedClient) {
    let (realm, _, _) = ids();
    let connector = Arc::new(FakeConnector::new(session));
    let client = Client::with_clock(routes(), SharedConnector(connector.clone()), FixedClock);
    let connected = client
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap();
    (connector, connected)
}

#[tokio::test]
async fn typed_routes_select_exact_transport_without_fallback() {
    let (realm, workload, provider) = ids();
    let session = Arc::new(FakeSession::default());
    let connector = Arc::new(FakeConnector::new(session));
    let client = Client::with_clock(routes(), SharedConnector(connector.clone()), FixedClock);
    let cases = [
        (
            TargetInput::LocalRoot(realm.clone()),
            TransportKind::LocalUnix,
        ),
        (TargetInput::Realm(realm.clone()), TransportKind::Provider),
        (
            TargetInput::Workload {
                realm: realm.clone(),
                workload,
            },
            TransportKind::NativeVsock,
        ),
        (
            TargetInput::Provider { realm, provider },
            TransportKind::InheritedSocket,
        ),
    ];
    for (target, transport) in cases {
        client
            .connect(
                target,
                ServiceKind::Daemon,
                TransportSelection::exact(transport),
            )
            .await
            .unwrap();
    }
    assert_eq!(connector.attempts.load(Ordering::SeqCst), 4);

    let (realm, _, _) = ids();
    let error = client
        .connect(
            TargetInput::LocalRoot(realm.clone()),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::Provider),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::TransportPolicyMismatch);
    assert_eq!(connector.attempts.load(Ordering::SeqCst), 4);

    connector.fail.store(true, Ordering::Release);
    let error = client
        .connect(
            TargetInput::LocalRoot(realm),
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::ConnectFailed);
    assert_eq!(connector.attempts.load(Ordering::SeqCst), 5);

    let error = client
        .connect(
            TargetInput::Service {
                owner: ServiceOwner::LocalRoot(ids().0),
                service: ServiceKind::Realm,
            },
            ServiceKind::Daemon,
            TransportSelection::exact(TransportKind::LocalUnix),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::InvalidService);
    assert_eq!(connector.attempts.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn constructs_every_generated_inventory_client() {
    assert_eq!(ServiceKind::ALL.len(), SERVICE_INVENTORY.len());
    let (realm, _, _) = ids();
    let session = Arc::new(FakeSession::default());
    let connector = Arc::new(FakeConnector::new(session));
    let client = Client::with_clock(routes(), SharedConnector(connector), FixedClock);
    for service in ServiceKind::ALL {
        let connected = client
            .connect(
                TargetInput::Service {
                    owner: ServiceOwner::LocalRoot(realm.clone()),
                    service: *service,
                },
                *service,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .await
            .unwrap();
        assert_eq!(connected.service().generated().kind(), *service);
        assert_eq!(
            connected.service().kind().spec().service,
            SERVICE_INVENTORY[*service as usize].service
        );
    }
}

#[tokio::test]
async fn metadata_uses_trusted_generation_and_per_hop_deadline() {
    let session = Arc::new(FakeSession::default());
    let (_, client) = daemon_client(session.clone()).await;
    let method = client.service().method(3).unwrap();
    client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let seen = session.seen.lock().unwrap();
    assert_eq!(seen[0].generation, 17);
    assert_eq!(seen[0].request_id, vec![7; 16]);
    assert!(seen[0].idempotency_key.is_empty());
    assert!(seen[0].timeout_nanos > 0);
    assert!(seen[0].timeout_nanos <= 30_000_000_000);
}

#[tokio::test]
async fn retries_only_with_stable_mutation_idempotency() {
    let session = Arc::new(FakeSession::default());
    session.fail_once(SessionFailure::Retryable);
    let (_, client) = daemon_client(session.clone()).await;
    let method = client.service().method(4).unwrap();
    client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(session.calls.load(Ordering::SeqCst), 2);
    {
        let seen = session.seen.lock().unwrap();
        assert_eq!(seen[0].request_id, seen[1].request_id);
        assert_eq!(seen[0].idempotency_key, seen[1].idempotency_key);
    }

    let missing = CallOptions {
        metadata: MetadataInput::new([7; 16], NOW, NOW + 30_000).unwrap(),
        retry: RetryPolicy::new(3).unwrap(),
    };
    let error = client
        .invoke(
            method,
            common::ServiceRequest::new(),
            missing,
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::IdempotencyRequired);

    session.fail_once(SessionFailure::Ambiguous);
    let error = client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::AmbiguousMutation);

    session.fail_once(SessionFailure::Retryable);
    session.fail_once(SessionFailure::Retryable);
    session.fail_once(SessionFailure::Retryable);
    let error = client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(true),
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::RetryLimitExceeded);
}

#[tokio::test]
async fn cancellation_signals_original_request() {
    let session = Arc::new(FakeSession::default());
    session.block.store(true, Ordering::Release);
    let (_, client) = daemon_client(session.clone()).await;
    let method = client.service().method(3).unwrap();
    let cancellation = CancellationToken::default();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        trigger.cancel();
    });
    let error = client
        .invoke(
            method,
            common::ServiceRequest::new(),
            options(false),
            &cancellation,
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::Cancelled);
    assert_eq!(session.cancellations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn attachment_indexes_must_match_session_order() {
    let session = Arc::new(FakeSession::default());
    *session.response_indexes.lock().unwrap() = vec![0];
    *session.attachments.lock().unwrap() = vec![SessionAttachment::new(1, KernelObjectType::Memfd)];
    let (_, client) = daemon_client(session).await;
    let error = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::AttachmentMismatch);
}

#[tokio::test]
async fn validates_closed_remote_errors_and_outcome_consistency() {
    let session = Arc::new(FakeSession::default());
    let mut response = common::ServiceResponse::new();
    response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_DENIED);
    let mut error = common::ErrorEnvelope::new();
    error.kind = EnumOrUnknown::new(common::ErrorKind::ERROR_KIND_UNAUTHORIZED);
    error.retry = EnumOrUnknown::new(common::RetryClass::RETRY_CLASS_NEVER);
    response.error = MessageField::some(error);
    *session.response_override.lock().unwrap() = Some(response);
    let (_, client) = daemon_client(session.clone()).await;
    let result = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await;
    assert_eq!(
        result.unwrap_err(),
        ClientError::Remote {
            kind: RemoteErrorKind::Forbidden,
            retry: RetryClass::Never,
        }
    );

    let mut inconsistent = session.response_override.lock().unwrap().clone().unwrap();
    inconsistent.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
    *session.response_override.lock().unwrap() = Some(inconsistent);
    let result = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await;
    assert_eq!(result.unwrap_err(), ClientError::ContractViolation);
}

#[tokio::test]
async fn named_streams_are_bounded_and_have_explicit_terminal_actions() {
    let session = Arc::new(FakeSession::default());
    *session.stream_id.lock().unwrap() = Some("stream-1".to_owned());
    session
        .stream_received
        .lock()
        .unwrap()
        .push_back(b"reply".to_vec());
    let (_, client) = daemon_client(session.clone()).await;
    let response = client
        .invoke(
            client.service().method(3).unwrap(),
            common::ServiceRequest::new(),
            options(false),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
    let stream = client.named_stream(&response).unwrap();
    stream.send(b"request").await.unwrap();
    assert_eq!(stream.receive().await.unwrap(), b"reply");
    stream.detach().await.unwrap();
    assert!(stream.is_terminal());
    assert_eq!(
        stream.send(b"late").await.unwrap_err(),
        ClientError::StreamDetached
    );

    let close = client
        .named_stream(&Response {
            message: response.message.clone(),
            attachments: vec![],
        })
        .unwrap();
    close.close().await.unwrap();
    let cancel = client
        .named_stream(&Response {
            message: response.message.clone(),
            attachments: vec![],
        })
        .unwrap();
    cancel.cancel().await.unwrap();
    assert_eq!(session.detached.load(Ordering::SeqCst), 1);
    assert_eq!(session.closed.load(Ordering::SeqCst), 1);
    assert_eq!(session.stream_cancelled.load(Ordering::SeqCst), 1);

    let oversized = vec![0; 256 * 1024 + 1];
    assert_eq!(
        stream.send(&oversized).await.unwrap_err(),
        ClientError::StreamLimitExceeded
    );

    let streams: Vec<_> = (0..128)
        .map(|_| {
            client
                .named_stream(&Response {
                    message: response.message.clone(),
                    attachments: vec![],
                })
                .unwrap()
        })
        .collect();
    assert_eq!(
        client
            .named_stream(&Response {
                message: response.message,
                attachments: vec![],
            })
            .unwrap_err(),
        ClientError::StreamLimitExceeded
    );
    drop(streams);
}

#[test]
fn debug_and_errors_are_redacted() {
    let metadata = MetadataInput::new([7; 16], NOW, NOW + 30_000)
        .unwrap()
        .with_correlation("sensitive-correlation")
        .unwrap()
        .with_idempotency(b"sensitive-idempotency".to_vec())
        .unwrap();
    let metadata_debug = format!("{metadata:?}");
    assert!(!metadata_debug.contains("sensitive"));

    let stream = StreamId::new("sensitive-stream").unwrap();
    assert!(!format!("{stream:?}").contains("sensitive"));

    let remote = ClientError::Remote {
        kind: RemoteErrorKind::Internal,
        retry: RetryClass::Never,
    };
    assert_eq!(remote.to_string(), "client-remote-error");
}
