use std::{
    fmt,
    fs::File,
    io::{ErrorKind, Read, Write},
    os::fd::OwnedFd,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::v2_provider::OperationId;
use tokio::io::unix::AsyncFd;

use crate::{
    BundleEndpointId, EndpointCloseRequest, EndpointCloseResult, EndpointConnectRequest,
    EndpointConnection, EndpointConnectionMetadata, EndpointInspectRequest, EndpointLeaseId,
    EndpointObservation, EndpointObservationState, EndpointPortError, EndpointSource,
    LocalEndpointPort, LocalTransportKind, OwnedEndpointConnection, OwnedEndpointDescriptor,
    OwnedLocalTransport, ReachabilityEvidence,
};

const MAX_CLOUD_HYPERVISOR_ACK_BYTES: usize = 64;

/// Opaque resolver capability accepted by the production connector.
#[derive(Clone)]
pub enum EndpointCapabilityId {
    VerifiedBundle(BundleEndpointId),
    AuthorizedLease(EndpointLeaseId),
}

impl fmt::Debug for EndpointCapabilityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::VerifiedBundle(_) => "EndpointCapabilityId::VerifiedBundle(<redacted>)",
            Self::AuthorizedLease(_) => "EndpointCapabilityId::AuthorizedLease(<redacted>)",
        })
    }
}

/// Closed request by which an integrator supplies a pre-authorized descriptor.
#[derive(Clone)]
pub struct EndpointResolveRequest {
    pub operation_id: OperationId,
    pub capability_id: EndpointCapabilityId,
    pub kind: LocalTransportKind,
    pub deadline: Duration,
}

impl fmt::Debug for EndpointResolveRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointResolveRequest")
            .field("capability_id", &self.capability_id)
            .field("kind", &self.kind)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

/// Resolver seam for opaque bundle and lease capabilities.
///
/// The resolver performs only capability-owner resolution or acceptance and
/// never authorizes by reachability or performs a transport handshake. Every
/// call must transfer exclusive ownership of a fresh connected descriptor
/// bound to verified identity and generation evidence; returning a duplicate
/// of a descriptor used by another handle violates this port contract.
#[async_trait]
pub trait LocalEndpointResolver: Send + Sync {
    async fn resolve(
        &self,
        request: EndpointResolveRequest,
    ) -> Result<OwnedEndpointDescriptor, EndpointPortError>;
}

/// Tokio production adapter for all canonical local transport kinds.
pub struct TokioLocalEndpointPort {
    resolver: Arc<dyn LocalEndpointResolver>,
}

impl TokioLocalEndpointPort {
    pub fn new(resolver: Arc<dyn LocalEndpointResolver>) -> Self {
        Self { resolver }
    }

    async fn connect_inner(
        &self,
        request: EndpointConnectRequest,
    ) -> Result<EndpointConnection, EndpointPortError> {
        if request.endpoint.kind() != request.kind
            || request.capabilities != request.kind.capability_profile()
        {
            return Err(EndpointPortError::InvariantViolation);
        }

        let descriptor = match request.endpoint.clone() {
            EndpointSource::Bundle { endpoint_id, .. } => {
                self.resolver
                    .resolve(EndpointResolveRequest {
                        operation_id: request.operation_id.clone(),
                        capability_id: EndpointCapabilityId::VerifiedBundle(endpoint_id),
                        kind: request.kind,
                        deadline: request.deadline,
                    })
                    .await?
            }
            EndpointSource::Lease { lease_id, .. } => {
                self.resolver
                    .resolve(EndpointResolveRequest {
                        operation_id: request.operation_id.clone(),
                        capability_id: EndpointCapabilityId::AuthorizedLease(lease_id),
                        kind: request.kind,
                        deadline: request.deadline,
                    })
                    .await?
            }
            EndpointSource::Owned(capability) => capability
                .claim()
                .map_err(|_| EndpointPortError::Unavailable)?,
        };

        if descriptor.kind() != request.kind {
            return Err(EndpointPortError::InvariantViolation);
        }
        if descriptor.identity() != &request.expected_identity {
            return Err(EndpointPortError::IdentityMismatch);
        }
        if descriptor.generation() != request.expected_generation {
            return Err(EndpointPortError::GenerationMismatch);
        }

        let identity = descriptor.identity().clone();
        let generation = descriptor.generation();
        let cloud_hypervisor_port = descriptor.cloud_hypervisor_port();
        let descriptor_fd = descriptor.into_owned_fd();
        let mut io =
            AsyncFd::new(File::from(descriptor_fd)).map_err(|_| EndpointPortError::Unavailable)?;
        if request.kind == LocalTransportKind::CloudHypervisorVsock {
            let port = cloud_hypervisor_port.ok_or(EndpointPortError::InvariantViolation)?;
            cloud_hypervisor_handshake(&mut io, port.get()).await?;
        } else {
            await_writable(&io).await?;
        }

        let transport =
            OwnedLocalTransport::from_connected(request.kind, OwnedFd::from(io.into_inner()));
        EndpointConnection::new(
            EndpointConnectionMetadata {
                operation_id: request.operation_id,
                handle_id: request.handle_id,
                binding_id: request.binding_id,
                identity,
                generation,
                kind: request.kind,
                capabilities: request.capabilities,
                reachability: ReachabilityEvidence::ReachableOnly,
            },
            transport,
        )
    }
}

