//! Provider-agent service composition over an established ComponentSession.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::Duration,
};

use d2b_contracts::{
    v2_component_session::MAX_LOGICAL_MESSAGE_BYTES,
    v2_identity::{ProviderId, ProviderType},
};
use d2b_provider::{ProviderClock, ProviderRegistry, SystemProviderClock};
use d2b_provider_toolkit::GeneratedProviderServiceServer;
use d2b_session::ComponentSessionDriver;
use protobuf::{Message, MessageField};
use tokio::{sync::Semaphore, task::JoinSet};
use ttrpc::{
    r#async::{Service, TtrpcContext},
    context,
    proto::{MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_REQUEST, MessageHeader},
};

const MAX_DISPATCH_IN_FLIGHT: usize = 64;
const DEFAULT_AUDIT_CAPACITY: usize = 1_024;
const MAX_AUDIT_CAPACITY: usize = 4_096;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAgentError {
    UnregisteredAdapter,
    RegistryNotAccepting,
    RegistrationRejected,
    InvalidAuditCapacity,
    SessionClosed,
    ProtocolViolation,
}

impl fmt::Display for ProviderAgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnregisteredAdapter => "provider adapter is not registered",
            Self::RegistryNotAccepting => "provider registry is not accepting requests",
            Self::RegistrationRejected => "provider registration was rejected",
            Self::InvalidAuditCapacity => "provider audit capacity is invalid",
            Self::SessionClosed => "provider component session closed",
            Self::ProtocolViolation => "provider component session protocol violation",
        })
    }
}

impl std::error::Error for ProviderAgentError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAgentAuditOutcome {
    Completed,
    Rejected,
    Overloaded,
    ProtocolViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAgentAuditEvent {
    pub provider_type: ProviderType,
    pub outcome: ProviderAgentAuditOutcome,
}

#[derive(Debug)]
struct BoundedAudit {
    capacity: usize,
    events: Mutex<VecDeque<ProviderAgentAuditEvent>>,
}

impl BoundedAudit {
    fn new(capacity: usize) -> Result<Self, ProviderAgentError> {
        if capacity == 0 || capacity > MAX_AUDIT_CAPACITY {
            return Err(ProviderAgentError::InvalidAuditCapacity);
        }
        Ok(Self {
            capacity,
            events: Mutex::new(VecDeque::with_capacity(capacity)),
        })
    }

    fn record(&self, event: ProviderAgentAuditEvent) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn snapshot(&self) -> Vec<ProviderAgentAuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .copied()
            .collect()
    }
}

pub struct ProviderAgentProcess {
    provider_type: ProviderType,
    driver: Arc<dyn ComponentSessionDriver>,
    server: Arc<GeneratedProviderServiceServer>,
    services: Arc<HashMap<String, Service>>,
    dispatch_permits: Arc<Semaphore>,
    audit: BoundedAudit,
}

impl fmt::Debug for ProviderAgentProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAgentProcess")
            .field("provider_type", &self.provider_type)
            .finish_non_exhaustive()
    }
}

impl ProviderAgentProcess {
    pub fn from_registry(
        registry: &ProviderRegistry,
        provider_id: &ProviderId,
        driver: Arc<dyn ComponentSessionDriver>,
    ) -> Result<Arc<Self>, ProviderAgentError> {
        Self::from_registry_with(
            registry,
            provider_id,
            driver,
            Arc::new(SystemProviderClock),
            DEFAULT_AUDIT_CAPACITY,
        )
    }

    pub fn from_registry_with(
        registry: &ProviderRegistry,
        provider_id: &ProviderId,
        driver: Arc<dyn ComponentSessionDriver>,
        clock: Arc<dyn ProviderClock>,
        audit_capacity: usize,
    ) -> Result<Arc<Self>, ProviderAgentError> {
        if registry.lifecycle() != d2b_contracts::v2_provider::RegistryLifecycle::Accepting {
            return Err(ProviderAgentError::RegistryNotAccepting);
        }
        let instance = registry
            .instance(provider_id)
            .ok_or(ProviderAgentError::UnregisteredAdapter)?;
        let provider_type = instance.provider_type();
        let server = Arc::new(
            GeneratedProviderServiceServer::new(instance, driver.clone(), clock)
                .map_err(|_| ProviderAgentError::RegistrationRejected)?,
        );
        let services = Arc::new(server.generated_services());
        if services.len() != 1 {
            return Err(ProviderAgentError::RegistrationRejected);
        }
        Ok(Arc::new(Self {
            provider_type,
            driver,
            server,
            services,
            dispatch_permits: Arc::new(Semaphore::new(MAX_DISPATCH_IN_FLIGHT)),
            audit: BoundedAudit::new(audit_capacity)?,
        }))
    }

