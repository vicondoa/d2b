use std::{
    sync::{Arc, Mutex as StdMutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        CloseReason, EndpointRole, PurposeClass, Remediation, RequestId, SessionErrorCode,
    },
    v2_identity::RealmId,
    v2_services::common::{self, CancelOutcome, Outcome},
};
use d2b_realm_router::service_v2::{
    CredentialCustody, REALM_SERVICE_NAME, RealmServiceError, RealmServiceLimits,
    RealmServiceProcess, RealmServiceServer, RealmSessionAuthority, SystemRealmClock,
};
use d2b_session::{
    Cancellation, ComponentSessionDriver, OwnedAttachment, RequestRegistry, SessionError,
    SessionEvent, StreamEvent, StreamId,
};
use protobuf::{Message, MessageField};
use tokio::sync::{Mutex, mpsc};
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MessageHeader};

const GENERATION: u64 = 9;
const REALM: &str = "aaaaaaaaaaaaaaaaaaaa";

struct ChannelDriver {
    inbound: Mutex<mpsc::Receiver<Vec<u8>>>,
    outbound: mpsc::Sender<Vec<u8>>,
    requests: StdMutex<RequestRegistry>,
}

fn disconnected<T>() -> d2b_session::Result<T> {
    Err(SessionError::new(SessionErrorCode::SessionDisconnected))
}

#[async_trait]
impl ComponentSessionDriver for ChannelDriver {
    fn generation(&self) -> u64 {
        GENERATION
    }

    async fn send_ttrpc(&self, frame: Vec<u8>) -> d2b_session::Result<()> {
        self.outbound
            .send(frame)
            .await
            .map_err(|_| SessionError::new(SessionErrorCode::SessionDisconnected))
    }

    async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
        self.inbound
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| SessionError::new(SessionErrorCode::SessionDisconnected))
    }

    async fn register_inbound_call(
        &self,
        request_id: RequestId,
    ) -> d2b_session::Result<Cancellation> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register(request_id)
    }

    async fn complete_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
        Ok(self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .complete(&request_id))
    }

    async fn remove_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
        Ok(self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request_id))
    }

    async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
        disconnected()
    }
    async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
        disconnected()
    }
    async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
        disconnected()
    }
    async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn drive_keepalive(&self, _: Instant) -> d2b_session::Result<()> {
        disconnected()
    }
    async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
        disconnected()
    }
    async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
        disconnected()
    }
}

fn gateway_authority() -> RealmSessionAuthority {
    RealmSessionAuthority::gateway_peer(
        RealmId::parse(REALM).unwrap(),
        EndpointRole::RealmController,
        PurposeClass::Enrolled,
    )
    .unwrap()
}

fn request(sequence: u8, operation: &str, resource: &str, digest: u8) -> common::ServiceRequest {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut metadata = common::RequestMetadata::new();
    metadata.request_id = vec![sequence; 16];
    metadata.correlation_id = format!("correlation-{sequence}");
    metadata.trace_id = vec![sequence.saturating_add(1); 16];
    metadata.idempotency_key = vec![sequence.saturating_add(2); 16];
    metadata.issued_at_unix_ms = now;
    metadata.expires_at_unix_ms = now + 30_000;
    metadata.session_generation = GENERATION;
    let mut scope = common::IdentityScope::new();
    scope.realm_id = REALM.to_owned();
    let mut request = common::ServiceRequest::new();
    request.metadata = MessageField::some(metadata);
    request.scope = MessageField::some(scope);
    request.resource_id = resource.to_owned();
    request.operation_id = operation.to_owned();
    request.request_digest = vec![digest; 32];
    request
}

async fn rpc<M: Message>(
    tx: &mpsc::Sender<Vec<u8>>,
    rx: &mut mpsc::Receiver<Vec<u8>>,
    stream_id: u32,
    method: &str,
    message: &M,
) -> ttrpc::Response {
    let request = ttrpc::Request {
        service: REALM_SERVICE_NAME.to_owned(),
        method: method.to_owned(),
        payload: message.write_to_bytes().unwrap(),
        ..Default::default()
    };
    let body = request.write_to_bytes().unwrap();
    let mut frame = Vec::from(MessageHeader::new_request(stream_id, body.len() as u32));
    frame.extend_from_slice(&body);
    tx.send(frame).await.unwrap();
    let frame = rx.recv().await.unwrap();
    let header = MessageHeader::from(&frame[..MESSAGE_HEADER_LENGTH]);
    assert_eq!(header.stream_id, stream_id);
    ttrpc::Response::parse_from_bytes(&frame[MESSAGE_HEADER_LENGTH..]).unwrap()
}