#[async_trait]
impl LocalEndpointPort for TokioLocalEndpointPort {
    async fn connect(
        &self,
        request: EndpointConnectRequest,
    ) -> Result<EndpointConnection, EndpointPortError> {
        if request.deadline.is_zero() {
            return Err(EndpointPortError::DeadlineExpired);
        }
        let deadline = request.deadline;
        tokio::time::timeout(deadline, self.connect_inner(request))
            .await
            .map_err(|_| EndpointPortError::DeadlineExpired)?
    }

    async fn inspect(
        &self,
        request: EndpointInspectRequest,
        connection: &OwnedEndpointConnection,
    ) -> Result<EndpointObservation, EndpointPortError> {
        if request.deadline.is_zero() {
            return Err(EndpointPortError::DeadlineExpired);
        }
        Ok(EndpointObservation {
            operation_id: request.operation_id,
            handle_id: request.handle_id,
            binding_id: request.binding_id,
            identity: request.expected_identity,
            generation: request.expected_generation,
            kind: request.kind,
            capabilities: request.capabilities,
            state: if connection.is_open() {
                EndpointObservationState::Connected
            } else {
                EndpointObservationState::Closed
            },
        })
    }

    async fn close(
        &self,
        request: EndpointCloseRequest,
        connection: &OwnedEndpointConnection,
    ) -> Result<EndpointCloseResult, EndpointPortError> {
        if request.deadline.is_zero() {
            return Err(EndpointPortError::DeadlineExpired);
        }
        Ok(EndpointCloseResult {
            operation_id: request.operation_id,
            handle_id: request.handle_id,
            binding_id: request.binding_id,
            identity: request.expected_identity,
            generation: request.expected_generation,
            state: connection.close(),
        })
    }
}

async fn await_writable(io: &AsyncFd<File>) -> Result<(), EndpointPortError> {
    let mut ready = io
        .writable()
        .await
        .map_err(|_| EndpointPortError::Unavailable)?;
    ready.clear_ready();
    Ok(())
}

async fn cloud_hypervisor_handshake(
    io: &mut AsyncFd<File>,
    port: u32,
) -> Result<(), EndpointPortError> {
    let connect_line = format!("CONNECT {port}\n");
    write_all(io, connect_line.as_bytes()).await?;

    let mut ack = Vec::with_capacity(MAX_CLOUD_HYPERVISOR_ACK_BYTES);
    loop {
        if ack.len() == MAX_CLOUD_HYPERVISOR_ACK_BYTES {
            return Err(EndpointPortError::BoundExceeded);
        }
        let mut byte = [0_u8; 1];
        let count = read_once(io, &mut byte).await?;
        if count == 0 {
            return Err(EndpointPortError::Unavailable);
        }
        ack.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }

    let line = std::str::from_utf8(&ack).map_err(|_| EndpointPortError::InvariantViolation)?;
    let token = line
        .strip_prefix("OK ")
        .and_then(|value| value.strip_suffix('\n'))
        .ok_or(EndpointPortError::InvariantViolation)?;
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EndpointPortError::InvariantViolation);
    }
    Ok(())
}

async fn write_all(io: &mut AsyncFd<File>, bytes: &[u8]) -> Result<(), EndpointPortError> {
    let mut written = 0;
    while written < bytes.len() {
        let mut ready = io
            .writable_mut()
            .await
            .map_err(|_| EndpointPortError::Unavailable)?;
        match ready.try_io(|inner| inner.get_mut().write(&bytes[written..])) {
            Ok(Ok(0)) => return Err(EndpointPortError::Unavailable),
            Ok(Ok(count)) => written += count,
            Ok(Err(error)) if error.kind() == ErrorKind::Interrupted => {}
            Ok(Err(_)) => return Err(EndpointPortError::Unavailable),
            Err(_) => {}
        }
    }
    Ok(())
}

async fn read_once(io: &mut AsyncFd<File>, bytes: &mut [u8]) -> Result<usize, EndpointPortError> {
    loop {
        let mut ready = io
            .readable_mut()
            .await
            .map_err(|_| EndpointPortError::Unavailable)?;
        match ready.try_io(|inner| inner.get_mut().read(bytes)) {
            Ok(Ok(count)) => return Ok(count),
            Ok(Err(error)) if error.kind() == ErrorKind::Interrupted => {}
            Ok(Err(_)) => return Err(EndpointPortError::Unavailable),
            Err(_) => {}
        }
    }
}
