use super::*;

pub const MAX_GUEST_CAPABILITIES: usize = 32;
pub const MAX_GUEST_EXEC_LIST_ENTRIES: usize = 32;
pub const MAX_GUEST_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_GUEST_FILE_CHUNK_BYTES: usize = 64 * 1024;
pub const CTAPHID_REPORT_BYTES: usize = 64;
pub const MAX_GUEST_WAIT_MS: u32 = 1_000;

fn validate_guest_scope(scope: &common::IdentityScope) -> Result<(), ServiceContractError> {
    validate_scope(scope)?;
    if scope.workload_id.is_empty() || !scope.provider_id.is_empty() || !scope.role_id.is_empty() {
        return Err(ServiceContractError::InvalidIdentity);
    }
    Ok(())
}

fn validate_guest_context(
    context: &guest::GuestOperationContext,
    requires_idempotency: bool,
) -> Result<(), ServiceContractError> {
    reject_unknown(context)?;
    validate_metadata(required_message(&context.metadata)?, requires_idempotency)?;
    validate_guest_scope(required_message(&context.scope)?)?;
    if !bounded_opaque(&context.operation_id, MAX_SERVICE_STRING_BYTES)
        || !required_digest(&context.request_digest)
    {
        return Err(ServiceContractError::InvalidOperationInput);
    }
    Ok(())
}

fn guest_context(
    context: &MessageField<guest::GuestOperationContext>,
) -> Result<&guest::GuestOperationContext, ServiceContractError> {
    context
        .as_ref()
        .ok_or(ServiceContractError::MissingMetadata)
}

fn validate_guest_terminal_request(
    request: &terminal::TerminalOpenRequest,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_guest_scope(required_message(&request.scope)?)
}

impl StrictWireMessage for guest::GuestExecRequest {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_terminal_request(
            self.terminal
                .as_ref()
                .ok_or(ServiceContractError::MissingOperationInput)?,
        )
    }
}

impl StrictWireMessage for guest::GuestOpenShellRequest {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_terminal_request(
            self.terminal
                .as_ref()
                .ok_or(ServiceContractError::MissingOperationInput)?,
        )
    }
}

pub fn validate_guest_exec_response_for_request(
    request: &guest::GuestExecRequest,
    response: &terminal::TerminalOpenResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_terminal_open_response_for_request(
        request
            .terminal
            .as_ref()
            .ok_or(ServiceContractError::MissingOperationInput)?,
        response,
    )
}

pub fn validate_guest_open_shell_response_for_request(
    request: &guest::GuestOpenShellRequest,
    response: &terminal::TerminalOpenResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_terminal_open_response_for_request(
        request
            .terminal
            .as_ref()
            .ok_or(ServiceContractError::MissingOperationInput)?,
        response,
    )
}

fn validate_capabilities(
    capabilities: &[EnumOrUnknown<guest::GuestCapability>],
) -> Result<(), ServiceContractError> {
    if capabilities.len() > MAX_GUEST_CAPABILITIES {
        return Err(ServiceContractError::BoundExceeded);
    }
    let mut unique = BTreeSet::new();
    for capability in capabilities {
        if !valid_required_enum(
            capability,
            guest::GuestCapability::GUEST_CAPABILITY_UNSPECIFIED,
        ) || !unique.insert(capability.value())
        {
            return Err(ServiceContractError::InvalidEnum);
        }
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestBootstrapRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        let context = guest_context(&self.context)?;
        validate_guest_context(context, requires_idempotency)?;
        let metadata = required_message(&context.metadata)?;
        if self.expected_generation == 0
            || self.expected_generation != metadata.session_generation
            || !required_digest(&self.expected_parent_static_public_key_digest)
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        validate_capabilities(&self.requested_capabilities)
    }
}

impl StrictWireMessage for guest::GuestReconnectRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        let context = guest_context(&self.context)?;
        validate_guest_context(context, requires_idempotency)?;
        let metadata = required_message(&context.metadata)?;
        if self.expected_generation == 0
            || self.expected_generation != metadata.session_generation
            || !bounded_opaque(&self.guest_identity_handle, MAX_SERVICE_STRING_BYTES)
            || !required_digest(&self.expected_guest_static_public_key_digest)
            || !required_digest(&self.expected_parent_static_public_key_digest)
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        validate_capabilities(&self.required_capabilities)
    }
}

impl StrictWireMessage for guest::GuestSessionResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
        {
            return Err(ServiceContractError::InvalidId);
        }
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        match outcome {
            common::Outcome::OUTCOME_SUCCEEDED => {
                if self.error.is_some()
                    || !bounded_opaque(&self.guest_identity_handle, MAX_SERVICE_STRING_BYTES)
                    || !required_digest(&self.guest_static_public_key_digest)
                    || !required_digest(&self.parent_static_public_key_digest)
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                validate_capabilities(&self.capabilities)
            }
            common::Outcome::OUTCOME_DENIED
            | common::Outcome::OUTCOME_CANCELLED
            | common::Outcome::OUTCOME_FAILED => {
                if !self.guest_identity_handle.is_empty()
                    || !self.guest_static_public_key_digest.is_empty()
                    || !self.parent_static_public_key_digest.is_empty()
                    || !self.capabilities.is_empty()
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                validate_error(
                    self.error
                        .as_ref()
                        .ok_or(ServiceContractError::InconsistentResponse)?,
                )
            }
            _ => Err(ServiceContractError::InconsistentResponse),
        }
    }
}