fn status_code(response: &ttrpc::Response) -> ttrpc::Code {
    response.status.as_ref().unwrap().code.enum_value().unwrap()
}

fn service_response(response: &ttrpc::Response) -> common::ServiceResponse {
    assert_eq!(
        status_code(response),
        ttrpc::Code::OK,
        "{}",
        response.status.as_ref().unwrap().message
    );
    common::ServiceResponse::parse_from_bytes(&response.payload).unwrap()
}

fn process_with_limits(
    limits: RealmServiceLimits,
) -> (
    Arc<RealmServiceProcess>,
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<Vec<u8>>,
) {
    let (request_tx, request_rx) = mpsc::channel(16);
    let (response_tx, response_rx) = mpsc::channel(16);
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(ChannelDriver {
        inbound: Mutex::new(request_rx),
        outbound: response_tx,
        requests: StdMutex::new(RequestRegistry::new(GENERATION).unwrap()),
    });
    let server = RealmServiceServer::new_with(
        gateway_authority(),
        driver.clone(),
        7,
        limits,
        Arc::new(SystemRealmClock),
    )
    .unwrap();
    let process = RealmServiceProcess::from_server(server, driver).unwrap();
    (process, request_tx, response_rx)
}

fn process() -> (
    Arc<RealmServiceProcess>,
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<Vec<u8>>,
) {
    process_with_limits(RealmServiceLimits::default())
}

#[test]
fn authority_keeps_remote_credentials_in_gateway_guests() {
    let authority = gateway_authority();
    assert_eq!(
        authority.credential_custody(),
        CredentialCustody::GatewayGuest
    );
    assert_eq!(authority.purpose(), PurposeClass::Enrolled);
    assert!(matches!(
        RealmSessionAuthority::new(
            RealmId::parse(REALM).unwrap(),
            EndpointRole::RemotePeer,
            d2b_contracts::v2_component_session::Locality::HostLocal,
            PurposeClass::Local,
            CredentialCustody::GatewayGuest,
        ),
        Err(RealmServiceError::InvalidAuthority)
    ));
}

#[tokio::test]
async fn authenticated_bootstrap_enrollment_route_and_shortcut_lifecycle() {
    let (process, tx, mut rx) = process();
    assert_eq!(process.service_names(), [REALM_SERVICE_NAME]);
    let task = tokio::spawn(process.clone().serve());

    let unresolved = rpc(
        &tx,
        &mut rx,
        1,
        "ResolveRoute",
        &request(1, "resolve-before-enroll", REALM, 1),
    )
    .await;
    assert_eq!(status_code(&unresolved), ttrpc::Code::NOT_FOUND);

    let bootstrap = service_response(
        &rpc(
            &tx,
            &mut rx,
            3,
            "Bootstrap",
            &request(2, "bootstrap-work", REALM, 2),
        )
        .await,
    );
    assert_eq!(
        bootstrap.outcome.enum_value().unwrap(),
        Outcome::OUTCOME_SUCCEEDED
    );

    let enroll = service_response(
        &rpc(
            &tx,
            &mut rx,
            5,
            "Enroll",
            &request(3, "enroll-work", REALM, 3),
        )
        .await,
    );
    assert_eq!(enroll.resource_handle, REALM);

    let route = service_response(
        &rpc(
            &tx,
            &mut rx,
            7,
            "ResolveRoute",
            &request(4, "resolve-work", REALM, 4),
        )
        .await,
    );
    assert_eq!(route.resource_handle, REALM);
    assert_eq!(route.result_digest.len(), 32);

    let mut authorize = request(5, "authorize-shortcut", "shortcut-a", 5);
    authorize.stream_id = REALM.to_owned();
    let shortcut = service_response(&rpc(&tx, &mut rx, 9, "AuthorizeShortcut", &authorize).await);
    assert_eq!(shortcut.resource_handle, "shortcut-a");
    assert_eq!(process.server().shortcut_count(), 1);

    let revoke = request(6, "revoke-shortcut", "shortcut-a", 5);
    let revoked = service_response(&rpc(&tx, &mut rx, 11, "RevokeShortcut", &revoke).await);
    assert_eq!(revoked.resource_handle, "shortcut-a");

    let close = request(7, "close-shortcut", "shortcut-a", 5);
    let closed = service_response(&rpc(&tx, &mut rx, 13, "ReportShortcutClose", &close).await);
    assert_eq!(closed.resource_handle, "shortcut-a");

    let mut inspect = request(8, "", "", 0);
    inspect.request_digest.clear();
    inspect.metadata.as_mut().unwrap().idempotency_key.clear();
    inspect.page_size = 1;
    let first = service_response(&rpc(&tx, &mut rx, 15, "Inspect", &inspect).await);
    assert_eq!(first.observations.len(), 1);
    assert!(!first.next_page_cursor.is_empty());

    task.abort();
}

