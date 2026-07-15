use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    v2_component_session::{
        AttachmentPurpose, CorrelationId, IdempotencyKey, MAX_ACTIVE_NAMED_STREAMS,
        MAX_REQUEST_LIFETIME_MS, RequestId, ServicePackage, TraceId,
    },
    v2_services::{
        StrictWireMessage,
        common::{
            self, ErrorKind as WireErrorKind, Outcome, RetryClass as WireRetryClass, ServiceRequest,
        },
        decode_strict, encode_strict,
    },
};
use protobuf::MessageField;
use tokio::sync::Notify;

use crate::session::StreamDispatcher;
use crate::{
    ClientError, ComponentSessionConnector, MethodHandle, NamedStream, OwnedAttachment,
    RemoteErrorKind, ResolvedTarget, RetryClass, ServiceHandle, ServiceKind, ServiceOwner,
    SessionCall, SessionFailure, SessionReply, TargetInput, TargetResolver, TransportPacket,
    TransportSelection,
};

pub trait WallClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl WallClock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[derive(Clone)]
pub struct MetadataInput {
    request_id: RequestId,
    correlation_id: Option<CorrelationId>,
    trace_id: Option<TraceId>,
    idempotency_key: Option<IdempotencyKey>,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl fmt::Debug for MetadataInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataInput")
            .field("has_correlation", &self.correlation_id.is_some())
            .field("has_trace", &self.trace_id.is_some())
            .field("has_idempotency", &self.idempotency_key.is_some())
            .field("issued_at_unix_ms", &self.issued_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

impl MetadataInput {
    pub fn new(
        request_id: [u8; 16],
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, ClientError> {
        let request_id =
            RequestId::new(request_id.to_vec()).map_err(|_| ClientError::InvalidMetadata)?;
        let input = Self {
            request_id,
            correlation_id: None,
            trace_id: None,
            idempotency_key: None,
            issued_at_unix_ms,
            expires_at_unix_ms,
        };
        input.validate_lifetime()?;
        Ok(input)
    }

    pub fn with_correlation(mut self, value: impl Into<String>) -> Result<Self, ClientError> {
        let value = value.into();
        if !value.is_ascii() {
            return Err(ClientError::InvalidMetadata);
        }
        self.correlation_id =
            Some(CorrelationId::new(value.into_bytes()).map_err(|_| ClientError::InvalidMetadata)?);
        Ok(self)
    }

    pub fn with_trace(mut self, value: [u8; 16]) -> Result<Self, ClientError> {
        self.trace_id =
            Some(TraceId::new(value.to_vec()).map_err(|_| ClientError::InvalidMetadata)?);
        Ok(self)
    }

    pub fn with_idempotency(mut self, value: Vec<u8>) -> Result<Self, ClientError> {
        self.idempotency_key =
            Some(IdempotencyKey::new(value).map_err(|_| ClientError::InvalidMetadata)?);
        Ok(self)
    }

    fn validate_lifetime(&self) -> Result<(), ClientError> {
        let lifetime = self
            .expires_at_unix_ms
            .checked_sub(self.issued_at_unix_ms)
            .ok_or(ClientError::InvalidMetadata)?;
        if self.issued_at_unix_ms == 0 || lifetime == 0 || lifetime > MAX_REQUEST_LIFETIME_MS {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(())
    }

    fn protobuf(&self, trusted_generation: u64) -> Result<common::RequestMetadata, ClientError> {
        if trusted_generation == 0 {
            return Err(ClientError::ContractViolation);
        }
        let correlation_id = self
            .correlation_id
            .as_ref()
            .map(|value| {
                std::str::from_utf8(value.as_bytes())
                    .map(str::to_owned)
                    .map_err(|_| ClientError::InvalidMetadata)
            })
            .transpose()?
            .unwrap_or_default();
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = self.request_id.as_bytes().to_vec();
        metadata.correlation_id = correlation_id;
        metadata.trace_id = self
            .trace_id
            .as_ref()
            .map(|value| value.as_bytes().to_vec())
            .unwrap_or_default();
        metadata.idempotency_key = self
            .idempotency_key
            .as_ref()
            .map(|value| value.as_bytes().to_vec())
            .unwrap_or_default();
        metadata.issued_at_unix_ms = self.issued_at_unix_ms;
        metadata.expires_at_unix_ms = self.expires_at_unix_ms;
        metadata.session_generation = trusted_generation;
        Ok(metadata)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u8,
}

impl RetryPolicy {
    pub fn new(max_attempts: u8) -> Result<Self, ClientError> {
        if !(1..=8).contains(&max_attempts) {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(Self { max_attempts })
    }

    pub const fn no_retry() -> Self {
        Self { max_attempts: 1 }
    }

    pub const fn max_attempts(self) -> u8 {
        self.max_attempts
    }
}

#[derive(Debug, Clone)]
pub struct CallOptions {
    pub metadata: MetadataInput,
    pub retry: RetryPolicy,
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.state.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

pub struct Client<R, C, W = SystemClock> {
    resolver: R,
    connector: Arc<C>,
    clock: Arc<W>,
}

impl<R, C> Client<R, C, SystemClock> {
    pub fn new(resolver: R, connector: C) -> Self {
        Self {
            resolver,
            connector: Arc::new(connector),
            clock: Arc::new(SystemClock),
        }
    }
}

impl<R, C, W> Client<R, C, W> {
    pub fn with_clock(resolver: R, connector: C, clock: W) -> Self {
        Self {
            resolver,
            connector: Arc::new(connector),
            clock: Arc::new(clock),
        }
    }
}

impl<R, C, W> Client<R, C, W>
where
    R: TargetResolver,
    C: ComponentSessionConnector,
    W: WallClock + 'static,
{
    pub async fn connect(
        &self,
        target: TargetInput,
        service: ServiceKind,
        selection: TransportSelection,
    ) -> Result<ConnectedClient, ClientError> {
        let resolved = self.resolver.resolve(&target, service, selection)?;
        let connected = self.connector.connect(&resolved, service).await?;
        let generation = connected.driver.generation();
        if generation == 0 {
            return Err(ClientError::ContractViolation);
        }
        let generated = ttrpc::r#async::Client::new(connected.ttrpc_socket);
        let stream_dispatcher = StreamDispatcher::new(Arc::clone(&connected.driver));
        Ok(ConnectedClient {
            target: resolved,
            service: ServiceHandle::new(service, generated),
            driver: connected.driver,
            generation,
            clock: self.clock.clone(),
            active_streams: Arc::new(AtomicU16::new(0)),
            stream_dispatcher,
        })
    }
}

pub struct ConnectedClient {
    target: ResolvedTarget,
    service: ServiceHandle,
    driver: crate::SharedDriver,
    generation: u64,
    clock: Arc<dyn WallClock>,
    active_streams: Arc<AtomicU16>,
    stream_dispatcher: Arc<StreamDispatcher>,
}

impl fmt::Debug for ConnectedClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectedClient")
            .field("target", &self.target)
            .field("service", &self.service.kind())
            .field("generation", &self.generation)
            .finish()
    }
}

pub struct Response {
    pub message: common::ServiceResponse,
    pub attachments: Vec<OwnedAttachment>,
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("attachment_count", &self.attachments.len())
            .field("has_stream", &!self.message.stream_id.is_empty())
            .finish()
    }
}

impl ConnectedClient {
    pub fn service(&self) -> &ServiceHandle {
        &self.service
    }

    pub async fn invoke(
        &self,
        method: MethodHandle,
        request: ServiceRequest,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<Response, ClientError> {
        self.invoke_with_attachments(method, request, Vec::new(), options, cancellation)
            .await
    }

    pub async fn invoke_with_attachments(
        &self,
        method: MethodHandle,
        mut request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<Response, ClientError> {
        if method.service() != self.service.kind() {
            return Err(ClientError::InvalidMethod);
        }
        let spec = method.spec();
        let has_idempotency = options.metadata.idempotency_key.is_some();
        if spec.requires_idempotency && !has_idempotency {
            return Err(ClientError::IdempotencyRequired);
        }
        options.metadata.validate_lifetime()?;
        request.scope = MessageField::some(scope_for(self.target.owner()));
        request.metadata = MessageField::some(options.metadata.protobuf(self.generation)?);
        request.attachment_indexes = (0..attachments.len())
            .map(|index| u32::try_from(index).map_err(|_| ClientError::AttachmentMismatch))
            .collect::<Result<Vec<_>, _>>()?;
        request
            .validate_wire(spec.requires_idempotency)
            .map_err(ClientError::ServiceContract)?;
        validate_outbound_attachments(
            &attachments,
            method,
            &options.metadata.request_id,
            self.generation,
        )?;
        let request_bytes = encode_strict(&request, spec.requires_idempotency)
            .map_err(ClientError::ServiceContract)?;

        let now = self.clock.now_unix_ms();
        let wall_remaining = options
            .metadata
            .expires_at_unix_ms
            .checked_sub(now)
            .ok_or(ClientError::DeadlineExpired)?
            .min(u64::from(spec.max_lifetime_ms));
        if wall_remaining == 0 {
            return Err(ClientError::DeadlineExpired);
        }
        let monotonic_deadline = Instant::now()
            .checked_add(Duration::from_millis(wall_remaining))
            .ok_or(ClientError::InvalidMetadata)?;

        let mut attempt = 0_u8;
        let has_attachments = !attachments.is_empty();
        let mut attachments = Some(attachments);
        loop {
            attempt = attempt.saturating_add(1);
            if cancellation.is_cancelled() {
                self.cancel_request(&options.metadata).await;
                return Err(ClientError::Cancelled);
            }
            let relative_timeout_nanos =
                self.relative_timeout(monotonic_deadline, options.metadata.expires_at_unix_ms)?;
            let call = SessionCall {
                method,
                packet: TransportPacket::with_attachments(
                    request_bytes.clone(),
                    attachments.take().unwrap_or_default(),
                ),
                relative_timeout_nanos,
            };
            let (request_bytes, request_attachments) = call.packet.into_parts();
            if !request_attachments.is_empty()
                && self
                    .driver
                    .send_attachments(request_attachments)
                    .await
                    .is_err()
            {
                return Err(ClientError::TransportFailed);
            }
            let result = tokio::select! {
                result = async {
                    let response_bytes = self
                        .service
                        .invoke(method, request_bytes, call.relative_timeout_nanos)
                        .await
                        .map_err(|_| SessionFailure::Retryable)?;
                    let attachment_count = decode_strict::<common::ServiceResponse>(
                        &response_bytes,
                        false,
                    )
                    .map_err(|_| SessionFailure::Protocol)?
                    .attachment_indexes
                    .len();
                    let attachments = if attachment_count == 0 {
                        Vec::new()
                    } else {
                        self.driver
                            .receive_attachments()
                            .await
                            .map_err(|_| SessionFailure::Protocol)?
                    };
                    Ok::<_, SessionFailure>(SessionReply {
                        packet: TransportPacket::with_attachments(
                            response_bytes,
                            attachments,
                        ),
                    })
                } => result,
                () = cancellation.cancelled() => {
                    self.cancel_request(&options.metadata).await;
                    return Err(ClientError::Cancelled);
                }
            };
            match result {
                Ok(reply) => match validate_reply(
                    reply,
                    method,
                    &options.metadata.request_id,
                    self.generation,
                ) {
                    Ok(response) => return Ok(response),
                    Err(
                        error @ ClientError::Remote {
                            retry: RetryClass::Safe,
                            ..
                        },
                    ) if !has_attachments
                        && can_retry(attempt, options.retry, spec.mutating, has_idempotency) =>
                    {
                        let _ = error;
                        tokio::task::yield_now().await;
                    }
                    Err(error) => return Err(error),
                },
                Err(failure)
                    if retryable_failure(failure, spec.mutating, has_idempotency)
                        && !has_attachments
                        && can_retry(attempt, options.retry, spec.mutating, has_idempotency) =>
                {
                    tokio::task::yield_now().await;
                }
                Err(SessionFailure::Ambiguous) if spec.mutating => {
                    return Err(ClientError::AmbiguousMutation);
                }
                Err(failure)
                    if retryable_failure(failure, spec.mutating, has_idempotency)
                        && attempt >= options.retry.max_attempts() =>
                {
                    return Err(ClientError::RetryLimitExceeded);
                }
                Err(failure) => return Err(crate::session::map_session_failure(failure)),
            }
        }
    }

    fn relative_timeout(
        &self,
        monotonic_deadline: Instant,
        absolute_expiry_unix_ms: u64,
    ) -> Result<u64, ClientError> {
        let wall_ms = absolute_expiry_unix_ms
            .checked_sub(self.clock.now_unix_ms())
            .ok_or(ClientError::DeadlineExpired)?;
        let monotonic = monotonic_deadline.saturating_duration_since(Instant::now());
        let relative = monotonic.min(Duration::from_millis(wall_ms));
        if relative.is_zero() {
            return Err(ClientError::DeadlineExpired);
        }
        Ok(relative.as_nanos().try_into().unwrap_or(u64::MAX))
    }

    async fn cancel_request(&self, metadata: &MetadataInput) {
        let _ = self
            .driver
            .cancel(self.generation, metadata.request_id.clone())
            .await;
    }

    pub async fn named_stream(&self, response: &Response) -> Result<NamedStream, ClientError> {
        if response.message.stream_id.is_empty() {
            return Err(ClientError::ContractViolation);
        }
        let channel = response
            .message
            .stream_id
            .strip_prefix("stream-")
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or(ClientError::ContractViolation)?;
        let id = d2b_session::StreamId::new(channel).map_err(|_| ClientError::ContractViolation)?;
        self.reserve_stream()?;
        let stream = match NamedStream::new(
            Arc::clone(&self.driver),
            &self.stream_dispatcher,
            id,
            Arc::clone(&self.active_streams),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                self.active_streams.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        if self
            .driver
            .open_named_stream(
                id,
                d2b_contracts::v2_component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
                d2b_contracts::v2_component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
            )
            .await
            .is_err()
        {
            drop(stream);
            return Err(ClientError::TransportFailed);
        }
        Ok(stream)
    }

    fn reserve_stream(&self) -> Result<(), ClientError> {
        self.active_streams
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_NAMED_STREAMS).then_some(active + 1)
            })
            .map(|_| ())
            .map_err(|_| ClientError::StreamLimitExceeded)
    }
}

fn can_retry(attempt: u8, policy: RetryPolicy, mutating: bool, has_idempotency: bool) -> bool {
    attempt < policy.max_attempts() && (!mutating || has_idempotency)
}

fn retryable_failure(failure: SessionFailure, mutating: bool, has_idempotency: bool) -> bool {
    match failure {
        SessionFailure::BeforeDispatch => !mutating || has_idempotency,
        SessionFailure::Retryable | SessionFailure::Disconnected => !mutating || has_idempotency,
        SessionFailure::Ambiguous => !mutating,
        SessionFailure::Deadline | SessionFailure::Cancelled | SessionFailure::Protocol => false,
    }
}

fn scope_for(owner: &ServiceOwner) -> common::IdentityScope {
    let mut scope = common::IdentityScope::new();
    match owner {
        ServiceOwner::LocalRoot(realm) | ServiceOwner::Realm(realm) => {
            scope.realm_id = realm.as_str().to_owned();
        }
        ServiceOwner::Workload { realm, workload } => {
            scope.realm_id = realm.as_str().to_owned();
            scope.workload_id = workload.as_str().to_owned();
        }
        ServiceOwner::Provider { realm, provider } => {
            scope.realm_id = realm.as_str().to_owned();
            scope.provider_id = provider.as_str().to_owned();
        }
    }
    scope
}

fn validate_outbound_attachments(
    attachments: &[OwnedAttachment],
    method: MethodHandle,
    request_id: &RequestId,
    generation: u64,
) -> Result<(), ClientError> {
    for (index, attachment) in attachments.iter().enumerate() {
        let expected_index = u16::try_from(index).map_err(|_| ClientError::AttachmentMismatch)?;
        let Some(descriptor) = attachment.descriptor() else {
            return Err(ClientError::AttachmentMismatch);
        };
        if descriptor.validate(expected_index).is_err()
            || descriptor.purpose != AttachmentPurpose::RequestInput
            || descriptor.service != service_package(method.service())
            || descriptor.method_id != method_id(method)
            || &descriptor.request_id != request_id
            || descriptor.reconnect_generation != generation
        {
            return Err(ClientError::AttachmentMismatch);
        }
    }
    Ok(())
}

fn validate_reply(
    reply: SessionReply,
    method: MethodHandle,
    request_id: &RequestId,
    generation: u64,
) -> Result<Response, ClientError> {
    let (bytes, attachments) = reply.packet.into_parts();
    let response: common::ServiceResponse =
        decode_strict(&bytes, false).map_err(ClientError::ServiceContract)?;
    response
        .validate_wire(false)
        .map_err(ClientError::ServiceContract)?;
    if response.attachment_indexes.len() != attachments.len()
        || response
            .attachment_indexes
            .iter()
            .zip(&attachments)
            .enumerate()
            .any(|(position, (expected, actual))| {
                let Ok(index) = u16::try_from(position) else {
                    return true;
                };
                let Some(descriptor) = actual.descriptor() else {
                    return true;
                };
                *expected != u32::from(index)
                    || descriptor.validate(index).is_err()
                    || descriptor.purpose != AttachmentPurpose::ResponseOutput
                    || descriptor.service != service_package(method.service())
                    || descriptor.method_id != method_id(method)
                    || &descriptor.request_id != request_id
                    || descriptor.packet_sequence == 0
                    || descriptor.reconnect_generation != generation
            })
    {
        return Err(ClientError::AttachmentMismatch);
    }
    let outcome = response
        .outcome
        .enum_value()
        .map_err(|_| ClientError::ContractViolation)?;
    if matches!(
        outcome,
        Outcome::OUTCOME_DENIED | Outcome::OUTCOME_CANCELLED | Outcome::OUTCOME_FAILED
    ) {
        let error = response
            .error
            .as_ref()
            .ok_or(ClientError::ContractViolation)?;
        return Err(ClientError::Remote {
            kind: map_remote_kind(
                error
                    .kind
                    .enum_value()
                    .map_err(|_| ClientError::ContractViolation)?,
            )?,
            retry: map_retry(
                error
                    .retry
                    .enum_value()
                    .map_err(|_| ClientError::ContractViolation)?,
            )?,
        });
    }
    Ok(Response {
        message: response,
        attachments,
    })
}

fn method_id(method: MethodHandle) -> u32 {
    let service = method.service().spec();
    method.spec().method_id(service.package, service.service)
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

fn map_remote_kind(kind: WireErrorKind) -> Result<RemoteErrorKind, ClientError> {
    Ok(match kind {
        WireErrorKind::ERROR_KIND_INVALID_REQUEST => RemoteErrorKind::InvalidRequest,
        WireErrorKind::ERROR_KIND_UNAUTHENTICATED => RemoteErrorKind::Unauthorized,
        WireErrorKind::ERROR_KIND_UNAUTHORIZED | WireErrorKind::ERROR_KIND_CAPABILITY_DENIED => {
            RemoteErrorKind::Forbidden
        }
        WireErrorKind::ERROR_KIND_NOT_FOUND => RemoteErrorKind::NotFound,
        WireErrorKind::ERROR_KIND_CONFLICT => RemoteErrorKind::Conflict,
        WireErrorKind::ERROR_KIND_DEADLINE_EXCEEDED => RemoteErrorKind::DeadlineExceeded,
        WireErrorKind::ERROR_KIND_CANCELLED => RemoteErrorKind::Cancelled,
        WireErrorKind::ERROR_KIND_GENERATION_MISMATCH => RemoteErrorKind::FailedPrecondition,
        WireErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED => RemoteErrorKind::ResourceExhausted,
        WireErrorKind::ERROR_KIND_UNAVAILABLE => RemoteErrorKind::Unavailable,
        WireErrorKind::ERROR_KIND_INVARIANT_VIOLATION | WireErrorKind::ERROR_KIND_INTERNAL => {
            RemoteErrorKind::Internal
        }
        WireErrorKind::ERROR_KIND_UNSPECIFIED => return Err(ClientError::ContractViolation),
    })
}

fn map_retry(retry: WireRetryClass) -> Result<RetryClass, ClientError> {
    Ok(match retry {
        WireRetryClass::RETRY_CLASS_NEVER | WireRetryClass::RETRY_CLASS_AFTER_INTERACTION => {
            RetryClass::Never
        }
        WireRetryClass::RETRY_CLASS_SAME_OPERATION => RetryClass::Safe,
        WireRetryClass::RETRY_CLASS_AFTER_OBSERVATION => RetryClass::Observe,
        WireRetryClass::RETRY_CLASS_UNSPECIFIED => return Err(ClientError::ContractViolation),
    })
}