    pub fn provider_type(&self) -> ProviderType {
        self.provider_type
    }

    pub fn service_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.services.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn audit_snapshot(&self) -> Vec<ProviderAgentAuditEvent> {
        self.audit.snapshot()
    }

    pub async fn serve(self: Arc<Self>) -> Result<(), ProviderAgentError> {
        let mut tasks = JoinSet::new();
        loop {
            while tasks.try_join_next().is_some() {}
            let frame = match self.driver.receive_ttrpc().await {
                Ok(frame) => frame,
                Err(_) => {
                    self.finish_tasks(&mut tasks).await;
                    return Err(ProviderAgentError::SessionClosed);
                }
            };
            let (header, request) = match decode_request_frame(&frame) {
                Ok(request) => request,
                Err(rejection) => {
                    self.record(ProviderAgentAuditOutcome::ProtocolViolation);
                    if let Some(stream_id) = rejection.stream_id {
                        let response = error_response(stream_id, rejection.code, rejection.reason)?;
                        self.driver
                            .send_ttrpc(response)
                            .await
                            .map_err(|_| ProviderAgentError::SessionClosed)?;
                    }
                    if rejection.fatal {
                        self.finish_tasks(&mut tasks).await;
                        return Err(ProviderAgentError::ProtocolViolation);
                    }
                    continue;
                }
            };
            let permit = match self.dispatch_permits.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    self.record(ProviderAgentAuditOutcome::Overloaded);
                    let response = error_response(
                        header.stream_id,
                        ttrpc::Code::RESOURCE_EXHAUSTED,
                        "provider-agent-overloaded",
                    )?;
                    self.driver
                        .send_ttrpc(response)
                        .await
                        .map_err(|_| ProviderAgentError::SessionClosed)?;
                    continue;
                }
            };
            let process = self.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let (response, outcome) = process.dispatch(header, request).await;
                process.record(outcome);
                if let Ok(response) = response {
                    let _ = process.driver.send_ttrpc(response).await;
                }
            });
        }
    }

    async fn finish_tasks(&self, tasks: &mut JoinSet<()>) {
        if !self.server.shutdown(SHUTDOWN_TIMEOUT).await {
            tasks.abort_all();
        }
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
        tasks.abort_all();
    }

    async fn dispatch(
        &self,
        header: MessageHeader,
        request: ttrpc::Request,
    ) -> (
        Result<Vec<u8>, ProviderAgentError>,
        ProviderAgentAuditOutcome,
    ) {
        let Some(service) = self.services.get(&request.service) else {
            return (
                error_response(
                    header.stream_id,
                    ttrpc::Code::INVALID_ARGUMENT,
                    "provider-service-unavailable",
                ),
                ProviderAgentAuditOutcome::Rejected,
            );
        };
        let Some(method) = service.methods.get(&request.method) else {
            return (
                error_response(
                    header.stream_id,
                    ttrpc::Code::UNIMPLEMENTED,
                    "provider-method-unavailable",
                ),
                ProviderAgentAuditOutcome::Rejected,
            );
        };
        let timeout_nano = request.timeout_nano;
        let context = TtrpcContext {
            mh: header,
            metadata: context::from_pb(&request.metadata),
            timeout_nano,
        };
        let response = if timeout_nano == 0 {
            method.handler(context, request).await
        } else {
            match tokio::time::timeout(
                Duration::from_nanos(u64::try_from(timeout_nano).unwrap_or(0)),
                method.handler(context, request),
            )
            .await
            {
                Ok(response) => response,
                Err(_) => {
                    return (
                        error_response(
                            header.stream_id,
                            ttrpc::Code::DEADLINE_EXCEEDED,
                            "provider-deadline-expired",
                        ),
                        ProviderAgentAuditOutcome::Rejected,
                    );
                }
            }
        };
        match response {
            Ok(response) => {
                let outcome = if response
                    .status
                    .as_ref()
                    .is_none_or(|status| status.code.enum_value().ok() == Some(ttrpc::Code::OK))
                {
                    ProviderAgentAuditOutcome::Completed
                } else {
                    ProviderAgentAuditOutcome::Rejected
                };
                (encode_response(header.stream_id, response), outcome)
            }
            Err(_) => (
                error_response(
                    header.stream_id,
                    ttrpc::Code::INVALID_ARGUMENT,
                    "provider-request-rejected",
                ),
                ProviderAgentAuditOutcome::Rejected,
            ),
        }
    }

    fn record(&self, outcome: ProviderAgentAuditOutcome) {
        self.audit.record(ProviderAgentAuditEvent {
            provider_type: self.provider_type,
            outcome,
        });
    }
}