fn validate_guest_session_response(
    context: &guest::GuestOperationContext,
    expected_generation: u64,
    expected_parent_digest: &[u8],
    expected_guest_digest: Option<&[u8]>,
    response: &guest::GuestSessionResponse,
) -> Result<(), ServiceContractError> {
    response.validate_wire(false)?;
    let metadata = required_message(&context.metadata)?;
    if response.operation_id != context.operation_id
        || response.session_generation != expected_generation
        || response.session_generation != metadata.session_generation
        || response.request_id != metadata.request_id
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    if response.outcome.enum_value().ok() == Some(common::Outcome::OUTCOME_SUCCEEDED)
        && (response.parent_static_public_key_digest != expected_parent_digest
            || expected_guest_digest
                .is_some_and(|digest| response.guest_static_public_key_digest != digest))
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

pub fn validate_guest_session_response_for_bootstrap(
    request: &guest::GuestBootstrapRequest,
    response: &guest::GuestSessionResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_guest_session_response(
        guest_context(&request.context)?,
        request.expected_generation,
        &request.expected_parent_static_public_key_digest,
        None,
        response,
    )?;
    if response.outcome.enum_value().ok() == Some(common::Outcome::OUTCOME_SUCCEEDED)
        && request.requested_capabilities.iter().any(|required| {
            !response
                .capabilities
                .iter()
                .any(|actual| actual.value() == required.value())
        })
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

pub fn validate_guest_session_response_for_reconnect(
    request: &guest::GuestReconnectRequest,
    response: &guest::GuestSessionResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_guest_session_response(
        guest_context(&request.context)?,
        request.expected_generation,
        &request.expected_parent_static_public_key_digest,
        Some(&request.expected_guest_static_public_key_digest),
        response,
    )?;
    if response.outcome.enum_value().ok() == Some(common::Outcome::OUTCOME_SUCCEEDED)
        && (response.guest_identity_handle != request.guest_identity_handle
            || request.required_capabilities.iter().any(|required| {
                !response
                    .capabilities
                    .iter()
                    .any(|actual| actual.value() == required.value())
            }))
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestCancelExecRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_context(guest_context(&self.context)?, requires_idempotency)?;
        if !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES)
            || self.control_sequence == 0
            || !valid_required_enum(
                &self.reason,
                guest::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_UNSPECIFIED,
            )
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        Ok(())
    }
}

impl StrictWireMessage for guest::GuestCancelExecResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES)
            || self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
            || !valid_required_enum(
                &self.cancellation,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNSPECIFIED,
            )
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        let cancellation = self
            .cancellation
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        match (outcome, cancellation, self.error.as_ref()) {
            (
                common::Outcome::OUTCOME_ACCEPTED,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED,
                None,
            )
            | (
                common::Outcome::OUTCOME_NOT_APPLICABLE,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_ALREADY_TERMINAL,
                None,
            ) => Ok(()),
            (
                common::Outcome::OUTCOME_FAILED,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNKNOWN_RESOURCE,
                Some(error),
            ) if error.kind.enum_value().ok() == Some(common::ErrorKind::ERROR_KIND_NOT_FOUND) => {
                validate_error(error)
            }
            (
                common::Outcome::OUTCOME_FAILED,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_GENERATION_MISMATCH,
                Some(error),
            ) if error.kind.enum_value().ok()
                == Some(common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH) =>
            {
                validate_error(error)
            }
            _ => Err(ServiceContractError::InconsistentResponse),
        }
    }
}

