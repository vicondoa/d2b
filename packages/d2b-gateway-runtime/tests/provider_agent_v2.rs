use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{CloseReason, Remediation, RequestId, SessionErrorCode},
    v2_identity::{ProviderId, ProviderType},
};
use d2b_gateway_runtime::provider_agent::{
    ProviderAgentAuditOutcome, ProviderAgentError, ProviderAgentProcess,
};
use d2b_provider::{ProviderRegistry, ProviderRegistryBuilder};
use d2b_provider_toolkit::{DeterministicClock, FakeProvider, Fixture, register_exact_instances};
use d2b_session::{
    Cancellation, ComponentSessionDriver, OwnedAttachment, SessionError, SessionEvent, StreamEvent,
    StreamId,
};
use protobuf::Message;
use tokio::sync::{Mutex, mpsc};
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MessageHeader};

struct StubDriver {
    generation: u64,
}

fn unsupported() -> d2b_session::Result<()> {
    Err(SessionError::new(SessionErrorCode::SessionDisconnected))
}

#[async_trait]
impl ComponentSessionDriver for StubDriver {
    fn generation(&self) -> u64 {
        self.generation
    }

    async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }

    async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
        unsupported().map(|()| Vec::new())
    }

    async fn register_inbound_call(&self, _: RequestId) -> d2b_session::Result<Cancellation> {
        unsupported().map(|()| unreachable!())
    }

    async fn complete_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }

    async fn remove_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }

    async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
        unsupported().map(|()| Vec::new())
    }

    async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
        unsupported().map(|()| unreachable!())
    }

    async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn drive_keepalive(&self, _: Instant) -> d2b_session::Result<()> {
        unsupported()
    }

    async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
        unsupported().map(|()| unreachable!())
    }

    async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
        unsupported()
    }
}

fn registry_for(provider_type: ProviderType, ordinal: usize) -> (ProviderRegistry, Fixture) {
    let fixture = Fixture::new(provider_type, ordinal).unwrap();
    let descriptor = fixture.descriptor.clone();
    let instance = Arc::new(FakeProvider::new(fixture.clone())).instance();
    let mut builder = ProviderRegistryBuilder::new(
        descriptor.registry_generation,
        descriptor.configured_scope_digest.clone(),
        fixture.now_unix_ms,
    );
    register_exact_instances(&mut builder, [instance]).unwrap();
    (builder.finish().unwrap(), fixture)
}

fn process_for(
    provider_type: ProviderType,
    ordinal: usize,
    driver: Arc<dyn ComponentSessionDriver>,
) -> (Arc<ProviderAgentProcess>, Fixture) {
    let (registry, fixture) = registry_for(provider_type, ordinal);
    let process = ProviderAgentProcess::from_registry_with(
        &registry,
        &fixture.descriptor.provider_id,
        driver,
        Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
        8,
    )
    .unwrap();
    (process, fixture)
}

#[test]
fn exposes_every_frozen_provider_service_family() {
    let expected = [
        (ProviderType::Runtime, "RuntimeProviderService"),
        (
            ProviderType::Infrastructure,
            "InfrastructureProviderService",
        ),
        (ProviderType::Transport, "TransportProviderService"),
        (ProviderType::Substrate, "SubstrateProviderService"),
        (ProviderType::Credential, "CredentialProviderService"),
        (ProviderType::Display, "DisplayProviderService"),
        (ProviderType::Network, "NetworkProviderService"),
        (ProviderType::Storage, "StorageProviderService"),
        (ProviderType::Device, "DeviceProviderService"),
        (ProviderType::Audio, "AudioProviderService"),
        (ProviderType::Observability, "ObservabilityProviderService"),
    ];
    for (ordinal, (provider_type, service)) in expected.into_iter().enumerate() {
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(StubDriver { generation: 7 });
        let (process, _) = process_for(provider_type, ordinal, driver);
        assert_eq!(process.provider_type(), provider_type);
        assert_eq!(
            process.service_names(),
            [format!("d2b.provider.v2.{service}")]
        );
    }
}

#[test]
fn rejects_unregistered_and_invalid_session_adapters() {
    let (registry, fixture) = registry_for(ProviderType::Runtime, 0);
    let missing = Fixture::new(ProviderType::Display, 8)
        .unwrap()
        .descriptor
        .provider_id;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(StubDriver { generation: 1 });
    assert!(matches!(
        ProviderAgentProcess::from_registry(&registry, &missing, driver),
        Err(ProviderAgentError::UnregisteredAdapter)
    ));

    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(StubDriver { generation: 0 });
    assert!(matches!(
        ProviderAgentProcess::from_registry(&registry, &fixture.descriptor.provider_id, driver),
        Err(ProviderAgentError::RegistrationRejected)
    ));
}

struct ChannelDriver {
    generation: u64,
    inbound: Mutex<mpsc::Receiver<Vec<u8>>>,
    outbound: mpsc::Sender<Vec<u8>>,
}

#[async_trait]
impl ComponentSessionDriver for ChannelDriver {
    fn generation(&self) -> u64 {
        self.generation
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

    async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }
    async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn register_inbound_call(&self, _: RequestId) -> d2b_session::Result<Cancellation> {
        unsupported().map(|()| unreachable!())
    }
    async fn complete_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }
    async fn remove_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
        unsupported().map(|()| false)
    }
    async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
        unsupported().map(|()| Vec::new())
    }
    async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
        unsupported().map(|()| unreachable!())
    }
    async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn drive_keepalive(&self, _: Instant) -> d2b_session::Result<()> {
        unsupported()
    }
    async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
        unsupported().map(|()| unreachable!())
    }
    async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
        unsupported()
    }
}

#[tokio::test]
async fn malformed_requests_return_closed_errors_and_bounded_audit() {
    let (request_tx, request_rx) = mpsc::channel(4);
    let (response_tx, mut response_rx) = mpsc::channel(4);
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(ChannelDriver {
        generation: 9,
        inbound: Mutex::new(request_rx),
        outbound: response_tx,
    });
    let (process, _) = process_for(ProviderType::Runtime, 0, driver);
    let service = process.service_names().pop().unwrap();
    let request = ttrpc::Request {
        service,
        method: "Health".to_owned(),
        payload: vec![0xff],
        ..Default::default()
    };
    let body = request.write_to_bytes().unwrap();
    let mut frame = Vec::from(MessageHeader::new_request(1, body.len() as u32));
    frame.extend_from_slice(&body);

    let task = tokio::spawn(process.clone().serve());
    request_tx.send(frame).await.unwrap();
    let response_frame = response_rx.recv().await.unwrap();
    let header = MessageHeader::from(&response_frame[..MESSAGE_HEADER_LENGTH]);
    assert_eq!(header.stream_id, 1);
    let response =
        ttrpc::Response::parse_from_bytes(&response_frame[MESSAGE_HEADER_LENGTH..]).unwrap();
    assert_eq!(
        response.status.as_ref().unwrap().message,
        "provider-request-rejected"
    );
    assert_eq!(
        process.audit_snapshot()[0].outcome,
        ProviderAgentAuditOutcome::Rejected
    );
    task.abort();
}

#[test]
fn provider_errors_do_not_expose_registration_identity() {
    let provider_id = ProviderId::parse("bbbbbbbbbbbbbbbbbbba").unwrap();
    let rendered = format!("{:?}", ProviderAgentError::UnregisteredAdapter);
    assert!(!rendered.contains(provider_id.as_str()));
}