struct FrameRejection {
    stream_id: Option<u32>,
    code: ttrpc::Code,
    reason: &'static str,
    fatal: bool,
}

fn decode_request_frame(frame: &[u8]) -> Result<(MessageHeader, ttrpc::Request), FrameRejection> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FrameRejection {
            stream_id: None,
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "provider-frame-invalid",
            fatal: true,
        })?;
    let header = MessageHeader::from(header_bytes);
    if header.stream_id == 0
        || header.stream_id % 2 == 0
        || header.type_ != MESSAGE_TYPE_REQUEST
        || header.flags != 0
    {
        return Err(FrameRejection {
            stream_id: Some(header.stream_id),
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "provider-frame-invalid",
            fatal: true,
        });
    }
    let body = &frame[MESSAGE_HEADER_LENGTH..];
    if header.length as usize != body.len() || body.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
        return Err(FrameRejection {
            stream_id: Some(header.stream_id),
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "provider-frame-invalid",
            fatal: true,
        });
    }
    let request = ttrpc::Request::parse_from_bytes(body).map_err(|_| FrameRejection {
        stream_id: Some(header.stream_id),
        code: ttrpc::Code::INVALID_ARGUMENT,
        reason: "provider-request-invalid",
        fatal: false,
    })?;
    if request.timeout_nano < 0 {
        return Err(FrameRejection {
            stream_id: Some(header.stream_id),
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "provider-deadline-invalid",
            fatal: false,
        });
    }
    Ok((header, request))
}

fn error_response(
    stream_id: u32,
    code: ttrpc::Code,
    reason: &'static str,
) -> Result<Vec<u8>, ProviderAgentError> {
    encode_response(
        stream_id,
        ttrpc::Response {
            status: MessageField::some(ttrpc::get_status(code, reason)),
            ..Default::default()
        },
    )
}

fn encode_response(
    stream_id: u32,
    response: ttrpc::Response,
) -> Result<Vec<u8>, ProviderAgentError> {
    let body = response
        .write_to_bytes()
        .map_err(|_| ProviderAgentError::ProtocolViolation)?;
    if body.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
        return Err(ProviderAgentError::ProtocolViolation);
    }
    let length = u32::try_from(body.len()).map_err(|_| ProviderAgentError::ProtocolViolation)?;
    let mut frame = Vec::from(MessageHeader::new_response(stream_id, length));
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Run a statically composed provider-agent process to ComponentSession close.
pub fn run_registered(process: Arc<ProviderAgentProcess>) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return ExitCode::FAILURE,
    };
    match runtime.block_on(process.serve()) {
        Err(ProviderAgentError::SessionClosed) => ExitCode::SUCCESS,
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// The standalone executable has no ambient adapter or serialized dynamic
/// loading path. Composition owners call [`ProviderAgentProcess::from_registry`]
/// and [`run_registered`] with an exact typed registry and established
/// ComponentSession.
pub fn run() -> ExitCode {
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_entrypoint_rejects_missing_registration() {
        assert_eq!(run(), ExitCode::FAILURE);
    }

    #[test]
    fn audit_capacity_is_closed_and_bounded() {
        assert!(matches!(
            BoundedAudit::new(0),
            Err(ProviderAgentError::InvalidAuditCapacity)
        ));
        let audit = BoundedAudit::new(2).unwrap();
        let event = ProviderAgentAuditEvent {
            provider_type: ProviderType::Runtime,
            outcome: ProviderAgentAuditOutcome::Completed,
        };
        audit.record(event);
        audit.record(event);
        audit.record(ProviderAgentAuditEvent {
            outcome: ProviderAgentAuditOutcome::Rejected,
            ..event
        });
        assert_eq!(audit.snapshot().len(), 2);
        assert_eq!(
            audit.snapshot()[1].outcome,
            ProviderAgentAuditOutcome::Rejected
        );
    }
}
