use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::v2_services::{
    StrictWireMessage, common,
    guest::{
        GuestCancelExecRequest, GuestCancelExecResponse, GuestInspectExecRequest,
        GuestInspectExecResponse, GuestOpenExecRetainedLogRequest,
    },
    guest_contract::{
        validate_guest_cancel_response_for_request, validate_guest_inspect_response_for_request,
        validate_guest_open_exec_retained_log_response_for_request,
    },
    guest_ttrpc::GuestService,
    terminal,
};
use protobuf::{EnumOrUnknown, MessageField};

use super::{DaemonMethod, DaemonOperationHandler, DaemonPeerRole, DaemonServiceV2};
use crate::{
    daemon_terminal::{PreparedTerminal, TerminalBinding, TerminalFailure, TerminalOpenResult},
    guest_terminal::{GuestProxySession, GuestTerminalConnector},
};

const GUEST_PROXY_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct DaemonGuestProxy<H> {
    daemon: Arc<DaemonServiceV2<H>>,
    connector: Arc<dyn GuestTerminalConnector>,
}

impl<H> DaemonGuestProxy<H> {
    pub(crate) fn new(
        daemon: Arc<DaemonServiceV2<H>>,
        connector: Arc<dyn GuestTerminalConnector>,
    ) -> Arc<Self> {
        Arc::new(Self { daemon, connector })
    }

    fn require_admin(&self) -> ttrpc::Result<()> {
        if self.daemon.peer_role() == DaemonPeerRole::Admin {
            Ok(())
        } else {
            Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "guest-proxy-admission-denied",
            ))
        }
    }

    fn acquire_permit(&self) -> ttrpc::Result<tokio::sync::OwnedSemaphorePermit> {
        self.daemon.in_flight().try_acquire_owned().map_err(|_| {
            rpc_error(
                ttrpc::Code::RESOURCE_EXHAUSTED,
                "guest-proxy-resource-exhausted",
            )
        })
    }

    async fn connect(
        &self,
        context: &d2b_contracts::v2_services::guest::GuestOperationContext,
    ) -> Result<Arc<dyn GuestProxySession>, TerminalFailure> {
        let scope = context.scope.as_ref().ok_or(TerminalFailure::Protocol)?;
        if scope.workload_id.is_empty()
            || scope.realm_id.is_empty()
            || !scope.provider_id.is_empty()
            || !scope.role_id.is_empty()
        {
            return Err(TerminalFailure::Protocol);
        }
        self.connector.connect_proxy(&scope.workload_id).await
    }
}