pub fn validate_guest_cancel_response_for_request(
    request: &guest::GuestCancelExecRequest,
    response: &guest::GuestCancelExecResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    response.validate_wire(false)?;
    let context = guest_context(&request.context)?;
    let metadata = required_message(&context.metadata)?;
    if response.operation_id != context.operation_id
        || response.resource_handle != request.resource_handle
        || response.session_generation != metadata.session_generation
        || response.request_id != metadata.request_id
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_exec_query(query: &guest::GuestInspectExecQuery) -> Result<(), ServiceContractError> {
    reject_unknown(query)?;
    use guest::guest_inspect_exec_query::Query;
    match query
        .query
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Query::Status(status) => {
            reject_unknown(status)?;
            if !bounded_opaque(&status.resource_handle, MAX_SERVICE_STRING_BYTES) {
                return Err(ServiceContractError::InvalidId);
            }
        }
        Query::Wait(wait) => {
            reject_unknown(wait)?;
            if !bounded_opaque(&wait.resource_handle, MAX_SERVICE_STRING_BYTES)
                || wait.known_state_generation == 0
                || wait.timeout_ms == 0
                || wait.timeout_ms > MAX_GUEST_WAIT_MS
            {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        Query::ListPage(page) => {
            reject_unknown(page)?;
            if page.page_size == 0
                || page.page_size as usize > MAX_GUEST_EXEC_LIST_ENTRIES
                || (!page.page_cursor.is_empty()
                    && !bounded_ascii(&page.page_cursor, MAX_PAGE_CURSOR_BYTES))
            {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestInspectExecRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_context(guest_context(&self.context)?, requires_idempotency)?;
        validate_exec_query(
            self.query
                .as_ref()
                .ok_or(ServiceContractError::MissingOperationInput)?,
        )
    }
}

fn terminal_state(state: guest::GuestExecState) -> bool {
    matches!(
        state,
        guest::GuestExecState::GUEST_EXEC_STATE_EXITED
            | guest::GuestExecState::GUEST_EXEC_STATE_SIGNALED
            | guest::GuestExecState::GUEST_EXEC_STATE_CANCELLED
            | guest::GuestExecState::GUEST_EXEC_STATE_PROTOCOL_ERROR
            | guest::GuestExecState::GUEST_EXEC_STATE_LOST
            | guest::GuestExecState::GUEST_EXEC_STATE_REAPED
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalOutcomeClass {
    Exited,
    Signaled,
    Cancelled,
    Failed,
    Other,
}

fn terminal_outcome_class(value: &terminal::TerminalOutcome) -> TerminalOutcomeClass {
    use terminal::terminal_outcome::Outcome;
    match value.outcome.as_ref() {
        Some(Outcome::Exited(_)) => TerminalOutcomeClass::Exited,
        Some(Outcome::Signaled(_)) => TerminalOutcomeClass::Signaled,
        Some(Outcome::Cancelled(_)) => TerminalOutcomeClass::Cancelled,
        Some(Outcome::Failed(_)) => TerminalOutcomeClass::Failed,
        Some(Outcome::Detached(_) | Outcome::Closed(_)) | None => TerminalOutcomeClass::Other,
    }
}

fn validate_exec_status(value: &guest::GuestExecStatus) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let state = value
        .state
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    let stdin = value
        .stdin_state
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if state == guest::GuestExecState::GUEST_EXEC_STATE_UNSPECIFIED
        || stdin == guest::GuestStdinState::GUEST_STDIN_STATE_UNSPECIFIED
        || !bounded_opaque(&value.resource_handle, MAX_SERVICE_STRING_BYTES)
        || value.state_generation == 0
        || value.stdout_start_offset > value.stdout_end_offset
        || value.stderr_start_offset > value.stderr_end_offset
        || (terminal_state(state)
            && matches!(
                stdin,
                guest::GuestStdinState::GUEST_STDIN_STATE_OPEN
                    | guest::GuestStdinState::GUEST_STDIN_STATE_CLOSING
            ))
        || (value.timed_out && terminal_state(state))
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    let outcome = value.terminal_outcome.as_ref();
    if let Some(outcome) = outcome {
        validate_terminal_outcome(outcome)?;
    }
    let outcome_class = outcome.map(terminal_outcome_class);
    let correlated = match state {
        guest::GuestExecState::GUEST_EXEC_STATE_CREATED
        | guest::GuestExecState::GUEST_EXEC_STATE_RUNNING => outcome_class.is_none(),
        guest::GuestExecState::GUEST_EXEC_STATE_EXITED => {
            outcome_class == Some(TerminalOutcomeClass::Exited)
        }
        guest::GuestExecState::GUEST_EXEC_STATE_SIGNALED => {
            outcome_class == Some(TerminalOutcomeClass::Signaled)
        }
        guest::GuestExecState::GUEST_EXEC_STATE_CANCELLED => {
            outcome_class == Some(TerminalOutcomeClass::Cancelled)
        }
        guest::GuestExecState::GUEST_EXEC_STATE_PROTOCOL_ERROR
        | guest::GuestExecState::GUEST_EXEC_STATE_LOST => {
            outcome_class == Some(TerminalOutcomeClass::Failed)
        }
        guest::GuestExecState::GUEST_EXEC_STATE_REAPED => matches!(
            outcome_class,
            Some(
                TerminalOutcomeClass::Exited
                    | TerminalOutcomeClass::Signaled
                    | TerminalOutcomeClass::Cancelled
                    | TerminalOutcomeClass::Failed
            )
        ),
        guest::GuestExecState::GUEST_EXEC_STATE_UNSPECIFIED => false,
    };
    if !correlated {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_exec_list(value: &guest::GuestExecListPage) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.entries.len() > MAX_GUEST_EXEC_LIST_ENTRIES
        || value.truncated == value.next_page_cursor.is_empty()
        || (!value.next_page_cursor.is_empty()
            && !bounded_ascii(&value.next_page_cursor, MAX_PAGE_CURSOR_BYTES))
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    let mut handles = BTreeSet::new();
    for entry in &value.entries {
        reject_unknown(entry)?;
        if !bounded_opaque(&entry.resource_handle, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &entry.state,
                guest::GuestExecState::GUEST_EXEC_STATE_UNSPECIFIED,
            )
            || entry.created_at_unix_ms == 0
            || !required_digest(&entry.argv_digest)
            || !handles.insert(entry.resource_handle.as_str())
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestInspectExecResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
        {
            return Err(ServiceContractError::InvalidId);
        }
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        if matches!(
            outcome,
            common::Outcome::OUTCOME_SUCCEEDED | common::Outcome::OUTCOME_DEGRADED
        ) {
            if self.error.is_some() {
                return Err(ServiceContractError::InconsistentResponse);
            }
            use guest::guest_inspect_exec_response::Result;
            match self
                .result
                .as_ref()
                .ok_or(ServiceContractError::InconsistentResponse)?
            {
                Result::Status(status) => validate_exec_status(status),
                Result::ListPage(page) => validate_exec_list(page),
            }
        } else if matches!(
            outcome,
            common::Outcome::OUTCOME_DENIED
                | common::Outcome::OUTCOME_CANCELLED
                | common::Outcome::OUTCOME_FAILED
        ) {
            if self.result.is_some() {
                return Err(ServiceContractError::InconsistentResponse);
            }
            validate_error(
                self.error
                    .as_ref()
                    .ok_or(ServiceContractError::InconsistentResponse)?,
            )
        } else {
            Err(ServiceContractError::InconsistentResponse)
        }
    }
}

pub fn validate_guest_inspect_response_for_request(
    request: &guest::GuestInspectExecRequest,
    response: &guest::GuestInspectExecResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(false)?;
    response.validate_wire(false)?;
    let context = guest_context(&request.context)?;
    let metadata = required_message(&context.metadata)?;
    if response.operation_id != context.operation_id
        || response.session_generation != metadata.session_generation
        || response.request_id != metadata.request_id
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_SUCCEEDED)
        && response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_DEGRADED)
    {
        return Ok(());
    }
    use guest::{guest_inspect_exec_query::Query, guest_inspect_exec_response::Result};
    let query = request
        .query
        .as_ref()
        .and_then(|query| query.query.as_ref())
        .ok_or(ServiceContractError::MissingOperationInput)?;
    let result = response
        .result
        .as_ref()
        .ok_or(ServiceContractError::InconsistentResponse)?;
    match (query, result) {
        (Query::Status(query), Result::Status(status))
            if status.resource_handle == query.resource_handle && !status.timed_out => {}
        (Query::Wait(query), Result::Status(status))
            if status.resource_handle == query.resource_handle =>
        {
            if status.state_generation < query.known_state_generation
                || (status.timed_out && status.state_generation != query.known_state_generation)
                || (!status.timed_out
                    && status.state_generation == query.known_state_generation
                    && !terminal_state(status.state.enum_value_or_default()))
            {
                return Err(ServiceContractError::InconsistentResponse);
            }
        }
        (Query::ListPage(_), Result::ListPage(_)) => {}
        _ => return Err(ServiceContractError::InconsistentResponse),
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestOpenExecRetainedLogRequest {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_context(guest_context(&self.context)?, true)?;
        if !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &self.output,
                terminal::OutputStream::OUTPUT_STREAM_UNSPECIFIED,
            )
            || self.max_bytes == 0
            || self.max_bytes as usize > MAX_TERMINAL_CHUNK_BYTES
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        Ok(())
    }
}

pub fn validate_guest_open_exec_retained_log_response_for_request(
    request: &guest::GuestOpenExecRetainedLogRequest,
    response: &terminal::TerminalOpenResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    validate_terminal_open_response_for_guest_context(guest_context(&request.context)?, response)?;
    if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED)
        || response.resource_handle != request.resource_handle
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    let range = response
        .retained_log
        .as_ref()
        .ok_or(ServiceContractError::InconsistentResponse)?;
    let requested_end = request
        .offset
        .checked_add(u64::from(request.max_bytes))
        .ok_or(ServiceContractError::BoundExceeded)?;
    if range.output != request.output
        || range.requested_offset != request.offset
        || range.max_bytes != request.max_bytes
        || range.start_offset < request.offset
        || range.start_offset > range.end_offset
        || range.end_offset > requested_end
        || range.end_offset.saturating_sub(range.start_offset) > u64::from(request.max_bytes)
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

pub fn retained_log_stream_validator(
    request: &guest::GuestOpenExecRetainedLogRequest,
    response: &terminal::TerminalOpenResponse,
) -> Result<TerminalStreamValidator, ServiceContractError> {
    validate_guest_open_exec_retained_log_response_for_request(request, response)?;
    let context = guest_context(&request.context)?;
    let metadata = required_message(&context.metadata)?;
    let request_id = metadata
        .request_id
        .as_slice()
        .try_into()
        .map_err(|_| ServiceContractError::InvalidId)?;
    let mut validator = TerminalStreamValidator::new(
        terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG,
        metadata.session_generation,
        request_id,
        context.operation_id.clone(),
        response.resource_handle.clone(),
    )?;
    validator.bind_retained_log_range(
        response
            .retained_log
            .as_ref()
            .ok_or(ServiceContractError::InconsistentResponse)?,
    )?;
    Ok(validator)
}

pub fn validate_terminal_open_response_for_guest_context(
    context: &guest::GuestOperationContext,
    response: &terminal::TerminalOpenResponse,
) -> Result<(), ServiceContractError> {
    validate_guest_context(context, true)?;
    response.validate_wire(false)?;
    let metadata = required_message(&context.metadata)?;
    if response.operation_id != context.operation_id
        || response.session_generation != metadata.session_generation
        || response.request_id != metadata.request_id
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestFileTransferRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_context(guest_context(&self.context)?, requires_idempotency)?;
        if !valid_required_enum(
            &self.artifact,
            guest::GuestArtifactId::GUEST_ARTIFACT_ID_UNSPECIFIED,
        ) || !bounded_opaque(&self.configured_intent_id, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &self.direction,
                guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_UNSPECIFIED,
            )
            || self.declared_size > MAX_GUEST_FILE_BYTES
            || self.offset > self.declared_size
            || !optional_digest(&self.expected_digest)
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        Ok(())
    }
}

fn validate_file_start(value: &guest::GuestFileTransferStart) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.artifact,
        guest::GuestArtifactId::GUEST_ARTIFACT_ID_UNSPECIFIED,
    ) || !bounded_opaque(&value.configured_intent_id, MAX_SERVICE_STRING_BYTES)
        || !valid_required_enum(
            &value.direction,
            guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_UNSPECIFIED,
        )
        || value.declared_size > MAX_GUEST_FILE_BYTES
        || value.offset > value.declared_size
        || !optional_digest(&value.expected_digest)
    {
        return Err(ServiceContractError::InvalidOperationInput);
    }
    Ok(())
}

fn validate_file_frame(value: &guest::GuestFileTransferFrame) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.session_generation == 0
        || RequestId::new(value.request_id.clone()).is_err()
        || value.sequence > MAX_TERMINAL_FRAME_SEQUENCE
        || !bounded_opaque(&value.operation_id, MAX_SERVICE_STRING_BYTES)
        || !bounded_opaque(&value.resource_handle, MAX_SERVICE_STRING_BYTES)
    {
        return Err(ServiceContractError::InvalidId);
    }
    use guest::guest_file_transfer_frame::Frame;
    match value
        .frame
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Frame::Start(start) => validate_file_start(start),
        Frame::Chunk(chunk) => {
            reject_unknown(chunk)?;
            if (chunk.data.is_empty() && !chunk.eof)
                || chunk.data.len() > MAX_GUEST_FILE_CHUNK_BYTES
                || chunk.total_size > MAX_GUEST_FILE_BYTES
                || (chunk.eof && !required_digest(&chunk.final_digest))
                || (!chunk.eof && !chunk.final_digest.is_empty())
            {
                return Err(ServiceContractError::BoundExceeded);
            }
            Ok(())
        }
        Frame::Credit(credit) => {
            reject_unknown(credit)?;
            if credit.bytes == 0 || credit.bytes as usize > MAX_GUEST_FILE_CHUNK_BYTES {
                return Err(ServiceContractError::BoundExceeded);
            }
            Ok(())
        }
        Frame::Complete(complete) => {
            reject_unknown(complete)?;
            if complete.total_size > MAX_GUEST_FILE_BYTES || !required_digest(&complete.digest) {
                return Err(ServiceContractError::BoundExceeded);
            }
            Ok(())
        }
        Frame::Cancel(cancel) => reject_unknown(cancel),
        Frame::Failed(failed) => {
            reject_unknown(failed)?;
            if !valid_required_enum(
                &failed.error,
                guest::GuestFileTransferErrorKind::GUEST_FILE_TRANSFER_ERROR_KIND_UNSPECIFIED,
            ) || !valid_required_enum(&failed.retry, common::RetryClass::RETRY_CLASS_UNSPECIFIED)
            {
                return Err(ServiceContractError::InvalidEnum);
            }
            Ok(())
        }
    }
}

