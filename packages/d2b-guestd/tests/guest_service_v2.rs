use std::{
    collections::HashMap,
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        BootstrapPskBinding, GuestBootstrapCredentialV1, GuestBootstrapPsk, GuestIdentityBindingV1,
        GuestSessionCredentialV1, LimitProfile, OperationId,
    },
    v2_services::{
        StrictWireMessage,
        common::{self, Outcome},
        guest,
        guest_ttrpc::GuestService,
        parse_server_stream_name, terminal,
    },
};
use d2b_guestd::{
    guest_service::{
        GuestAuthorizationGate, GuestAuthorizationPhase, GuestOperationHandler, GuestServiceV2,
        GuestStream, GuestStreamBinding, GuestStreamInput, GuestStreamMethod, GuestWallClock,
    },
    service_v2::{
        BootstrapPeerEvidence, FramedGuestTransport, GuestSessionAuthority, GuestSessionError,
        GuestSessionMaterial, GuestStaticIdentity, SealedIdentityStore, guest_bootstrap_policy,
        guest_enrolled_policy,
    },
};
use d2b_session::{
    BootstrapAdmission, BootstrapPsk, ComponentSessionDriver, HandshakeCredentials, Secret32,
    SessionEngine, StreamEvent, StreamId,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

const GENERATION: u64 = 7;
const NOW_MS: u64 = 10_000;
const CHANNEL_BINDING: [u8; 32] = [0x44; 32];
const PARENT_PRIVATE: [u8; 32] = [0x31; 32];
const GUEST_PRIVATE: [u8; 32] = [0x42; 32];
const PSK: [u8; 32] = [0x55; 32];
const REPLAY_NONCE: [u8; 32] = [0x66; 32];
const OPERATION_ID: [u8; 16] = [0x77; 16];
const WORKLOAD_ID: &str = "bbbbbbbbbbbbbbbbbbba";

struct FixedClock;

impl GuestWallClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        NOW_MS
    }
}

struct BlockingAuthorizationGate {
    phase: GuestAuthorizationPhase,
    reached: Notify,
    release: Notify,
}

#[async_trait]
impl GuestAuthorizationGate for BlockingAuthorizationGate {
    async fn before_point_of_no_return(&self, phase: GuestAuthorizationPhase) {
        assert_eq!(phase, self.phase);
        self.reached.notify_one();
        self.release.notified().await;
    }
}

fn material(parent_public: [u8; 32], bootstrap: bool) -> GuestSessionMaterial {
    let guest_public = GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
        .unwrap()
        .public_key()
        .unwrap();
    let guest_identity_digest: [u8; 32] = Sha256::digest(guest_public).into();
    let bootstrap = bootstrap.then(|| {
        GuestBootstrapCredentialV1::new(
            BootstrapPskBinding {
                operation_id: OperationId::new(OPERATION_ID.to_vec()).unwrap(),
                replay_nonce: REPLAY_NONCE,
                expires_at_unix_ms: NOW_MS + 1_000,
            },
            NOW_MS - 1,
            GuestBootstrapPsk::generate_with(|bytes| {
                bytes.copy_from_slice(&PSK);
                Ok(())
            })
            .unwrap(),
        )
        .unwrap()
    });
    let identity_binding = if bootstrap.is_some() {
        GuestIdentityBindingV1::UnboundBootstrap
    } else {
        GuestIdentityBindingV1::Enrolled {
            guest_identity_digest,
            guest_static_public_key: guest_public,
        }
    };
    let credential = GuestSessionCredentialV1::new(
        GENERATION,
        parent_public,
        CHANNEL_BINDING,
        identity_binding,
        bootstrap,
    )
    .unwrap()
    .encode()
    .unwrap();
    GuestSessionMaterial::from_credential_bytes(&credential).expect("valid session material")
}

fn parent_identity() -> GuestStaticIdentity {
    GuestStaticIdentity::from_private_key(PARENT_PRIVATE).expect("parent identity")
}

