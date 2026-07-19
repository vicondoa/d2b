//! Typed client for authenticated `d2b.guest.v2` management operations.

use d2b_contracts::v2_services::{
    StrictWireMessage, common, guest,
    guest_contract::{
        validate_guest_cancel_response_for_request, validate_guest_inspect_response_for_request,
        validate_guest_open_exec_retained_log_response_for_request,
    },
    terminal::{self, TerminalSelection},
};
use protobuf::{EnumOrUnknown, MessageField};

use crate::{
    CallOptions, CancellationToken, ClientError, ConnectedClient, DaemonClient, DaemonTerminal,
    MetadataInput, ServiceKind,
    daemon_service::{ensure_terminal_open_outcome, map_ttrpc_error, remote_error},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
enum GuestMethod {
    CancelExec = 3,
    InspectExec = 4,
    OpenExecRetainedLog = 5,
}

#[derive(Debug, Clone)]
pub struct GuestOperation {
    pub operation_id: String,
    pub request_digest: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct GuestInspectCall {
    pub operation: GuestOperation,
    pub query: guest::GuestInspectExecQuery,
}

#[derive(Debug, Clone)]
pub struct GuestCancelCall {
    pub operation: GuestOperation,
    pub resource_handle: String,
    pub control_sequence: u64,
    pub reason: guest::GuestExecCancelReason,
}

#[derive(Debug, Clone)]
pub struct GuestRetainedLogCall {
    pub operation: GuestOperation,
    pub resource_handle: String,
    pub output: terminal::OutputStream,
    pub offset: u64,
    pub max_bytes: u32,
}

pub struct GuestClient {
    inner: ConnectedClient,
}

impl std::fmt::Debug for GuestClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GuestClient([authenticated])")
    }
}

impl GuestClient {
    pub fn new(inner: ConnectedClient) -> Result<Self, ClientError> {
        if inner.service().kind() != ServiceKind::Guest {
            return Err(ClientError::InvalidService);
        }
        Ok(Self { inner })
    }

    pub const fn session_generation(&self) -> u64 {
        self.inner.session_generation()
    }

    pub async fn inspect_exec(
        &self,
        call: GuestInspectCall,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<guest::GuestInspectExecResponse, ClientError> {
        let method = self.method(GuestMethod::InspectExec)?;
        let (context, ttrpc_context) = self.operation_context(method, &call.operation, &options)?;
        let request = guest::GuestInspectExecRequest {
            context: MessageField::some(context),
            query: MessageField::some(call.query),
            ..Default::default()
        };
        request
            .validate_wire(false)
            .map_err(ClientError::ServiceContract)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .guest()?
                    .inspect_exec(ttrpc_context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        validate_guest_inspect_response_for_request(&request, &response)
            .map_err(ClientError::ServiceContract)?;
        ensure_guest_outcome(&response.outcome, response.error.as_ref())?;
        Ok(response)
    }

    pub async fn cancel_exec(
        &self,
        call: GuestCancelCall,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<guest::GuestCancelExecResponse, ClientError> {
        let method = self.method(GuestMethod::CancelExec)?;
        let (context, ttrpc_context) = self.operation_context(method, &call.operation, &options)?;
        let request = guest::GuestCancelExecRequest {
            context: MessageField::some(context),
            resource_handle: call.resource_handle,
            control_sequence: call.control_sequence,
            reason: EnumOrUnknown::new(call.reason),
            ..Default::default()
        };
        request
            .validate_wire(true)
            .map_err(ClientError::ServiceContract)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .guest()?
                    .cancel_exec(ttrpc_context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        validate_guest_cancel_response_for_request(&request, &response)
            .map_err(ClientError::ServiceContract)?;
        if response.error.is_some() {
            return Err(remote_error(
                response
                    .error
                    .as_ref()
                    .ok_or(ClientError::ContractViolation)?,
            ));
        }
        Ok(response)
    }

    pub async fn open_exec_retained_log(
        &self,
        call: GuestRetainedLogCall,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<DaemonTerminal, ClientError> {
        let method = self.method(GuestMethod::OpenExecRetainedLog)?;
        let request_id = options.metadata.request_id_bytes();
        let (context, ttrpc_context) = self.operation_context(method, &call.operation, &options)?;
        let request = guest::GuestOpenExecRetainedLogRequest {
            context: MessageField::some(context),
            resource_handle: call.resource_handle.clone(),
            output: EnumOrUnknown::new(call.output),
            offset: call.offset,
            max_bytes: call.max_bytes,
            ..Default::default()
        };
        request
            .validate_wire(true)
            .map_err(ClientError::ServiceContract)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .guest()?
                    .open_exec_retained_log(ttrpc_context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        validate_guest_open_exec_retained_log_response_for_request(&request, &response)
            .map_err(ClientError::ServiceContract)?;
        ensure_terminal_open_outcome(&response)?;
        let selection = TerminalSelection {
            selection: Some(terminal::terminal_selection::Selection::RetainedLog(
                terminal::RetainedLogSelection {
                    exec_handle: call.resource_handle,
                    output: EnumOrUnknown::new(call.output),
                    offset: call.offset,
                    max_bytes: call.max_bytes,
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        DaemonClient::terminal_from_open_response(
            &self.inner,
            terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG,
            self.session_generation(),
            request_id,
            &call.operation.operation_id,
            response,
            selection,
        )
        .await
    }

    fn method(&self, method: GuestMethod) -> Result<crate::MethodHandle, ClientError> {
        self.inner.service().method(method as u16)
    }

    fn operation_context(
        &self,
        method: crate::MethodHandle,
        operation: &GuestOperation,
        options: &CallOptions,
    ) -> Result<(guest::GuestOperationContext, ttrpc::context::Context), ClientError> {
        let (metadata, scope, context) = self.inner.prepare_operation_context(method, options)?;
        let operation_context = guest::GuestOperationContext {
            metadata: MessageField::some(metadata),
            scope: MessageField::some(scope),
            operation_id: operation.operation_id.clone(),
            request_digest: operation.request_digest.to_vec(),
            ..Default::default()
        };
        Ok((operation_context, context))
    }

    async fn call_with_cancellation<T>(
        &self,
        call: impl std::future::Future<Output = ttrpc::Result<T>>,
        metadata: &MetadataInput,
        cancellation: &CancellationToken,
    ) -> Result<T, ClientError> {
        if cancellation.is_cancelled() {
            self.inner.cancel_request(metadata).await;
            return Err(ClientError::Cancelled);
        }
        tokio::select! {
            response = call => response.map_err(map_ttrpc_error),
            () = cancellation.cancelled() => {
                self.inner.cancel_request(metadata).await;
                Err(ClientError::Cancelled)
            }
        }
    }
}

fn ensure_guest_outcome(
    outcome: &EnumOrUnknown<common::Outcome>,
    error: Option<&common::ErrorEnvelope>,
) -> Result<(), ClientError> {
    match outcome
        .enum_value()
        .map_err(|_| ClientError::ContractViolation)?
    {
        common::Outcome::OUTCOME_SUCCEEDED | common::Outcome::OUTCOME_DEGRADED => Ok(()),
        common::Outcome::OUTCOME_DENIED
        | common::Outcome::OUTCOME_CANCELLED
        | common::Outcome::OUTCOME_FAILED => {
            Err(remote_error(error.ok_or(ClientError::ContractViolation)?))
        }
        _ => Err(ClientError::ContractViolation),
    }
}