impl StrictWireMessage for guest::GuestFileTransferFrame {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        validate_file_frame(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestStreamDirection {
    ClientToServer,
    ServerToClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestStreamState {
    AwaitStart,
    Active,
    Closing,
    Terminal,
}

pub struct FileTransferStreamValidator {
    generation: u64,
    request_id: [u8; 16],
    operation_id: String,
    resource_handle: String,
    artifact: guest::GuestArtifactId,
    configured_intent_id: String,
    direction: guest::GuestFileTransferDirection,
    declared_size: u64,
    expected_digest: Vec<u8>,
    next_offset: u64,
    available_credit: u64,
    accepted_eof: Option<AcceptedFileEof>,
    next_client_sequence: u64,
    next_server_sequence: u64,
    state: GuestStreamState,
}

struct AcceptedFileEof {
    total_size: u64,
    digest: Vec<u8>,
}

impl fmt::Debug for FileTransferStreamValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileTransferStreamValidator")
            .field("generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("direction", &self.direction)
            .field("state", &self.state)
            .finish()
    }
}

impl FileTransferStreamValidator {
    pub fn new(
        request: &guest::GuestFileTransferRequest,
        response: &terminal::TerminalOpenResponse,
    ) -> Result<Self, ServiceContractError> {
        request.validate_wire(true)?;
        validate_terminal_open_response_for_guest_context(
            guest_context(&request.context)?,
            response,
        )?;
        if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED)
            || response.retained_log.is_some()
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        let context = guest_context(&request.context)?;
        let metadata = required_message(&context.metadata)?;
        Ok(Self {
            generation: metadata.session_generation,
            request_id: metadata
                .request_id
                .as_slice()
                .try_into()
                .map_err(|_| ServiceContractError::InvalidId)?,
            operation_id: context.operation_id.clone(),
            resource_handle: response.resource_handle.clone(),
            artifact: request.artifact.enum_value_or_default(),
            configured_intent_id: request.configured_intent_id.clone(),
            direction: request.direction.enum_value_or_default(),
            declared_size: request.declared_size,
            expected_digest: request.expected_digest.clone(),
            next_offset: request.offset,
            available_credit: 0,
            accepted_eof: None,
            next_client_sequence: 0,
            next_server_sequence: 0,
            state: GuestStreamState::AwaitStart,
        })
    }

    pub fn accept(
        &mut self,
        direction: GuestStreamDirection,
        frame: &guest::GuestFileTransferFrame,
    ) -> Result<(), ServiceContractError> {
        frame.validate_wire(false)?;
        validate_guest_frame_binding(
            GuestFrameBinding {
                generation: frame.session_generation,
                request_id: &frame.request_id,
                operation_id: &frame.operation_id,
                resource_handle: &frame.resource_handle,
            },
            GuestFrameBinding {
                generation: self.generation,
                request_id: &self.request_id,
                operation_id: &self.operation_id,
                resource_handle: &self.resource_handle,
            },
        )?;
        let expected = match direction {
            GuestStreamDirection::ClientToServer => self.next_client_sequence,
            GuestStreamDirection::ServerToClient => self.next_server_sequence,
        };
        if frame.sequence != expected {
            return Err(ServiceContractError::InconsistentResponse);
        }
        use guest::guest_file_transfer_frame::Frame;
        match (self.state, direction, frame.frame.as_ref()) {
            (
                GuestStreamState::AwaitStart,
                GuestStreamDirection::ClientToServer,
                Some(Frame::Start(start)),
            ) if start.artifact.enum_value().ok() == Some(self.artifact)
                && start.configured_intent_id == self.configured_intent_id
                && start.direction.enum_value().ok() == Some(self.direction)
                && start.declared_size == self.declared_size
                && start.offset == self.next_offset
                && start.expected_digest == self.expected_digest =>
            {
                self.state = GuestStreamState::Active;
            }
            (GuestStreamState::Active, actual, Some(Frame::Chunk(chunk)))
                if actual == self.data_sender() && chunk.offset == self.next_offset =>
            {
                let len = u64::try_from(chunk.data.len())
                    .map_err(|_| ServiceContractError::BoundExceeded)?;
                if len > self.available_credit {
                    return Err(ServiceContractError::BoundExceeded);
                }
                let next_offset = self
                    .next_offset
                    .checked_add(len)
                    .ok_or(ServiceContractError::BoundExceeded)?;
                if next_offset > self.declared_size
                    || chunk.total_size != self.declared_size
                    || (chunk.eof && next_offset != self.declared_size)
                    || (chunk.eof
                        && !self.expected_digest.is_empty()
                        && chunk.final_digest != self.expected_digest)
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                self.next_offset = next_offset;
                self.available_credit -= len;
                if chunk.eof {
                    self.accepted_eof = Some(AcceptedFileEof {
                        total_size: chunk.total_size,
                        digest: chunk.final_digest.clone(),
                    });
                    self.state = GuestStreamState::Closing;
                }
            }
            (GuestStreamState::Active, actual, Some(Frame::Credit(credit)))
                if actual != self.data_sender() && credit.next_offset == self.next_offset =>
            {
                let available_credit = self
                    .available_credit
                    .checked_add(u64::from(credit.bytes))
                    .ok_or(ServiceContractError::BoundExceeded)?;
                if available_credit > u64::from(MAX_NAMED_STREAM_QUEUE_BYTES) {
                    return Err(ServiceContractError::BoundExceeded);
                }
                self.available_credit = available_credit;
            }
            (
                GuestStreamState::Closing,
                GuestStreamDirection::ServerToClient,
                Some(Frame::Complete(complete)),
            ) => {
                let eof = self
                    .accepted_eof
                    .as_ref()
                    .ok_or(ServiceContractError::InconsistentResponse)?;
                if complete.total_size != self.declared_size
                    || complete.total_size != eof.total_size
                    || complete.digest != eof.digest
                    || self.next_offset != self.declared_size
                    || (!self.expected_digest.is_empty() && complete.digest != self.expected_digest)
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                self.state = GuestStreamState::Terminal;
            }
            (
                GuestStreamState::AwaitStart | GuestStreamState::Active,
                GuestStreamDirection::ClientToServer,
                Some(Frame::Cancel(_)),
            ) => self.state = GuestStreamState::Closing,
            (
                GuestStreamState::AwaitStart | GuestStreamState::Active | GuestStreamState::Closing,
                GuestStreamDirection::ServerToClient,
                Some(Frame::Failed(_)),
            ) => self.state = GuestStreamState::Terminal,
            _ => return Err(ServiceContractError::InconsistentResponse),
        }
        let next = expected
            .checked_add(1)
            .ok_or(ServiceContractError::BoundExceeded)?;
        match direction {
            GuestStreamDirection::ClientToServer => self.next_client_sequence = next,
            GuestStreamDirection::ServerToClient => self.next_server_sequence = next,
        }
        Ok(())
    }

    fn data_sender(&self) -> GuestStreamDirection {
        match self.direction {
            guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST => {
                GuestStreamDirection::ClientToServer
            }
            guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_GUEST_TO_HOST => {
                GuestStreamDirection::ServerToClient
            }
            guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_UNSPECIFIED => {
                unreachable!("validated request")
            }
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.state == GuestStreamState::Terminal
    }

    pub fn accept_transport_credit(&self, bytes: u32) -> Result<(), ServiceContractError> {
        validate_transport_credit(self.state, bytes)
    }

    pub fn accept_transport_close(&self) -> Result<(), ServiceContractError> {
        validate_transport_terminal(self.state)
    }

    pub fn accept_transport_reset(&self) -> Result<(), ServiceContractError> {
        validate_transport_terminal(self.state)
    }
}

impl StrictWireMessage for guest::GuestSecurityKeyRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_guest_context(guest_context(&self.context)?, requires_idempotency)?;
        let action = self
            .action
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        if !bounded_opaque(&self.device_handle, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &self.ceremony,
                guest::GuestSecurityKeyCeremonyKind::GUEST_SECURITY_KEY_CEREMONY_KIND_UNSPECIFIED,
            )
            || match action {
                guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_START => {
                    !self.ceremony_handle.is_empty()
                }
                guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_RESUME => {
                    !bounded_opaque(&self.ceremony_handle, MAX_SERVICE_STRING_BYTES)
                }
                guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_UNSPECIFIED => true,
            }
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        Ok(())
    }
}