#[tokio::test]
async fn rejects_relay_identity_as_local_authority_and_reports_cancel_outcomes() {
    let bad = RealmSessionAuthority::new(
        RealmId::parse(REALM).unwrap(),
        EndpointRole::RemotePeer,
        d2b_contracts::v2_component_session::Locality::HostLocal,
        PurposeClass::Local,
        CredentialCustody::None,
    );
    assert_eq!(bad.unwrap_err(), RealmServiceError::InvalidAuthority);

    let (process, tx, mut rx) = process();
    let task = tokio::spawn(process.clone().serve());
    let mut wrong_scope = request(9, "wrong-realm", REALM, 9);
    wrong_scope.scope.as_mut().unwrap().realm_id = "baaaaaaaaaaaaaaaaaaa".to_owned();
    let denied = rpc(&tx, &mut rx, 9, "Bootstrap", &wrong_scope).await;
    assert_eq!(status_code(&denied), ttrpc::Code::PERMISSION_DENIED);

    let cancel = common::CancelRequest {
        request_id: vec![0x44; 16],
        session_generation: GENERATION,
        ..Default::default()
    };
    let response = rpc(&tx, &mut rx, 11, "Cancel", &cancel).await;
    assert_eq!(
        status_code(&response),
        ttrpc::Code::OK,
        "{}",
        response.status.as_ref().unwrap().message
    );
    let cancel_response = common::CancelResponse::parse_from_bytes(&response.payload).unwrap();
    assert_eq!(
        cancel_response.outcome.enum_value().unwrap(),
        CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
    );

    let stale_cancel = common::CancelRequest {
        session_generation: GENERATION + 1,
        ..cancel
    };
    let response = rpc(&tx, &mut rx, 13, "Cancel", &stale_cancel).await;
    let cancel_response = common::CancelResponse::parse_from_bytes(&response.payload).unwrap();
    assert_eq!(
        cancel_response.outcome.enum_value().unwrap(),
        CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
    );

    task.abort();
}

#[tokio::test]
async fn shortcut_capacity_fails_closed() {
    let limits = RealmServiceLimits {
        max_bindings: 2,
        max_shortcuts: 1,
        max_mutation_records: 8,
        audit_capacity: 8,
    };
    let (process, tx, mut rx) = process_with_limits(limits);
    let task = tokio::spawn(process.serve());

    service_response(
        &rpc(
            &tx,
            &mut rx,
            1,
            "Bootstrap",
            &request(1, "bootstrap-capacity", REALM, 1),
        )
        .await,
    );
    service_response(
        &rpc(
            &tx,
            &mut rx,
            3,
            "Enroll",
            &request(2, "enroll-capacity", REALM, 2),
        )
        .await,
    );

    let mut first = request(3, "authorize-first", "shortcut-first", 3);
    first.stream_id = REALM.to_owned();
    service_response(&rpc(&tx, &mut rx, 5, "AuthorizeShortcut", &first).await);

    let mut second = request(4, "authorize-second", "shortcut-second", 4);
    second.stream_id = REALM.to_owned();
    let rejected = rpc(&tx, &mut rx, 7, "AuthorizeShortcut", &second).await;
    assert_eq!(status_code(&rejected), ttrpc::Code::RESOURCE_EXHAUSTED);
    assert_eq!(
        rejected.status.as_ref().unwrap().message,
        "realm-shortcut-table-full"
    );

    task.abort();
}