#[async_trait]
impl<H: DaemonOperationHandler + 'static> GuestService for DaemonGuestProxy<H> {
    async fn cancel_exec(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        request: GuestCancelExecRequest,
    ) -> ttrpc::Result<GuestCancelExecResponse> {
        let _permit = self.acquire_permit()?;
        self.require_admin()?;
        request.validate_wire(true).map_err(|_| invalid_request())?;
        let context = request.context.as_ref().ok_or_else(invalid_request)?;
        let admitted = self
            .daemon
            .admit(ttrpc_context, DaemonMethod::Exec, context.metadata.as_ref())
            .await?;
        let session = connect_admitted(self, context, &admitted).await?;
        let mut upstream = request.clone();
        rebind_context(
            upstream.context.as_mut().ok_or_else(invalid_request)?,
            session.generation(),
        )?;
        let public_request_id = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request)?
            .request_id
            .clone();
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => {
                session.cancel_request(&public_request_id).await;
                Err(cancelled())
            }
            response = tokio::time::timeout(
                admitted.context.remaining.min(GUEST_PROXY_TIMEOUT),
                session.cancel_exec(upstream.clone(), admitted.context.remaining.min(GUEST_PROXY_TIMEOUT)),
            ) => match response {
                Ok(Ok(mut response)) => {
                    response.session_generation = self.daemon.generation();
                    validate_guest_cancel_response_for_request(&request, &response)
                        .map_err(|_| response_error())?;
                    Ok(response)
                }
                Ok(Err(error)) => Err(proxy_error(error)),
                Err(_) => {
                    session.cancel_request(&public_request_id).await;
                    Err(deadline_exceeded())
                }
            }
        };
        admitted.finish(result).await
    }

    async fn inspect_exec(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        request: GuestInspectExecRequest,
    ) -> ttrpc::Result<GuestInspectExecResponse> {
        let _permit = self.acquire_permit()?;
        self.require_admin()?;
        request
            .validate_wire(false)
            .map_err(|_| invalid_request())?;
        let context = request.context.as_ref().ok_or_else(invalid_request)?;
        let admitted = self
            .daemon
            .admit(
                ttrpc_context,
                DaemonMethod::Inspect,
                context.metadata.as_ref(),
            )
            .await?;
        let session = connect_admitted(self, context, &admitted).await?;
        let mut upstream = request.clone();
        rebind_context(
            upstream.context.as_mut().ok_or_else(invalid_request)?,
            session.generation(),
        )?;
        let public_request_id = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request)?
            .request_id
            .clone();
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => {
                session.cancel_request(&public_request_id).await;
                Err(cancelled())
            }
            response = tokio::time::timeout(
                admitted.context.remaining.min(GUEST_PROXY_TIMEOUT),
                session.inspect_exec(upstream.clone(), admitted.context.remaining.min(GUEST_PROXY_TIMEOUT)),
            ) => match response {
                Ok(Ok(mut response)) => {
                    response.session_generation = self.daemon.generation();
                    validate_guest_inspect_response_for_request(&request, &response)
                        .map_err(|_| response_error())?;
                    Ok(response)
                }
                Ok(Err(error)) => Err(proxy_error(error)),
                Err(_) => {
                    session.cancel_request(&public_request_id).await;
                    Err(deadline_exceeded())
                }
            }
        };
        admitted.finish(result).await
    }

    async fn open_exec_retained_log(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        request: GuestOpenExecRetainedLogRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        let _permit = self.acquire_permit()?;
        self.require_admin()?;
        request.validate_wire(true).map_err(|_| invalid_request())?;
        let context = request.context.as_ref().ok_or_else(invalid_request)?;
        let admitted = self
            .daemon
            .admit(ttrpc_context, DaemonMethod::Exec, context.metadata.as_ref())
            .await?;
        let session = connect_admitted(self, context, &admitted).await?;
        let mut upstream_request = request.clone();
        rebind_context(
            upstream_request
                .context
                .as_mut()
                .ok_or_else(invalid_request)?,
            session.generation(),
        )?;
        let timeout = admitted.context.remaining.min(GUEST_PROXY_TIMEOUT);
        let public_request_id = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request)?
            .request_id
            .clone();
        let upstream_response = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => {
                session.cancel_request(&public_request_id).await;
                return admitted.finish(Err(cancelled())).await;
            }
            response = tokio::time::timeout(
                timeout,
                session.prepare_retained_log(upstream_request.clone(), timeout),
            ) => match response {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => return admitted.finish(Err(proxy_error(error))).await,
                Err(_) => {
                    session.cancel_request(&public_request_id).await;
                    return admitted.finish(Err(deadline_exceeded())).await;
                }
            }
        };
        if upstream_response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED) {
            let mut response = upstream_response;
            response.session_generation = self.daemon.generation();
            validate_guest_open_exec_retained_log_response_for_request(&request, &response)
                .map_err(|_| response_error())?;
            return admitted.finish(Ok(response)).await;
        }
        let range = upstream_response
            .retained_log
            .as_ref()
            .cloned()
            .ok_or_else(response_error)?;
        let prepared: Arc<dyn PreparedTerminal> = Arc::new(RetainedLogPrepared {
            session: Arc::clone(&session),
            request: upstream_request,
            response: upstream_response,
            abandoned: AtomicBool::new(false),
        });
        let request_id: [u8; 16] = public_request_id
            .as_slice()
            .try_into()
            .map_err(|_| invalid_request())?;
        let binding = TerminalBinding {
            session_generation: self.daemon.generation(),
            request_id,
            operation_id: context.operation_id.clone(),
            resource_handle: request.resource_handle.clone(),
            peer_principal: format!("local-{}-admin", self.daemon.peer_uid()),
            peer_uid: self.daemon.peer_uid(),
            kind: terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG,
            retained_log: Some(range.clone()),
        };
        let stream_id = match self
            .daemon
            .terminals()
            .reserve(
                binding,
                Arc::clone(&prepared),
                admitted.context.cancellation.clone(),
            )
            .await
        {
            Ok(stream_id) => stream_id,
            Err(error) => {
                prepared.abandoned().await;
                return admitted.finish(Err(proxy_error(error))).await;
            }
        };
        let response = terminal::TerminalOpenResponse {
            outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
            operation_id: context.operation_id.clone(),
            stream_id,
            session_generation: self.daemon.generation(),
            request_id: public_request_id.clone(),
            resource_handle: request.resource_handle.clone(),
            retained_log: MessageField::some(range),
            ..Default::default()
        };
        if validate_guest_open_exec_retained_log_response_for_request(&request, &response).is_err()
        {
            let _ = self
                .daemon
                .terminals()
                .cancel(self.daemon.generation(), &public_request_id);
            prepared.abandoned().await;
            return admitted.finish(Err(response_error())).await;
        }
        admitted.finish(Ok(response)).await
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        self.require_admin()?;
        self.daemon.cancel_request(&request)
    }
}