fn validate_security_key_frame(
    value: &guest::GuestSecurityKeyFrame,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.session_generation == 0
        || RequestId::new(value.request_id.clone()).is_err()
        || value.sequence > MAX_TERMINAL_FRAME_SEQUENCE
        || !bounded_opaque(&value.operation_id, MAX_SERVICE_STRING_BYTES)
        || !bounded_opaque(&value.resource_handle, MAX_SERVICE_STRING_BYTES)
    {
        return Err(ServiceContractError::InvalidId);
    }
    use guest::guest_security_key_frame::Frame;
    match value
        .frame
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Frame::Open(open) => {
            reject_unknown(open)?;
            if !valid_required_enum(
                &open.action,
                guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_UNSPECIFIED,
            ) || !bounded_opaque(&open.device_handle, MAX_SERVICE_STRING_BYTES)
                || !bounded_opaque(&open.ceremony_handle, MAX_SERVICE_STRING_BYTES)
                || !valid_required_enum(
                    &open.ceremony,
                    guest::GuestSecurityKeyCeremonyKind::GUEST_SECURITY_KEY_CEREMONY_KIND_UNSPECIFIED,
                )
            {
                return Err(ServiceContractError::InvalidOperationInput);
            }
        }
        Frame::GuestReport(report) | Frame::DeviceReport(report) => {
            reject_unknown(report)?;
            if report.report.len() != CTAPHID_REPORT_BYTES {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        Frame::ApprovalRequest(approval) => {
            reject_unknown(approval)?;
            if !valid_required_enum(
                &approval.approval,
                guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_UNSPECIFIED,
            ) {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
        Frame::Approval(approval) => {
            reject_unknown(approval)?;
            if !valid_required_enum(
                &approval.decision,
                guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_UNSPECIFIED,
            ) {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
        Frame::Cancel(cancel) => reject_unknown(cancel)?,
        Frame::Complete(complete) => {
            reject_unknown(complete)?;
            if !valid_required_enum(
                &complete.outcome,
                guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_UNSPECIFIED,
            ) {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
        Frame::Failed(failed) => {
            reject_unknown(failed)?;
            if !valid_required_enum(
                &failed.error,
                guest::GuestSecurityKeyErrorKind::GUEST_SECURITY_KEY_ERROR_KIND_UNSPECIFIED,
            ) || !valid_required_enum(&failed.retry, common::RetryClass::RETRY_CLASS_UNSPECIFIED)
            {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestSecurityKeyFrame {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        validate_security_key_frame(self)
    }
}

pub struct SecurityKeyStreamValidator {
    generation: u64,
    request_id: [u8; 16],
    operation_id: String,
    resource_handle: String,
    action: guest::GuestSecurityKeyAction,
    device_handle: String,
    ceremony_handle: String,
    ceremony: guest::GuestSecurityKeyCeremonyKind,
    approval: SecurityKeyApprovalState,
    next_client_sequence: u64,
    next_server_sequence: u64,
    state: GuestStreamState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecurityKeyApprovalState {
    NotRequired,
    Required,
    Requested,
    Granted,
    Denied,
}

impl fmt::Debug for SecurityKeyStreamValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyStreamValidator")
            .field("generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("state", &self.state)
            .finish()
    }
}

impl SecurityKeyStreamValidator {
    pub fn new(
        request: &guest::GuestSecurityKeyRequest,
        response: &terminal::TerminalOpenResponse,
    ) -> Result<Self, ServiceContractError> {
        request.validate_wire(true)?;
        validate_terminal_open_response_for_guest_context(
            guest_context(&request.context)?,
            response,
        )?;
        if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED)
            || response.retained_log.is_some()
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        if request.action.enum_value().ok()
            == Some(guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_RESUME)
            && response.resource_handle != request.ceremony_handle
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        let context = guest_context(&request.context)?;
        let metadata = required_message(&context.metadata)?;
        Ok(Self {
            generation: metadata.session_generation,
            request_id: metadata
                .request_id
                .as_slice()
                .try_into()
                .map_err(|_| ServiceContractError::InvalidId)?,
            operation_id: context.operation_id.clone(),
            resource_handle: response.resource_handle.clone(),
            action: request.action.enum_value_or_default(),
            device_handle: request.device_handle.clone(),
            ceremony_handle: if request.ceremony_handle.is_empty() {
                response.resource_handle.clone()
            } else {
                request.ceremony_handle.clone()
            },
            ceremony: request.ceremony.enum_value_or_default(),
            approval: if request.approval_required {
                SecurityKeyApprovalState::Required
            } else {
                SecurityKeyApprovalState::NotRequired
            },
            next_client_sequence: 0,
            next_server_sequence: 0,
            state: GuestStreamState::AwaitStart,
        })
    }

    pub fn accept(
        &mut self,
        direction: GuestStreamDirection,
        frame: &guest::GuestSecurityKeyFrame,
    ) -> Result<(), ServiceContractError> {
        frame.validate_wire(false)?;
        validate_guest_frame_binding(
            GuestFrameBinding {
                generation: frame.session_generation,
                request_id: &frame.request_id,
                operation_id: &frame.operation_id,
                resource_handle: &frame.resource_handle,
            },
            GuestFrameBinding {
                generation: self.generation,
                request_id: &self.request_id,
                operation_id: &self.operation_id,
                resource_handle: &self.resource_handle,
            },
        )?;
        let expected = match direction {
            GuestStreamDirection::ClientToServer => self.next_client_sequence,
            GuestStreamDirection::ServerToClient => self.next_server_sequence,
        };
        if frame.sequence != expected {
            return Err(ServiceContractError::InconsistentResponse);
        }
        use guest::guest_security_key_frame::Frame;
        match (self.state, direction, frame.frame.as_ref()) {
            (
                GuestStreamState::AwaitStart,
                GuestStreamDirection::ClientToServer,
                Some(Frame::Open(open)),
            ) if open.action.enum_value().ok() == Some(self.action)
                && open.device_handle == self.device_handle
                && open.ceremony_handle == self.ceremony_handle
                && open.ceremony.enum_value().ok() == Some(self.ceremony) =>
            {
                self.state = GuestStreamState::Active;
            }
            (
                GuestStreamState::Active,
                GuestStreamDirection::ServerToClient,
                Some(Frame::GuestReport(_)),
            ) => {}
            (
                GuestStreamState::Active,
                GuestStreamDirection::ClientToServer,
                Some(Frame::DeviceReport(_)),
            ) if self.approval != SecurityKeyApprovalState::Denied => {}
            (
                GuestStreamState::Active,
                GuestStreamDirection::ServerToClient,
                Some(Frame::ApprovalRequest(_)),
            ) if self.approval == SecurityKeyApprovalState::Required => {
                self.approval = SecurityKeyApprovalState::Requested;
            }
            (
                GuestStreamState::Active,
                GuestStreamDirection::ClientToServer,
                Some(Frame::Approval(approval)),
            ) if self.approval == SecurityKeyApprovalState::Requested => {
                self.approval = match approval.decision.enum_value().map_err(|_| {
                    ServiceContractError::InvalidEnum
                })? {
                    guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_APPROVED => {
                        SecurityKeyApprovalState::Granted
                    }
                    guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_DENIED => {
                        SecurityKeyApprovalState::Denied
                    }
                    guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_UNSPECIFIED => {
                        return Err(ServiceContractError::InvalidEnum);
                    }
                };
            }
            (
                GuestStreamState::AwaitStart | GuestStreamState::Active,
                GuestStreamDirection::ClientToServer,
                Some(Frame::Cancel(_)),
            ) => self.state = GuestStreamState::Closing,
            (
                GuestStreamState::Active | GuestStreamState::Closing,
                GuestStreamDirection::ServerToClient,
                Some(Frame::Complete(complete)),
            ) => {
                let outcome = complete
                    .outcome
                    .enum_value()
                    .map_err(|_| ServiceContractError::InvalidEnum)?;
                let valid = match self.approval {
                    SecurityKeyApprovalState::NotRequired | SecurityKeyApprovalState::Granted => {
                        outcome
                            != guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_UNSPECIFIED
                    }
                    SecurityKeyApprovalState::Required | SecurityKeyApprovalState::Requested => {
                        outcome
                            != guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_SUCCEEDED
                    }
                    SecurityKeyApprovalState::Denied => matches!(
                        outcome,
                        guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED
                            | guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_CANCELLED
                    ),
                };
                if !valid {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                self.state = GuestStreamState::Terminal;
            }
            (
                GuestStreamState::AwaitStart | GuestStreamState::Active | GuestStreamState::Closing,
                GuestStreamDirection::ServerToClient,
                Some(Frame::Failed(_)),
            ) => self.state = GuestStreamState::Terminal,
            _ => return Err(ServiceContractError::InconsistentResponse),
        }
        let next = expected
            .checked_add(1)
            .ok_or(ServiceContractError::BoundExceeded)?;
        match direction {
            GuestStreamDirection::ClientToServer => self.next_client_sequence = next,
            GuestStreamDirection::ServerToClient => self.next_server_sequence = next,
        }
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        self.state == GuestStreamState::Terminal
    }

    pub fn accept_transport_credit(&self, bytes: u32) -> Result<(), ServiceContractError> {
        validate_transport_credit(self.state, bytes)
    }

    pub fn accept_transport_close(&self) -> Result<(), ServiceContractError> {
        validate_transport_terminal(self.state)
    }

    pub fn accept_transport_reset(&self) -> Result<(), ServiceContractError> {
        validate_transport_terminal(self.state)
    }
}

fn validate_transport_credit(
    state: GuestStreamState,
    bytes: u32,
) -> Result<(), ServiceContractError> {
    if bytes == 0 || bytes > MAX_NAMED_STREAM_QUEUE_BYTES || state == GuestStreamState::Terminal {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_transport_terminal(state: GuestStreamState) -> Result<(), ServiceContractError> {
    if state != GuestStreamState::Terminal {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

struct GuestFrameBinding<'a> {
    generation: u64,
    request_id: &'a [u8],
    operation_id: &'a str,
    resource_handle: &'a str,
}

fn validate_guest_frame_binding(
    actual: GuestFrameBinding<'_>,
    expected: GuestFrameBinding<'_>,
) -> Result<(), ServiceContractError> {
    if actual.generation != expected.generation
        || actual.request_id != expected.request_id
        || actual.operation_id != expected.operation_id
        || actual.resource_handle != expected.resource_handle
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

impl StrictWireMessage for guest::GuestShutdownRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        let context = guest_context(&self.context)?;
        validate_guest_context(context, requires_idempotency)?;
        let metadata = required_message(&context.metadata)?;
        if !valid_required_enum(
            &self.action,
            guest::GuestPowerAction::GUEST_POWER_ACTION_UNSPECIFIED,
        ) || self.deadline_unix_ms <= metadata.issued_at_unix_ms
            || self.deadline_unix_ms > metadata.expires_at_unix_ms
        {
            return Err(ServiceContractError::InvalidDeadline);
        }
        Ok(())
    }
}

impl StrictWireMessage for guest::GuestShutdownResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
        {
            return Err(ServiceContractError::InvalidId);
        }
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        let phase = self
            .phase
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        let final_outcome = self
            .final_outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        match (phase, final_outcome, outcome, self.error.as_ref()) {
            (
                guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_ACCEPTED,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_UNSPECIFIED,
                common::Outcome::OUTCOME_ACCEPTED,
                None,
            )
            | (
                guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED
                | guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_ALREADY_APPLIED,
                common::Outcome::OUTCOME_SUCCEEDED,
                None,
            )
            | (
                guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_CANCELLED,
                common::Outcome::OUTCOME_CANCELLED,
                None,
            ) => Ok(()),
            (
                guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_FAILED,
                common::Outcome::OUTCOME_FAILED,
                Some(error),
            ) => validate_error(error),
            _ => Err(ServiceContractError::InconsistentResponse),
        }
    }
}

pub fn validate_guest_shutdown_response_for_request(
    request: &guest::GuestShutdownRequest,
    response: &guest::GuestShutdownResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    response.validate_wire(false)?;
    let context = guest_context(&request.context)?;
    let metadata = required_message(&context.metadata)?;
    if response.operation_id != context.operation_id
        || response.session_generation != metadata.session_generation
        || response.request_id != metadata.request_id
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}
