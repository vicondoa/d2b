use std::{fmt, sync::Arc, time::Instant};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointPolicy, RequestId, ServicePackage, TransportClass},
    v2_services::{common, decode_strict},
};
use d2b_session::{ComponentSessionDriver, HandshakeCredentials, SessionEngine};
use d2b_unix_session::UnixSeqpacketTransport;
use protobuf::Message;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::Mutex,
};
use ttrpc::{
    r#async::transport::Socket,
    proto::{MESSAGE_HEADER_LENGTH, MessageHeader},
};

use crate::{
    ClientError, ComponentSessionConnector, ConnectedSession, ResolvedTarget, ServiceKind,
    TransportKind,
};

struct PendingSession {
    transport: UnixSeqpacketTransport,
    policy: EndpointPolicy,
    credentials: HandshakeCredentials,
}

pub struct HostSocketConnector {
    transport: TransportKind,
    pending: Mutex<Option<PendingSession>>,
}

impl HostSocketConnector {
    pub fn new(
        transport: UnixSeqpacketTransport,
        policy: EndpointPolicy,
        credentials: HandshakeCredentials,
    ) -> Result<Self, ClientError> {
        let selected = match policy.transport_binding.transport {
            TransportClass::UnixSeqpacket => TransportKind::LocalUnix,
            TransportClass::InheritedSocketpair => TransportKind::InheritedSocket,
            _ => return Err(ClientError::TransportPolicyMismatch),
        };
        Ok(Self {
            transport: selected,
            pending: Mutex::new(Some(PendingSession {
                transport,
                policy,
                credentials,
            })),
        })
    }
}

impl fmt::Debug for HostSocketConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostSocketConnector([redacted])")
    }
}

#[async_trait]
impl ComponentSessionConnector for HostSocketConnector {
    async fn connect(
        &self,
        target: &ResolvedTarget,
        service: ServiceKind,
    ) -> Result<ConnectedSession, ClientError> {
        if target.transport() != self.transport {
            return Err(ClientError::TransportPolicyMismatch);
        }
        let pending = self
            .pending
            .lock()
            .await
            .take()
            .ok_or(ClientError::ConnectFailed)?;
        if pending.policy.service != service_package(service) {
            return Err(ClientError::InvalidService);
        }
        let engine = SessionEngine::establish_initiator(
            pending.transport,
            pending.policy,
            pending.credentials,
            Instant::now(),
        )
        .await
        .map_err(|_| ClientError::ConnectFailed)?;
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
        let (client, bridge) = tokio::io::duplex(2 * 1024 * 1024);
        tokio::spawn(pump_ttrpc(bridge, Arc::clone(&driver)));
        Ok(ConnectedSession {
            driver,
            ttrpc_socket: Socket::new(client),
        })
    }
}

async fn pump_ttrpc(
    mut socket: DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
) -> Result<(), ()> {
    loop {
        let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
        socket.read_exact(&mut header_bytes).await.map_err(|_| ())?;
        let header = MessageHeader::from(header_bytes);
        if header.length as usize
            > d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES as usize
        {
            return Err(());
        }
        let mut body = vec![0_u8; header.length as usize];
        socket.read_exact(&mut body).await.map_err(|_| ())?;
        let request = ttrpc::Request::parse_from_bytes(&body).map_err(|_| ())?;
        let request_id = request_id(&request)?;
        let mut frame = header_bytes.to_vec();
        frame.extend_from_slice(&body);
        let response = driver.invoke(request_id, frame).await.map_err(|_| ())?;
        socket.write_all(&response).await.map_err(|_| ())?;
    }
}

fn request_id(request: &ttrpc::Request) -> Result<RequestId, ()> {
    let bytes = if request.method == "Cancel" {
        decode_strict::<common::CancelRequest>(&request.payload, false)
            .map_err(|_| ())?
            .request_id
    } else {
        decode_strict::<common::ServiceRequest>(&request.payload, false)
            .map_err(|_| ())?
            .metadata
            .as_ref()
            .ok_or(())?
            .request_id
            .clone()
    };
    RequestId::new(bytes).map_err(|_| ())
}

fn service_package(service: ServiceKind) -> ServicePackage {
    match service {
        ServiceKind::Daemon => ServicePackage::DaemonV2,
        ServiceKind::Realm => ServicePackage::RealmV2,
        ServiceKind::Guest => ServicePackage::GuestV2,
        ServiceKind::ProviderRuntime
        | ServiceKind::ProviderInfrastructure
        | ServiceKind::ProviderTransport
        | ServiceKind::ProviderSubstrate
        | ServiceKind::ProviderCredential
        | ServiceKind::ProviderDisplay
        | ServiceKind::ProviderNetwork
        | ServiceKind::ProviderStorage
        | ServiceKind::ProviderDevice
        | ServiceKind::ProviderAudio
        | ServiceKind::ProviderObservability => ServicePackage::ProviderV2,
        ServiceKind::Broker => ServicePackage::BrokerV2,
        ServiceKind::User => ServicePackage::UserV2,
        ServiceKind::RuntimeSystemdUser => ServicePackage::RuntimeSystemdUserV2,
        ServiceKind::Shell => ServicePackage::ShellV2,
        ServiceKind::Clipboard => ServicePackage::ClipboardV2,
        ServiceKind::ClipboardPicker => ServicePackage::ClipboardPickerV2,
        ServiceKind::Notify => ServicePackage::NotifyV2,
        ServiceKind::SecurityKey => ServicePackage::SecurityKeyV2,
        ServiceKind::Wayland => ServicePackage::WaylandV2,
        ServiceKind::Activation => ServicePackage::ActivationV2,
        ServiceKind::Tty => ServicePackage::TtyV2,
    }
}