async fn connect_admitted<H: DaemonOperationHandler>(
    proxy: &DaemonGuestProxy<H>,
    context: &d2b_contracts::v2_services::guest::GuestOperationContext,
    admitted: &super::AdmittedCall,
) -> ttrpc::Result<Arc<dyn GuestProxySession>> {
    tokio::select! {
        biased;
        () = admitted.context.cancellation.cancelled() => Err(cancelled()),
        result = tokio::time::timeout(
            admitted.context.remaining.min(GUEST_PROXY_TIMEOUT),
            proxy.connect(context),
        ) => match result {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(error)) => Err(proxy_error(error)),
            Err(_) => Err(deadline_exceeded()),
        }
    }
}

fn rebind_context(
    context: &mut d2b_contracts::v2_services::guest::GuestOperationContext,
    generation: u64,
) -> ttrpc::Result<()> {
    context
        .metadata
        .as_mut()
        .ok_or_else(invalid_request)?
        .session_generation = generation;
    Ok(())
}

struct RetainedLogPrepared {
    session: Arc<dyn GuestProxySession>,
    request: GuestOpenExecRetainedLogRequest,
    response: terminal::TerminalOpenResponse,
    abandoned: AtomicBool,
}

#[async_trait]
impl PreparedTerminal for RetainedLogPrepared {
    async fn open(
        &self,
        _: &TerminalBinding,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        self.session
            .open_prepared_retained_log(&self.request, self.response.clone(), selection)
            .await
    }

    async fn abandoned(&self) {
        if !self.abandoned.swap(true, Ordering::AcqRel) {
            self.session
                .abandon_retained_log(&self.response.stream_id)
                .await;
        }
    }
}

fn proxy_error(error: TerminalFailure) -> ttrpc::Error {
    match error {
        TerminalFailure::Unauthorized => rpc_error(
            ttrpc::Code::PERMISSION_DENIED,
            "guest-proxy-admission-denied",
        ),
        TerminalFailure::ResourceExhausted => rpc_error(
            ttrpc::Code::RESOURCE_EXHAUSTED,
            "guest-proxy-resource-exhausted",
        ),
        TerminalFailure::GenerationMismatch
        | TerminalFailure::InvalidSelection
        | TerminalFailure::Protocol => invalid_request(),
        TerminalFailure::NotFound
        | TerminalFailure::Conflict
        | TerminalFailure::Unavailable
        | TerminalFailure::Internal => {
            rpc_error(ttrpc::Code::FAILED_PRECONDITION, "guest-proxy-unavailable")
        }
    }
}

fn invalid_request() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INVALID_ARGUMENT, "guest-proxy-request-invalid")
}

fn cancelled() -> ttrpc::Error {
    rpc_error(ttrpc::Code::CANCELLED, "guest-proxy-request-cancelled")
}

fn deadline_exceeded() -> ttrpc::Error {
    rpc_error(
        ttrpc::Code::DEADLINE_EXCEEDED,
        "guest-proxy-deadline-exceeded",
    )
}

fn response_error() -> ttrpc::Error {
    rpc_error(
        ttrpc::Code::INTERNAL,
        "guest-proxy-response-contract-invalid",
    )
}