fn context() -> ttrpc::r#async::TtrpcContext {
    ttrpc::r#async::TtrpcContext {
        mh: ttrpc::proto::MessageHeader::new_request(1, 0),
        metadata: HashMap::new(),
        timeout_nano: 0,
    }
}

fn metadata(generation: u64) -> common::RequestMetadata {
    common::RequestMetadata {
        request_id: vec![0x11; 16],
        idempotency_key: vec![0x22; 16],
        issued_at_unix_ms: NOW_MS,
        expires_at_unix_ms: NOW_MS + 1_000,
        session_generation: generation,
        ..Default::default()
    }
}

fn scope(workload: &str) -> common::IdentityScope {
    common::IdentityScope {
        realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
        workload_id: workload.to_owned(),
        ..Default::default()
    }
}

fn operation_context(generation: u64, workload: &str) -> guest::GuestOperationContext {
    guest::GuestOperationContext {
        metadata: MessageField::some(metadata(generation)),
        scope: MessageField::some(scope(workload)),
        operation_id: "operation-1".to_owned(),
        request_digest: vec![0x33; 32],
        ..Default::default()
    }
}

fn exec_request(generation: u64, workload: &str) -> guest::GuestExecRequest {
    let mut exec_metadata = metadata(generation);
    exec_metadata.request_id = vec![0x12; 16];
    exec_metadata.idempotency_key = vec![0x23; 16];
    guest::GuestExecRequest {
        terminal: MessageField::some(terminal::TerminalOpenRequest {
            metadata: MessageField::some(exec_metadata),
            scope: MessageField::some(scope(workload)),
            resource_id: "guest-exec".to_owned(),
            operation_id: "operation-1".to_owned(),
            request_digest: vec![0x33; 32],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn service(
    handler: Arc<dyn GuestOperationHandler>,
    session: d2b_guestd::service_v2::EstablishedGuestSession,
) -> GuestServiceV2 {
    GuestServiceV2::with_handler_and_clock(handler, session, Arc::new(FixedClock))
}

fn bootstrap_evidence() -> BootstrapPeerEvidence {
    BootstrapPeerEvidence::new(OPERATION_ID.to_vec(), REPLAY_NONCE).expect("evidence")
}

struct BootstrapStore {
    sealed: AtomicBool,
}

impl SealedIdentityStore for BootstrapStore {
    fn load(&self) -> Result<Option<GuestStaticIdentity>, GuestSessionError> {
        Ok(None)
    }

    fn seal(&self, _: &GuestStaticIdentity) -> Result<(), GuestSessionError> {
        if self.sealed.swap(true, Ordering::AcqRel) {
            return Err(GuestSessionError::IdentityUnsafe);
        }
        Ok(())
    }

    fn generate(&self) -> Result<GuestStaticIdentity, GuestSessionError> {
        GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
    }
}

struct EnrolledStore;

impl SealedIdentityStore for EnrolledStore {
    fn load(&self) -> Result<Option<GuestStaticIdentity>, GuestSessionError> {
        GuestStaticIdentity::from_private_key(GUEST_PRIVATE).map(Some)
    }

    fn seal(&self, _: &GuestStaticIdentity) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::IdentityUnsafe)
    }
}

struct TestOperations {
    ready: bool,
}

#[async_trait]
impl GuestOperationHandler for TestOperations {
    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        vec![
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
            guest::GuestCapability::GUEST_CAPABILITY_SHUTDOWN,
        ]
    }

    fn scope_authorized(&self, scope: &common::IdentityScope) -> bool {
        scope.workload_id == WORKLOAD_ID
    }

    fn stream_ready(&self, _: GuestStreamMethod) -> bool {
        self.ready
    }

    async fn serve_exec(
        &self,
        _: guest::GuestExecRequest,
        _: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        mut stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        if !self.ready {
            return Err(GuestSessionError::Service);
        }
        tokio::spawn(async move {
            let GuestStreamInput::Message(bytes) = stream.receive().await else {
                return;
            };
            if stream.consume(bytes.len()).await.is_err()
                || terminal::TerminalStreamFrame::parse_from_bytes(&bytes).is_err()
            {
                let _ = stream.reset().await;
                return;
            }
            for (sequence, frame) in [
                terminal::terminal_stream_frame::Frame::Started(terminal::TerminalStarted {
                    kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_EXEC),
                    ..Default::default()
                }),
                terminal::terminal_stream_frame::Frame::Outcome(terminal::TerminalOutcome {
                    outcome: Some(terminal::terminal_outcome::Outcome::Exited(
                        terminal::TerminalExited {
                            exit_code: 0,
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
            ]
            .into_iter()
            .enumerate()
            {
                let message = terminal::TerminalStreamFrame {
                    session_generation: binding.generation,
                    request_id: binding.request_id.clone(),
                    sequence: sequence as u64,
                    operation_id: binding.operation_id.clone(),
                    resource_handle: binding.resource_handle.clone(),
                    frame: Some(frame),
                    ..Default::default()
                };
                if stream.send(&message).await.is_err() {
                    return;
                }
            }
            let _ = stream.close().await;
        });
        Ok(())
    }
}

struct BlockingOperations {
    started: Arc<Notify>,
}

#[async_trait]
impl GuestOperationHandler for BlockingOperations {
    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        vec![guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED]
    }

    fn scope_authorized(&self, scope: &common::IdentityScope) -> bool {
        scope.workload_id == WORKLOAD_ID
    }

    fn stream_ready(&self, _: GuestStreamMethod) -> bool {
        true
    }

    async fn inspect_exec(
        &self,
        _: guest::GuestInspectExecRequest,
        _: GuestStreamBinding,
    ) -> Result<guest::GuestInspectExecResponse, GuestSessionError> {
        self.started.notify_one();
        pending().await
    }
}

struct ResumeOperations;

#[async_trait]
impl GuestOperationHandler for ResumeOperations {
    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        vec![
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
            guest::GuestCapability::GUEST_CAPABILITY_SECURITY_KEY,
        ]
    }

    fn scope_authorized(&self, scope: &common::IdentityScope) -> bool {
        scope.workload_id == WORKLOAD_ID
    }

    fn stream_ready(&self, _: GuestStreamMethod) -> bool {
        true
    }

    async fn serve_security_key(
        &self,
        request: guest::GuestSecurityKeyRequest,
        response: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<(), GuestSessionError> {
        assert_eq!(response.resource_handle, request.ceremony_handle);
        assert_eq!(binding.resource_handle, request.ceremony_handle);
        Ok(())
    }
}

fn admitted_psk() -> d2b_session::AdmittedBootstrapPsk {
    let binding = BootstrapPskBinding {
        operation_id: OperationId::new(OPERATION_ID.to_vec()).expect("operation id"),
        replay_nonce: REPLAY_NONCE,
        expires_at_unix_ms: NOW_MS + 1_000,
    };
    let mut admission =
        BootstrapAdmission::new(binding.clone(), BootstrapPsk::new(PSK).expect("psk"))
            .expect("bootstrap admission");
    admission
        .consume(&binding.operation_id, &binding.replay_nonce, NOW_MS)
        .expect("admitted psk")
}

async fn enrolled_pair() -> (
    d2b_guestd::service_v2::EstablishedGuestSession,
    Arc<dyn ComponentSessionDriver>,
) {
    let parent_public = parent_identity().public_key().expect("parent public");
    let guest_public = GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
        .expect("guest identity")
        .public_key()
        .expect("guest public");
    let authority =
        GuestSessionAuthority::new(material(parent_public, false), Arc::new(EnrolledStore));
    let (guest_io, parent_io) = tokio::io::duplex(256 * 1024);
    let guest =
        authority.establish_responder(FramedGuestTransport::new(guest_io), Instant::now(), NOW_MS);
    let parent = SessionEngine::establish_initiator(
        FramedGuestTransport::new(parent_io),
        guest_enrolled_policy(GENERATION, CHANNEL_BINDING),
        HandshakeCredentials::Kk {
            local_private: Secret32::new(PARENT_PRIVATE).expect("parent private"),
            remote_public: guest_public,
        },
        Instant::now(),
    );
    let (guest, parent) = tokio::join!(guest, parent);
    (
        guest.expect("guest session"),
        Arc::new(parent.expect("parent session").into_driver()),
    )
}

async fn bootstrap_pair(
    store: Arc<BootstrapStore>,
) -> (
    d2b_guestd::service_v2::EstablishedGuestSession,
    Arc<dyn ComponentSessionDriver>,
) {
    let parent_public = parent_identity().public_key().expect("parent public");
    let authority = GuestSessionAuthority::new(material(parent_public, true), store);
    let (guest_io, parent_io) = tokio::io::duplex(256 * 1024);
    let guest = authority.establish_bootstrap_initiator(
        FramedGuestTransport::new(guest_io),
        bootstrap_evidence(),
        Instant::now(),
        NOW_MS,
    );
    let parent = SessionEngine::establish_responder(
        FramedGuestTransport::new(parent_io),
        guest_bootstrap_policy(GENERATION, CHANNEL_BINDING),
        HandshakeCredentials::IkPsk2Responder {
            local_private: Secret32::new(PARENT_PRIVATE).expect("parent private"),
            psk: admitted_psk(),
        },
        Instant::now(),
    );
    let (guest, parent) = tokio::join!(guest, parent);
    (
        guest.expect("guest bootstrap session"),
        Arc::new(parent.expect("parent bootstrap session").into_driver()),
    )
}

fn reconnect_request() -> guest::GuestReconnectRequest {
    let parent_public = parent_identity().public_key().expect("parent public");
    let guest_public = GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
        .expect("guest identity")
        .public_key()
        .expect("guest public");
    guest::GuestReconnectRequest {
        context: MessageField::some(operation_context(GENERATION, WORKLOAD_ID)),
        expected_generation: GENERATION,
        guest_identity_handle: format!("guest-{}", hex(&Sha256::digest(guest_public)[..28])),
        expected_guest_static_public_key_digest: Sha256::digest(guest_public).to_vec(),
        expected_guest_identity_digest: Sha256::digest(guest_public).to_vec(),
        expected_guest_static_public_key: guest_public.to_vec(),
        expected_parent_static_public_key_digest: Sha256::digest(parent_public).to_vec(),
        required_capabilities: vec![EnumOrUnknown::new(
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
        )],
        ..Default::default()
    }
}

fn bootstrap_request() -> guest::GuestBootstrapRequest {
    let parent_public = parent_identity().public_key().expect("parent public");
    guest::GuestBootstrapRequest {
        context: MessageField::some(operation_context(GENERATION, WORKLOAD_ID)),
        expected_generation: GENERATION,
        expected_parent_static_public_key_digest: Sha256::digest(parent_public).to_vec(),
        requested_capabilities: vec![EnumOrUnknown::new(
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
        )],
        ..Default::default()
    }
}

async fn authorize_enrolled(service: &GuestServiceV2) {
    service
        .reconnect(&context(), reconnect_request())
        .await
        .expect("valid reconnect");
}

#[tokio::test]
async fn bootstrap_validates_static_digest_capabilities_and_seals_identity() {
    let parent_public = parent_identity().public_key().expect("parent public");
    let store = Arc::new(BootstrapStore {
        sealed: AtomicBool::new(false),
    });
    let authority = GuestSessionAuthority::new(material(parent_public, true), store.clone());
    let (guest_io, parent_io) = tokio::io::duplex(256 * 1024);
    let guest = authority.establish_bootstrap_initiator(
        FramedGuestTransport::new(guest_io),
        bootstrap_evidence(),
        Instant::now(),
        NOW_MS,
    );
    let parent = SessionEngine::establish_responder(
        FramedGuestTransport::new(parent_io),
        guest_bootstrap_policy(GENERATION, CHANNEL_BINDING),
        HandshakeCredentials::IkPsk2Responder {
            local_private: Secret32::new(PARENT_PRIVATE).expect("parent private"),
            psk: admitted_psk(),
        },
        Instant::now(),
    );
    let (guest, parent) = tokio::join!(guest, parent);
    let _parent = parent.expect("parent session");
    let service = service(
        Arc::new(TestOperations { ready: true }),
        guest.expect("guest session"),
    );
    assert!(
        service
            .exec(&context(), exec_request(GENERATION, WORKLOAD_ID))
            .await
            .is_err(),
        "bootstrap sessions must not execute before identity confirmation"
    );
    let request = guest::GuestBootstrapRequest {
        context: MessageField::some(operation_context(GENERATION, WORKLOAD_ID)),
        expected_generation: GENERATION,
        expected_parent_static_public_key_digest: Sha256::digest(parent_public).to_vec(),
        requested_capabilities: vec![EnumOrUnknown::new(
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
        )],
        ..Default::default()
    };
    let response = service
        .bootstrap(&context(), request)
        .await
        .expect("bootstrap response");
    assert_eq!(
        response.outcome.enum_value_or_default(),
        Outcome::OUTCOME_SUCCEEDED
    );
    assert!(store.sealed.load(Ordering::Acquire));
    assert_eq!(response.guest_identity_handle.len(), 62);
    assert_eq!(response.guest_static_public_key_digest.len(), 32);
    let guest_public = GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
        .unwrap()
        .public_key()
        .unwrap();
    assert_eq!(response.guest_static_public_key, guest_public);
    assert_eq!(
        response.guest_identity_digest,
        Sha256::digest(guest_public).to_vec()
    );
    assert!(!format!("{service:?}").contains("42424242"));
}

#[tokio::test]
async fn bootstrap_rejects_peer_operation_or_nonce_mismatch_before_handshake() {
    let parent_public = parent_identity().public_key().expect("parent public");
    let authority = GuestSessionAuthority::new(
        material(parent_public, true),
        Arc::new(BootstrapStore {
            sealed: AtomicBool::new(false),
        }),
    );
    let (guest_io, _peer_io) = tokio::io::duplex(4096);
    let wrong = BootstrapPeerEvidence::new(vec![0x7a; 16], [0x7b; 32]).unwrap();
    assert!(
        authority
            .establish_bootstrap_initiator(
                FramedGuestTransport::new(guest_io),
                wrong,
                Instant::now(),
                NOW_MS,
            )
            .await
            .is_err()
    );
    let rendered = format!("{:?}", bootstrap_evidence());
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("777777"));
}

#[tokio::test]
async fn cancelled_bootstrap_cannot_seal_or_authorize() {
    let store = Arc::new(BootstrapStore {
        sealed: AtomicBool::new(false),
    });
    let (guest, _parent) = bootstrap_pair(Arc::clone(&store)).await;
    let gate = Arc::new(BlockingAuthorizationGate {
        phase: GuestAuthorizationPhase::Bootstrap,
        reached: Notify::new(),
        release: Notify::new(),
    });
    let service = Arc::new(GuestServiceV2::with_handler_clock_and_gate(
        Arc::new(TestOperations { ready: true }),
        guest,
        Arc::new(FixedClock),
        gate.clone(),
    ));
    let bootstrap_service = Arc::clone(&service);
    let task = tokio::spawn(async move {
        bootstrap_service
            .bootstrap(&context(), bootstrap_request())
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), gate.reached.notified())
        .await
        .expect("bootstrap reached point-of-no-return gate");
    let cancelled = service
        .cancel(
            &context(),
            common::CancelRequest {
                request_id: vec![0x11; 16],
                session_generation: GENERATION,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        cancelled.outcome.enum_value_or_default(),
        common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
    );
    gate.release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(!store.sealed.load(Ordering::Acquire));
    assert!(
        service
            .exec(&context(), exec_request(GENERATION, WORKLOAD_ID))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_reconnect_cannot_authorize() {
    let (guest, _parent) = enrolled_pair().await;
    let gate = Arc::new(BlockingAuthorizationGate {
        phase: GuestAuthorizationPhase::Reconnect,
        reached: Notify::new(),
        release: Notify::new(),
    });
    let service = Arc::new(GuestServiceV2::with_handler_clock_and_gate(
        Arc::new(TestOperations { ready: true }),
        guest,
        Arc::new(FixedClock),
        gate.clone(),
    ));
    let reconnect_service = Arc::clone(&service);
    let task = tokio::spawn(async move {
        reconnect_service
            .reconnect(&context(), reconnect_request())
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), gate.reached.notified())
        .await
        .expect("reconnect reached point-of-no-return gate");
    let cancelled = service
        .cancel(
            &context(),
            common::CancelRequest {
                request_id: vec![0x11; 16],
                session_generation: GENERATION,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        cancelled.outcome.enum_value_or_default(),
        common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
    );
    gate.release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(
        service
            .exec(&context(), exec_request(GENERATION, WORKLOAD_ID))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn bootstrap_peer_evidence_is_read_from_the_peer_frame() {
    let evidence = bootstrap_evidence();
    let framed = evidence.framed_bytes();
    let (guest_io, mut peer_io) = tokio::io::duplex(4096);
    let send = async move { peer_io.write_all(&framed).await.unwrap() };
    let receive = async move {
        let mut transport = FramedGuestTransport::new(guest_io);
        transport.receive_bootstrap_evidence().await.unwrap()
    };
    let (_, received) = tokio::join!(send, receive);
    let rendered = format!("{received:?}");
    assert_eq!(rendered, "BootstrapPeerEvidence(<redacted>)");
}

#[test]
fn shared_credential_rejects_substituted_identity_digest() {
    let guest_public = GuestStaticIdentity::from_private_key(GUEST_PRIVATE)
        .unwrap()
        .public_key()
        .unwrap();
    let credential = GuestSessionCredentialV1::new(
        GENERATION,
        parent_identity().public_key().unwrap(),
        CHANNEL_BINDING,
        GuestIdentityBindingV1::Enrolled {
            guest_identity_digest: [0x55; 32],
            guest_static_public_key: guest_public,
        },
        None,
    )
    .unwrap()
    .encode()
    .unwrap();
    assert_eq!(
        GuestSessionMaterial::from_credential_bytes(&credential)
            .expect_err("substituted identity digest"),
        GuestSessionError::InvalidConfiguration
    );
}

#[tokio::test]
async fn reconnect_rejects_generation_identity_and_scope_mismatch() {
    let (guest, _parent) = enrolled_pair().await;
    let service = service(Arc::new(TestOperations { ready: true }), guest);
    let request = reconnect_request();
    request
        .validate_wire(true)
        .expect("valid reconnect request");
    let first = service
        .reconnect(&context(), request.clone())
        .await
        .expect("valid reconnect");
    let replay = service
        .reconnect(&context(), request.clone())
        .await
        .expect("idempotent reconnect replay");
    assert_eq!(first, replay);
    let mut wrong_generation = request.clone();
    wrong_generation.expected_generation += 1;
    assert!(
        service
            .reconnect(&context(), wrong_generation)
            .await
            .is_err()
    );
    let mut substituted_digest = request.clone();
    substituted_digest.expected_guest_identity_digest = vec![0x99; 32];
    let substituted_metadata = substituted_digest
        .context
        .as_mut()
        .unwrap()
        .metadata
        .as_mut()
        .unwrap();
    substituted_metadata.request_id = vec![0x14; 16];
    substituted_metadata.idempotency_key = vec![0x25; 16];
    assert!(
        service
            .reconnect(&context(), substituted_digest)
            .await
            .is_err()
    );
    let mut wrong_scope = request;
    wrong_scope
        .context
        .as_mut()
        .unwrap()
        .scope
        .as_mut()
        .unwrap()
        .workload_id = "other".to_owned();
    assert!(service.reconnect(&context(), wrong_scope).await.is_err());
}

#[tokio::test]
async fn exec_server_reserves_stream_and_transfers_one_message_per_frame() {
    let (guest, parent) = enrolled_pair().await;
    let service = service(Arc::new(TestOperations { ready: true }), guest);
    let request = exec_request(GENERATION, WORKLOAD_ID);
    assert!(
        service.exec(&context(), request.clone()).await.is_err(),
        "enrolled sessions must reconnect before operations"
    );
    authorize_enrolled(&service).await;
    let mut request = request;
    let request_metadata = request
        .terminal
        .as_mut()
        .unwrap()
        .metadata
        .as_mut()
        .unwrap();
    request_metadata.request_id = vec![0x13; 16];
    request_metadata.idempotency_key = vec![0x24; 16];
    let response = service
        .exec(&context(), request)
        .await
        .expect("accepted exec");
    assert_eq!(
        response.outcome.enum_value_or_default(),
        Outcome::OUTCOME_ACCEPTED
    );
    let stream_number = parse_server_stream_name(&response.stream_id).expect("stream name");
    let stream = StreamId::new(stream_number).expect("stream");
    let credit = LimitProfile::local_default().named_stream_queue_bytes;
    parent
        .open_named_stream(stream, credit, credit)
        .await
        .expect("host opens once");
    let selection = terminal::TerminalStreamFrame {
        session_generation: GENERATION,
        request_id: response.request_id.clone(),
        sequence: 0,
        operation_id: response.operation_id.clone(),
        resource_handle: response.resource_handle.clone(),
        frame: Some(terminal::terminal_stream_frame::Frame::Select(
            terminal::TerminalSelection {
                selection: Some(terminal::terminal_selection::Selection::Exec(
                    terminal::ExecSelection {
                        authority: EnumOrUnknown::new(
                            terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY,
                        ),
                        selection: Some(terminal::exec_selection::Selection::Arbitrary(
                            terminal::ArbitraryExecSelection {
                                argv: vec![b"/bin/true".to_vec()],
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    parent
        .send_named_stream(stream, selection.write_to_bytes().expect("encode"))
        .await
        .expect("send selection");
    let mut kinds = Vec::new();
    while kinds.len() < 2 {
        match parent
            .receive_named_stream()
            .await
            .expect("receive terminal frame")
        {
            StreamEvent::Data { stream: got, bytes } => {
                assert_eq!(got, stream);
                parent
                    .grant_named_stream_credit(stream, bytes.len() as u32)
                    .await
                    .expect("credit");
                let frame =
                    terminal::TerminalStreamFrame::parse_from_bytes(&bytes).expect("one frame");
                kinds.push(frame.frame.expect("payload"));
            }
            StreamEvent::RemoteClosed { .. } => break,
            StreamEvent::Reset { .. } => panic!("stream reset"),
        }
    }
    assert!(matches!(
        kinds.as_slice(),
        [
            terminal::terminal_stream_frame::Frame::Started(_),
            terminal::terminal_stream_frame::Frame::Outcome(_)
        ]
    ));
}

#[tokio::test]
async fn backend_failure_returns_no_stream_and_redacted_debug() {
    let (guest, parent) = enrolled_pair().await;
    let service = service(Arc::new(TestOperations { ready: false }), guest);
    authorize_enrolled(&service).await;
    assert!(
        service
            .exec(&context(), exec_request(GENERATION, WORKLOAD_ID))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), parent.receive_named_stream())
            .await
            .is_err()
    );
    let material = material(parent_identity().public_key().unwrap(), false);
    let debug = format!("{material:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("31313131"));
}

#[tokio::test]
async fn expired_requests_fail_and_cancel_signals_the_active_operation() {
    let (guest, _parent) = enrolled_pair().await;
    let started = Arc::new(Notify::new());
    let service = Arc::new(service(
        Arc::new(BlockingOperations {
            started: Arc::clone(&started),
        }),
        guest,
    ));
    authorize_enrolled(&service).await;

    let mut expired = exec_request(GENERATION, WORKLOAD_ID);
    let expired_metadata = expired
        .terminal
        .as_mut()
        .unwrap()
        .metadata
        .as_mut()
        .unwrap();
    expired_metadata.request_id = vec![0x31; 16];
    expired_metadata.idempotency_key = vec![0x41; 16];
    expired_metadata.issued_at_unix_ms = NOW_MS - 2_000;
    expired_metadata.expires_at_unix_ms = NOW_MS - 1_000;
    assert!(service.exec(&context(), expired).await.is_err());

    let mut inspect_metadata = metadata(GENERATION);
    inspect_metadata.request_id = vec![0x32; 16];
    inspect_metadata.idempotency_key.clear();
    let inspect = guest::GuestInspectExecRequest {
        context: MessageField::some(guest::GuestOperationContext {
            metadata: MessageField::some(inspect_metadata.clone()),
            scope: MessageField::some(scope(WORKLOAD_ID)),
            operation_id: "inspect-operation".to_owned(),
            request_digest: vec![0x52; 32],
            ..Default::default()
        }),
        query: MessageField::some(guest::GuestInspectExecQuery {
            query: Some(guest::guest_inspect_exec_query::Query::Status(
                guest::GuestExecStatusQuery {
                    resource_handle: "exec-handle".to_owned(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
        ..Default::default()
    };
    let inspect_service = Arc::clone(&service);
    let mut task =
        tokio::spawn(async move { inspect_service.inspect_exec(&context(), inspect).await });
    tokio::select! {
        () = started.notified() => {}
        result = &mut task => panic!("inspect failed before dispatch: {result:?}"),
        () = tokio::time::sleep(Duration::from_secs(1)) => panic!("inspect dispatch timed out"),
    }

    let response = service
        .cancel(
            &context(),
            common::CancelRequest {
                request_id: inspect_metadata.request_id,
                session_generation: GENERATION,
                ..Default::default()
            },
        )
        .await
        .expect("cancel response");
    assert_eq!(
        response.outcome.enum_value_or_default(),
        common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancelled inspect completion")
            .expect("inspect task")
            .is_err()
    );
}

#[tokio::test]
async fn security_key_resume_reuses_the_existing_ceremony_handle() {
    let (guest, _parent) = enrolled_pair().await;
    let service = service(Arc::new(ResumeOperations), guest);
    authorize_enrolled(&service).await;
    let mut resume_metadata = metadata(GENERATION);
    resume_metadata.request_id = vec![0x61; 16];
    resume_metadata.idempotency_key = vec![0x62; 16];
    let response = service
        .security_key(
            &context(),
            guest::GuestSecurityKeyRequest {
                context: MessageField::some(guest::GuestOperationContext {
                    metadata: MessageField::some(resume_metadata),
                    scope: MessageField::some(scope(WORKLOAD_ID)),
                    operation_id: "resume-operation".to_owned(),
                    request_digest: vec![0x63; 32],
                    ..Default::default()
                }),
                action: EnumOrUnknown::new(
                    guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_RESUME,
                ),
                device_handle: "device-1".to_owned(),
                ceremony_handle: "ceremony-existing".to_owned(),
                ceremony: EnumOrUnknown::new(
                    guest::GuestSecurityKeyCeremonyKind::GUEST_SECURITY_KEY_CEREMONY_KIND_U2F,
                ),
                ..Default::default()
            },
        )
        .await
        .expect("resume accepted");
    assert_eq!(response.resource_handle, "ceremony-existing");
}

#[test]
fn static_identity_uses_x25519_and_session_material_is_strict() {
    let private = decode_hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let expected = decode_hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
    let identity = GuestStaticIdentity::from_private_key(private).expect("identity");
    assert_eq!(identity.public_key().expect("public key"), expected);
    let malformed = vec![0_u8; 156];
    assert_eq!(
        GuestSessionMaterial::decode(&malformed).expect_err("old material"),
        GuestSessionError::InvalidConfiguration
    );
    assert_eq!(
        d2b_guestd::service_v2::DAEMON_SERVICE_SCHEMA_FINGERPRINT_HEX,
        "4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a"
    );
    assert_eq!(
        d2b_guestd::service_v2::GUEST_SERVICE_SCHEMA_FINGERPRINT_HEX,
        "e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d"
    );
}

fn decode_hex(value: &str) -> [u8; 32] {
    let mut decoded = [0_u8; 32];
    for (index, slot) in decoded.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).expect("hex");
    }
    decoded
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