fn rpc_error(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use d2b_contracts::v2_component_session::{
        CloseReason, GuestSessionCredentialV1, Remediation, RequestId, SessionErrorCode,
    };
    use d2b_contracts::v2_services::{
        daemon,
        guest::{GuestExecCancelReason, GuestExecCancellationOutcome, GuestExecState},
        guest_ttrpc::create_guest_service,
    };
    use d2b_session::{
        Cancellation, ComponentSessionDriver, OwnedAttachment, RequestRegistry, SessionError,
        SessionEvent, StreamEvent, StreamId,
    };
    use tokio::sync::Notify;

    use super::*;

    const PUBLIC_GENERATION: u64 = 7;
    const GUEST_GENERATION: u64 = 19;

    struct PublicDriver {
        registry: Mutex<RequestRegistry>,
        opened: AtomicUsize,
        reset: AtomicUsize,
    }

    impl PublicDriver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                registry: Mutex::new(RequestRegistry::new(PUBLIC_GENERATION).unwrap()),
                opened: AtomicUsize::new(0),
                reset: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for PublicDriver {
        fn generation(&self) -> u64 {
            PUBLIC_GENERATION
        }

        async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            self.registry.lock().unwrap().register(request_id)
        }

        async fn complete_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.registry.lock().unwrap().complete(&request_id))
        }

        async fn remove_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.registry.lock().unwrap().remove(&request_id))
        }

        async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
            self.opened.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            self.reset.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn drive_keepalive(&self, _: std::time::Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
            Ok(())
        }
    }

    struct NoopHandler;

    #[async_trait]
    impl DaemonOperationHandler for NoopHandler {
        async fn handle_service(
            &self,
            _: super::super::DaemonAdapter,
            _: DaemonMethod,
            _: d2b_contracts::v2_services::common::ServiceRequest,
            _: &super::super::DaemonCallContext,
        ) -> Result<
            d2b_contracts::v2_services::common::ServiceResponse,
            super::super::DaemonServiceFailure,
        > {
            Err(super::super::DaemonServiceFailure::Backend)
        }

        async fn list_realms(
            &self,
            _: d2b_contracts::v2_services::common::ServiceRequest,
            _: &super::super::DaemonCallContext,
        ) -> Result<daemon::ListRealmsResponse, super::super::DaemonServiceFailure> {
            Err(super::super::DaemonServiceFailure::Backend)
        }

        async fn list_workloads(
            &self,
            _: d2b_contracts::v2_services::common::ServiceRequest,
            _: &super::super::DaemonCallContext,
        ) -> Result<daemon::ListWorkloadsResponse, super::super::DaemonServiceFailure> {
            Err(super::super::DaemonServiceFailure::Backend)
        }

        async fn inspect(
            &self,
            _: d2b_contracts::v2_services::common::ServiceRequest,
            _: &super::super::DaemonCallContext,
        ) -> Result<daemon::InspectResponse, super::super::DaemonServiceFailure> {
            Err(super::super::DaemonServiceFailure::Backend)
        }

        async fn prepare_terminal(
            &self,
            _: DaemonMethod,
            _: &terminal::TerminalOpenRequest,
            _: &super::super::DaemonCallContext,
        ) -> Result<Arc<dyn PreparedTerminal>, TerminalFailure> {
            Err(TerminalFailure::Unavailable)
        }
    }

    struct FakeProxySession {
        generations: Mutex<Vec<u64>>,
        cancelled: AtomicUsize,
        abandoned: AtomicUsize,
        prepare_blocked: AtomicBool,
        prepare_entered: AtomicBool,
        prepare_release: Notify,
    }

    impl FakeProxySession {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                generations: Mutex::new(Vec::new()),
                cancelled: AtomicUsize::new(0),
                abandoned: AtomicUsize::new(0),
                prepare_blocked: AtomicBool::new(false),
                prepare_entered: AtomicBool::new(false),
                prepare_release: Notify::new(),
            })
        }

        fn new_with_blocked_prepare() -> Arc<Self> {
            let session = Self::new();
            session.prepare_blocked.store(true, Ordering::Release);
            session
        }

        fn record_context(
            &self,
            context: &d2b_contracts::v2_services::guest::GuestOperationContext,
        ) {
            self.generations.lock().unwrap().push(
                context
                    .metadata
                    .as_ref()
                    .expect("metadata")
                    .session_generation,
            );
        }
    }

    impl std::fmt::Debug for FakeProxySession {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("FakeProxySession(REDACTED)")
        }
    }

    #[async_trait]
    impl GuestProxySession for FakeProxySession {
        fn generation(&self) -> u64 {
            GUEST_GENERATION
        }

        async fn cancel_exec(
            &self,
            request: GuestCancelExecRequest,
            _: Duration,
        ) -> Result<GuestCancelExecResponse, TerminalFailure> {
            let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
            self.record_context(context);
            Ok(GuestCancelExecResponse {
                outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
                operation_id: context.operation_id.clone(),
                session_generation: GUEST_GENERATION,
                request_id: context.metadata.as_ref().unwrap().request_id.clone(),
                resource_handle: request.resource_handle,
                cancellation: EnumOrUnknown::new(
                    GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED,
                ),
                ..Default::default()
            })
        }

        async fn inspect_exec(
            &self,
            request: GuestInspectExecRequest,
            _: Duration,
        ) -> Result<GuestInspectExecResponse, TerminalFailure> {
            let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
            self.record_context(context);
            let resource_handle = request
                .query
                .as_ref()
                .and_then(|query| query.query.as_ref())
                .and_then(|query| match query {
                    d2b_contracts::v2_services::guest::guest_inspect_exec_query::Query::Status(
                        status,
                    ) => Some(status.resource_handle.clone()),
                    _ => None,
                })
                .ok_or(TerminalFailure::Protocol)?;
            let mut response = GuestInspectExecResponse {
                outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
                operation_id: context.operation_id.clone(),
                session_generation: GUEST_GENERATION,
                request_id: context.metadata.as_ref().unwrap().request_id.clone(),
                ..Default::default()
            };
            response.set_status(d2b_contracts::v2_services::guest::GuestExecStatus {
                resource_handle,
                state: EnumOrUnknown::new(GuestExecState::GUEST_EXEC_STATE_RUNNING),
                stdin_state:
                    d2b_contracts::v2_services::guest::GuestStdinState::GUEST_STDIN_STATE_OPEN
                        .into(),
                state_generation: 1,
                ..Default::default()
            });
            Ok(response)
        }

        async fn prepare_retained_log(
            &self,
            request: GuestOpenExecRetainedLogRequest,
            _: Duration,
        ) -> Result<terminal::TerminalOpenResponse, TerminalFailure> {
            let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
            self.record_context(context);
            self.prepare_entered.store(true, Ordering::Release);
            if self.prepare_blocked.load(Ordering::Acquire) {
                self.prepare_release.notified().await;
            }
            Ok(terminal::TerminalOpenResponse {
                outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
                operation_id: context.operation_id.clone(),
                stream_id: "stream-300".to_owned(),
                session_generation: GUEST_GENERATION,
                request_id: context.metadata.as_ref().unwrap().request_id.clone(),
                resource_handle: request.resource_handle,
                retained_log: MessageField::some(terminal::TerminalRetainedLogRange {
                    output: request.output,
                    requested_offset: request.offset,
                    start_offset: request.offset,
                    end_offset: request.offset + u64::from(request.max_bytes),
                    max_bytes: request.max_bytes,
                    eof: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
        }

        async fn open_prepared_retained_log(
            &self,
            _: &GuestOpenExecRetainedLogRequest,
            _: terminal::TerminalOpenResponse,
            _: terminal::TerminalSelection,
        ) -> Result<TerminalOpenResult, TerminalFailure> {
            Err(TerminalFailure::Internal)
        }

        async fn abandon_retained_log(&self, _: &str) {
            self.abandoned.fetch_add(1, Ordering::AcqRel);
        }

        async fn cancel_request(&self, _: &[u8]) {
            self.cancelled.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct FakeConnector {
        session: Arc<FakeProxySession>,
        connections: AtomicUsize,
    }

    impl std::fmt::Debug for FakeConnector {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("FakeConnector(REDACTED)")
        }
    }

    #[async_trait]
    impl GuestTerminalConnector for FakeConnector {
        async fn acquire_material(
            &self,
            _: &str,
        ) -> Result<GuestSessionCredentialV1, TerminalFailure> {
            Err(TerminalFailure::Unavailable)
        }

        async fn connect_with_material(
            &self,
            _: &str,
            _: GuestSessionCredentialV1,
        ) -> Result<Arc<crate::guest_terminal::GuestTerminalSession>, TerminalFailure> {
            Err(TerminalFailure::Unavailable)
        }

        async fn connect_proxy(
            &self,
            workload: &str,
        ) -> Result<Arc<dyn GuestProxySession>, TerminalFailure> {
            assert_eq!(workload, "bbbbbbbbbbbbbbbbbbba");
            self.connections.fetch_add(1, Ordering::AcqRel);
            let session: Arc<dyn GuestProxySession> = self.session.clone();
            Ok(session)
        }
    }

    fn rpc_context() -> ttrpc::r#async::TtrpcContext {
        ttrpc::r#async::TtrpcContext {
            mh: ttrpc::proto::MessageHeader::new_request(1, 0),
            metadata: HashMap::new(),
            timeout_nano: 0,
        }
    }

    fn operation_context(
        request_byte: u8,
    ) -> d2b_contracts::v2_services::guest::GuestOperationContext {
        let now = super::super::now_unix_ms();
        d2b_contracts::v2_services::guest::GuestOperationContext {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: vec![request_byte; 16],
                idempotency_key: vec![request_byte.wrapping_add(1); 16],
                issued_at_unix_ms: now,
                expires_at_unix_ms: now + 5_000,
                session_generation: PUBLIC_GENERATION,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
                ..Default::default()
            }),
            operation_id: format!("operation-{request_byte}"),
            request_digest: vec![request_byte.wrapping_add(2); 32],
            ..Default::default()
        }
    }

    fn inspect_request(request_byte: u8) -> GuestInspectExecRequest {
        let mut query = d2b_contracts::v2_services::guest::GuestInspectExecQuery::new();
        query.set_status(d2b_contracts::v2_services::guest::GuestExecStatusQuery {
            resource_handle: "exec-1".to_owned(),
            ..Default::default()
        });
        GuestInspectExecRequest {
            context: MessageField::some(operation_context(request_byte)),
            query: MessageField::some(query),
            ..Default::default()
        }
    }

    fn proxy(
        role: DaemonPeerRole,
        connector: Arc<dyn GuestTerminalConnector>,
    ) -> (Arc<DaemonGuestProxy<NoopHandler>>, Arc<PublicDriver>) {
        let driver = PublicDriver::new();
        let erased: Arc<dyn ComponentSessionDriver> = driver.clone();
        let daemon = Arc::new(DaemonServiceV2::new(
            Arc::new(NoopHandler),
            erased,
            role,
            1000,
        ));
        (DaemonGuestProxy::new(daemon, connector), driver)
    }

    #[tokio::test]
    async fn inspect_and_cancel_proxy_preserve_public_binding_and_rebind_guest_generation() {
        let session = FakeProxySession::new();
        let connector = Arc::new(FakeConnector {
            session: Arc::clone(&session),
            connections: AtomicUsize::new(0),
        });
        let (proxy, _) = proxy(DaemonPeerRole::Admin, connector.clone());

        let inspect = inspect_request(1);
        let inspect_response = proxy
            .inspect_exec(&rpc_context(), inspect.clone())
            .await
            .unwrap();
        assert_eq!(inspect_response.session_generation, PUBLIC_GENERATION);
        assert_eq!(
            inspect_response.request_id,
            inspect
                .context
                .as_ref()
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()
                .request_id
        );

        let cancel = GuestCancelExecRequest {
            context: MessageField::some(operation_context(2)),
            resource_handle: "exec-1".to_owned(),
            control_sequence: 1,
            reason: EnumOrUnknown::new(
                GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED,
            ),
            ..Default::default()
        };
        let cancel_response = proxy
            .cancel_exec(&rpc_context(), cancel.clone())
            .await
            .unwrap();
        assert_eq!(cancel_response.session_generation, PUBLIC_GENERATION);
        assert_eq!(
            cancel_response.request_id,
            cancel
                .context
                .as_ref()
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()
                .request_id
        );
        assert_eq!(
            *session.generations.lock().unwrap(),
            [GUEST_GENERATION, GUEST_GENERATION]
        );
        assert_eq!(connector.connections.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn retained_log_proxy_allocates_public_stream_and_cancel_abandons_upstream() {
        let session = FakeProxySession::new();
        let connector = Arc::new(FakeConnector {
            session: Arc::clone(&session),
            connections: AtomicUsize::new(0),
        });
        let (proxy, driver) = proxy(DaemonPeerRole::Admin, connector);
        let request = GuestOpenExecRetainedLogRequest {
            context: MessageField::some(operation_context(3)),
            resource_handle: "exec-1".to_owned(),
            output: EnumOrUnknown::new(terminal::OutputStream::OUTPUT_STREAM_STDOUT),
            offset: 4,
            max_bytes: 4,
            ..Default::default()
        };
        let response = proxy
            .open_exec_retained_log(&rpc_context(), request.clone())
            .await
            .unwrap();
        assert_eq!(response.session_generation, PUBLIC_GENERATION);
        assert_eq!(response.resource_handle, "exec-1");
        assert!(response.stream_id.starts_with("stream-"));
        assert_eq!(driver.opened.load(Ordering::Acquire), 1);
        validate_guest_open_exec_retained_log_response_for_request(&request, &response).unwrap();

        let cancellation = common::CancelRequest {
            session_generation: PUBLIC_GENERATION,
            request_id: request
                .context
                .as_ref()
                .unwrap()
                .metadata
                .as_ref()
                .unwrap()
                .request_id
                .clone(),
            ..Default::default()
        };
        let cancelled = proxy.cancel(&rpc_context(), cancellation).await.unwrap();
        assert_eq!(
            cancelled.outcome.enum_value().unwrap(),
            common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while session.abandoned.load(Ordering::Acquire) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(session.abandoned.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn retained_log_cancel_propagates_upstream_once_when_response_is_also_ready() {
        let session = FakeProxySession::new_with_blocked_prepare();
        let connector = Arc::new(FakeConnector {
            session: Arc::clone(&session),
            connections: AtomicUsize::new(0),
        });
        let (proxy, driver) = proxy(DaemonPeerRole::Admin, connector);
        let request = GuestOpenExecRetainedLogRequest {
            context: MessageField::some(operation_context(6)),
            resource_handle: "exec-1".to_owned(),
            output: EnumOrUnknown::new(terminal::OutputStream::OUTPUT_STREAM_STDOUT),
            offset: 4,
            max_bytes: 4,
            ..Default::default()
        };
        let request_id = request
            .context
            .as_ref()
            .unwrap()
            .metadata
            .as_ref()
            .unwrap()
            .request_id
            .clone();
        let task = tokio::spawn({
            let proxy = Arc::clone(&proxy);
            let request = request.clone();
            async move { proxy.open_exec_retained_log(&rpc_context(), request).await }
        });
        while !session.prepare_entered.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        let cancellation = common::CancelRequest {
            session_generation: PUBLIC_GENERATION,
            request_id: request_id.clone(),
            ..Default::default()
        };
        let first = proxy
            .cancel(&rpc_context(), cancellation.clone())
            .await
            .unwrap();
        let second = proxy.cancel(&rpc_context(), cancellation).await.unwrap();
        session.prepare_release.notify_waiters();

        assert_eq!(
            first.outcome.enum_value().unwrap(),
            common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
        );
        assert_eq!(
            second.outcome.enum_value().unwrap(),
            common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
        );
        assert!(task.await.unwrap().is_err());
        assert_eq!(session.cancelled.load(Ordering::Acquire), 1);
        assert_eq!(session.abandoned.load(Ordering::Acquire), 0);
        assert_eq!(driver.opened.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn guest_cancel_bypasses_saturated_operation_slots() {
        let unavailable: Arc<dyn GuestTerminalConnector> =
            Arc::new(crate::guest_terminal::UnavailableGuestTerminalConnector);
        let (proxy, _) = proxy(DaemonPeerRole::Admin, unavailable);
        let semaphore = proxy.daemon.in_flight();
        let permits = (0..64)
            .map(|_| semaphore.clone().try_acquire_owned().unwrap())
            .collect::<Vec<_>>();
        let response = proxy
            .cancel(
                &rpc_context(),
                common::CancelRequest {
                    session_generation: PUBLIC_GENERATION,
                    request_id: vec![0x77; 16],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );
        assert_eq!(permits.len(), 64);
    }

    #[tokio::test]
    async fn proxy_is_registered_and_fails_closed_without_material_or_admin() {
        let unavailable: Arc<dyn GuestTerminalConnector> =
            Arc::new(crate::guest_terminal::UnavailableGuestTerminalConnector);
        let (admin, _) = proxy(DaemonPeerRole::Admin, Arc::clone(&unavailable));
        let services = create_guest_service(admin.clone());
        assert!(services.contains_key("d2b.guest.v2.GuestService"));
        assert!(
            admin
                .inspect_exec(&rpc_context(), inspect_request(4))
                .await
                .is_err()
        );

        let session = FakeProxySession::new();
        let connector = Arc::new(FakeConnector {
            session,
            connections: AtomicUsize::new(0),
        });
        let (launcher, _) = proxy(DaemonPeerRole::Launcher, connector.clone());
        assert!(
            launcher
                .inspect_exec(&rpc_context(), inspect_request(5))
                .await
                .is_err()
        );
        assert_eq!(connector.connections.load(Ordering::Acquire), 0);
    }
}
