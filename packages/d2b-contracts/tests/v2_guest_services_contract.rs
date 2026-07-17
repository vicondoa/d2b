#![cfg(feature = "v2-services")]

use d2b_contracts::v2_services::guest_contract::{
    CTAPHID_REPORT_BYTES, FileTransferStreamValidator, GuestStreamDirection,
    SecurityKeyStreamValidator, retained_log_stream_validator,
    validate_guest_cancel_response_for_request, validate_guest_exec_response_for_request,
    validate_guest_inspect_response_for_request,
    validate_guest_open_exec_retained_log_response_for_request,
    validate_guest_open_shell_response_for_request, validate_guest_session_response_for_bootstrap,
    validate_guest_session_response_for_reconnect, validate_guest_shutdown_response_for_request,
    validate_terminal_open_response_for_guest_context,
};
use d2b_contracts::v2_services::{
    ServiceContractError, StrictWireMessage, common, decode_strict, encode_strict, guest,
    method_spec, terminal,
};
use protobuf::{Message, MessageField};

const GENERATION: u64 = 7;
const REQUEST_ID: [u8; 16] = [0x11; 16];

fn metadata() -> common::RequestMetadata {
    common::RequestMetadata {
        request_id: REQUEST_ID.to_vec(),
        idempotency_key: vec![0x22; 16],
        issued_at_unix_ms: 1_000,
        expires_at_unix_ms: 5_000,
        session_generation: GENERATION,
        ..Default::default()
    }
}

fn context() -> guest::GuestOperationContext {
    guest::GuestOperationContext {
        metadata: MessageField::some(metadata()),
        scope: MessageField::some(common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            ..Default::default()
        }),
        operation_id: "operation-1".to_owned(),
        request_digest: vec![0x33; 32],
        ..Default::default()
    }
}

fn terminal_response(resource_handle: &str) -> terminal::TerminalOpenResponse {
    terminal::TerminalOpenResponse {
        outcome: common::Outcome::OUTCOME_ACCEPTED.into(),
        operation_id: "operation-1".to_owned(),
        stream_id: "stream-256".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        resource_handle: resource_handle.to_owned(),
        ..Default::default()
    }
}

fn terminal_request(scope: common::IdentityScope) -> terminal::TerminalOpenRequest {
    terminal::TerminalOpenRequest {
        metadata: MessageField::some(metadata()),
        scope: MessageField::some(scope),
        resource_id: "workload-1".to_owned(),
        operation_id: "operation-1".to_owned(),
        request_digest: vec![0x33; 32],
        ..Default::default()
    }
}

#[test]
fn guest_terminal_wrappers_require_exact_workload_scope() {
    let generated = include_str!("../src/generated_v2_services/guest_ttrpc.rs");
    assert_eq!(
        generated
            .matches("super::StrictWireMessage::validate_wire(&decoded, true)")
            .count(),
        2
    );
    assert!(generated.contains("super::guest::GuestExecRequest as ::protobuf::Message"));
    assert!(generated.contains("super::guest::GuestOpenShellRequest as ::protobuf::Message"));

    let valid = terminal_request(common::IdentityScope {
        realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
        workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
        ..Default::default()
    });
    let exec = guest::GuestExecRequest {
        terminal: MessageField::some(valid.clone()),
        ..Default::default()
    };
    let shell = guest::GuestOpenShellRequest {
        terminal: MessageField::some(valid),
        ..Default::default()
    };
    exec.validate_wire(true).unwrap();
    shell.validate_wire(true).unwrap();
    validate_guest_exec_response_for_request(&exec, &terminal_response("exec-1")).unwrap();
    validate_guest_open_shell_response_for_request(&shell, &terminal_response("shell-1")).unwrap();

    for invalid_scope in [
        common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            ..Default::default()
        },
        common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            provider_id: "caaaaaaaaaaaaaaaaaaq".to_owned(),
            ..Default::default()
        },
        common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            role_id: "daaaaaaaaaaaaaaaaaaq".to_owned(),
            ..Default::default()
        },
    ] {
        let invalid = guest::GuestExecRequest {
            terminal: MessageField::some(terminal_request(invalid_scope)),
            ..Default::default()
        };
        assert_eq!(
            invalid.validate_wire(true),
            Err(ServiceContractError::InvalidIdentity)
        );
    }
}

fn round_trip<T>(value: &T, requires_idempotency: bool)
where
    T: StrictWireMessage + Clone + PartialEq + std::fmt::Debug,
{
    let encoded = encode_strict(value, requires_idempotency).expect("strict encode");
    assert_eq!(
        decode_strict::<T>(&encoded, requires_idempotency).expect("strict decode"),
        *value
    );
}

#[test]
fn bootstrap_and_reconnect_bind_identity_generation_and_capabilities() {
    let bootstrap = guest::GuestBootstrapRequest {
        context: MessageField::some(context()),
        expected_generation: GENERATION,
        expected_parent_static_public_key_digest: vec![0x44; 32],
        requested_capabilities: vec![
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED.into(),
            guest::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED.into(),
        ],
        ..Default::default()
    };
    let response = guest::GuestSessionResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        guest_identity_handle: "guest-identity-1".to_owned(),
        guest_static_public_key_digest: vec![0x55; 32],
        parent_static_public_key_digest: vec![0x44; 32],
        capabilities: bootstrap.requested_capabilities.clone(),
        ..Default::default()
    };
    round_trip(&bootstrap, true);
    round_trip(&response, false);
    validate_guest_session_response_for_bootstrap(&bootstrap, &response).unwrap();

    let reconnect = guest::GuestReconnectRequest {
        context: MessageField::some(context()),
        expected_generation: GENERATION,
        guest_identity_handle: "guest-identity-1".to_owned(),
        expected_guest_static_public_key_digest: vec![0x55; 32],
        expected_parent_static_public_key_digest: vec![0x44; 32],
        required_capabilities: vec![guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED.into()],
        ..Default::default()
    };
    round_trip(&reconnect, true);
    validate_guest_session_response_for_reconnect(&reconnect, &response).unwrap();

    let mut mismatch = response;
    mismatch.session_generation += 1;
    assert_eq!(
        validate_guest_session_response_for_reconnect(&reconnect, &mismatch),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn cancel_exec_uses_the_exact_resource_and_generation() {
    let request = guest::GuestCancelExecRequest {
        context: MessageField::some(context()),
        resource_handle: "exec-1".to_owned(),
        control_sequence: 1,
        reason: guest::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED.into(),
        ..Default::default()
    };
    let response = guest::GuestCancelExecResponse {
        outcome: common::Outcome::OUTCOME_ACCEPTED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        resource_handle: "exec-1".to_owned(),
        cancellation:
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED.into(),
        ..Default::default()
    };
    round_trip(&request, true);
    round_trip(&response, false);
    validate_guest_cancel_response_for_request(&request, &response).unwrap();

    let mut mismatch = response;
    mismatch.resource_handle = "exec-2".to_owned();
    assert_eq!(
        validate_guest_cancel_response_for_request(&request, &mismatch),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut contradictory = guest::GuestCancelExecResponse {
        outcome: common::Outcome::OUTCOME_ACCEPTED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        resource_handle: "exec-1".to_owned(),
        cancellation:
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNKNOWN_RESOURCE
                .into(),
        ..Default::default()
    };
    assert_eq!(
        contradictory.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );
    contradictory.outcome = common::Outcome::OUTCOME_FAILED.into();
    contradictory.error = MessageField::some(common::ErrorEnvelope {
        kind: common::ErrorKind::ERROR_KIND_NOT_FOUND.into(),
        retry: common::RetryClass::RETRY_CLASS_NEVER.into(),
        ..Default::default()
    });
    contradictory.validate_wire(false).unwrap();
}

fn status_query() -> guest::GuestInspectExecRequest {
    let mut query = guest::GuestInspectExecQuery::new();
    query.set_status(guest::GuestExecStatusQuery {
        resource_handle: "exec-1".to_owned(),
        ..Default::default()
    });
    guest::GuestInspectExecRequest {
        context: MessageField::some(context()),
        query: MessageField::some(query),
        ..Default::default()
    }
}

fn running_status() -> guest::GuestExecStatus {
    guest::GuestExecStatus {
        resource_handle: "exec-1".to_owned(),
        state: guest::GuestExecState::GUEST_EXEC_STATE_RUNNING.into(),
        stdin_state: guest::GuestStdinState::GUEST_STDIN_STATE_OPEN.into(),
        stdout_end_offset: 4,
        stderr_end_offset: 2,
        state_generation: 1,
        ..Default::default()
    }
}

#[test]
fn inspect_exec_query_and_result_oneofs_are_exact() {
    let request = status_query();
    let mut response = guest::GuestInspectExecResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        ..Default::default()
    };
    response.set_status(running_status());
    round_trip(&request, false);
    round_trip(&response, false);
    validate_guest_inspect_response_for_request(&request, &response).unwrap();

    let mut list_query = guest::GuestInspectExecQuery::new();
    list_query.set_list_page(guest::GuestExecListPageQuery {
        page_size: 32,
        ..Default::default()
    });
    let list_request = guest::GuestInspectExecRequest {
        context: MessageField::some(context()),
        query: MessageField::some(list_query),
        ..Default::default()
    };
    let mut list_response = guest::GuestInspectExecResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        ..Default::default()
    };
    list_response.set_list_page(guest::GuestExecListPage {
        entries: vec![guest::GuestExecListEntry {
            resource_handle: "exec-1".to_owned(),
            state: guest::GuestExecState::GUEST_EXEC_STATE_RUNNING.into(),
            created_at_unix_ms: 1_000,
            argv_digest: vec![0x66; 32],
            ..Default::default()
        }],
        ..Default::default()
    });
    validate_guest_inspect_response_for_request(&list_request, &list_response).unwrap();

    let mut wrong = list_response;
    wrong.set_status(running_status());
    assert_eq!(
        validate_guest_inspect_response_for_request(&list_request, &wrong),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn retained_log_open_is_separate_mutating_idempotent_authority() {
    let inspect = method_spec("d2b.guest.v2", "GuestService", "InspectExec").unwrap();
    assert!(!inspect.mutating);
    assert!(!inspect.requires_idempotency);
    let open = method_spec("d2b.guest.v2", "GuestService", "OpenExecRetainedLog").unwrap();
    assert!(open.mutating);
    assert!(open.requires_idempotency);

    let mut query = guest::GuestInspectExecQuery::new();
    query.set_status(guest::GuestExecStatusQuery {
        resource_handle: "exec-1".to_owned(),
        ..Default::default()
    });
    let mut encoded = query.write_to_bytes().unwrap();
    encoded.extend_from_slice(&[0x22, 0x00]);
    let query = guest::GuestInspectExecQuery::parse_from_bytes(&encoded).unwrap();
    let request = guest::GuestInspectExecRequest {
        context: MessageField::some(context()),
        query: MessageField::some(query),
        ..Default::default()
    };
    assert_eq!(
        request.validate_wire(false),
        Err(ServiceContractError::UnknownField)
    );
}

fn inspect_status_response(status: guest::GuestExecStatus) -> guest::GuestInspectExecResponse {
    let mut response = guest::GuestInspectExecResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        ..Default::default()
    };
    response.set_status(status);
    response
}

#[test]
fn exec_status_and_wait_correlations_are_strict() {
    let mut cancelled = terminal::TerminalOutcome::new();
    cancelled.set_cancelled(terminal::TerminalCancelled::new());
    let contradictory = guest::GuestExecStatus {
        resource_handle: "exec-1".to_owned(),
        state: guest::GuestExecState::GUEST_EXEC_STATE_EXITED.into(),
        stdin_state: guest::GuestStdinState::GUEST_STDIN_STATE_CLOSED.into(),
        terminal_outcome: MessageField::some(cancelled),
        state_generation: 5,
        ..Default::default()
    };
    assert_eq!(
        inspect_status_response(contradictory).validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );
    let mut exited = terminal::TerminalOutcome::new();
    exited.set_exited(terminal::TerminalExited {
        exit_code: 0,
        ..Default::default()
    });
    let exited_status = guest::GuestExecStatus {
        resource_handle: "exec-1".to_owned(),
        state: guest::GuestExecState::GUEST_EXEC_STATE_EXITED.into(),
        stdin_state: guest::GuestStdinState::GUEST_STDIN_STATE_CLOSED.into(),
        terminal_outcome: MessageField::some(exited),
        state_generation: 5,
        ..Default::default()
    };
    inspect_status_response(exited_status.clone())
        .validate_wire(false)
        .unwrap();
    let mut open_stdin = exited_status;
    open_stdin.stdin_state = guest::GuestStdinState::GUEST_STDIN_STATE_OPEN.into();
    assert_eq!(
        inspect_status_response(open_stdin).validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut wait_query = guest::GuestInspectExecQuery::new();
    wait_query.set_wait(guest::GuestExecWaitQuery {
        resource_handle: "exec-1".to_owned(),
        known_state_generation: 5,
        timeout_ms: 100,
        ..Default::default()
    });
    let wait_request = guest::GuestInspectExecRequest {
        context: MessageField::some(context()),
        query: MessageField::some(wait_query),
        ..Default::default()
    };

    let mut timed_out = running_status();
    timed_out.state_generation = 5;
    timed_out.timed_out = true;
    validate_guest_inspect_response_for_request(
        &wait_request,
        &inspect_status_response(timed_out.clone()),
    )
    .unwrap();

    timed_out.state_generation = 4;
    assert_eq!(
        validate_guest_inspect_response_for_request(
            &wait_request,
            &inspect_status_response(timed_out.clone())
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    timed_out.state_generation = 6;
    assert_eq!(
        validate_guest_inspect_response_for_request(
            &wait_request,
            &inspect_status_response(timed_out.clone())
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    timed_out.timed_out = false;
    validate_guest_inspect_response_for_request(
        &wait_request,
        &inspect_status_response(timed_out.clone()),
    )
    .unwrap();
    timed_out.state_generation = 5;
    assert_eq!(
        validate_guest_inspect_response_for_request(
            &wait_request,
            &inspect_status_response(timed_out)
        ),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn retained_log_stream_is_server_allocated_and_query_bound() {
    use d2b_contracts::v2_services::TerminalFrameDirection;
    use terminal::terminal_stream_frame::Frame;

    let request = guest::GuestOpenExecRetainedLogRequest {
        context: MessageField::some(context()),
        resource_handle: "exec-1".to_owned(),
        output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
        offset: 4,
        max_bytes: 4,
        ..Default::default()
    };
    let mut response = terminal_response("exec-1");
    response.retained_log = MessageField::some(terminal::TerminalRetainedLogRange {
        output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
        requested_offset: 4,
        start_offset: 5,
        end_offset: 8,
        max_bytes: 4,
        eof: true,
        ..Default::default()
    });
    validate_guest_open_exec_retained_log_response_for_request(&request, &response).unwrap();

    for (outcome, kind) in [
        (
            common::Outcome::OUTCOME_DENIED,
            common::ErrorKind::ERROR_KIND_UNAUTHORIZED,
        ),
        (
            common::Outcome::OUTCOME_CANCELLED,
            common::ErrorKind::ERROR_KIND_CANCELLED,
        ),
        (
            common::Outcome::OUTCOME_FAILED,
            common::ErrorKind::ERROR_KIND_INTERNAL,
        ),
    ] {
        let closed = terminal::TerminalOpenResponse {
            outcome: outcome.into(),
            operation_id: "operation-1".to_owned(),
            session_generation: GENERATION,
            request_id: REQUEST_ID.to_vec(),
            error: MessageField::some(common::ErrorEnvelope {
                kind: kind.into(),
                retry: common::RetryClass::RETRY_CLASS_NEVER.into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        validate_guest_open_exec_retained_log_response_for_request(&request, &closed).unwrap();
        assert!(retained_log_stream_validator(&request, &closed).is_err());
    }

    let mut validator = retained_log_stream_validator(&request, &response).unwrap();
    let frame = |sequence, frame| terminal::TerminalStreamFrame {
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        sequence,
        operation_id: "operation-1".to_owned(),
        resource_handle: "exec-1".to_owned(),
        frame: Some(frame),
        ..Default::default()
    };

    let mut selection = terminal::TerminalSelection::new();
    selection.set_retained_log(terminal::RetainedLogSelection {
        exec_handle: "exec-1".to_owned(),
        output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
        offset: 4,
        max_bytes: 4,
        ..Default::default()
    });
    validator
        .accept(
            TerminalFrameDirection::ClientToServer,
            &frame(0, Frame::Select(selection)),
        )
        .unwrap();
    validator
        .accept(
            TerminalFrameDirection::ServerToClient,
            &frame(
                0,
                Frame::Started(terminal::TerminalStarted {
                    kind: terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG.into(),
                    stdout_offset: 5,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            TerminalFrameDirection::ServerToClient,
            &frame(
                1,
                Frame::Stdout(terminal::TerminalOutput {
                    offset: 5,
                    data: b"log".to_vec(),
                    eof: true,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    let mut outcome = terminal::TerminalOutcome::new();
    outcome.set_closed(terminal::TerminalClosed::new());
    validator
        .accept(
            TerminalFrameDirection::ServerToClient,
            &frame(2, Frame::Outcome(outcome)),
        )
        .unwrap();
    assert!(validator.is_terminal());

    let mut substituted = response.clone();
    substituted.retained_log.as_mut().unwrap().end_offset = 9;
    assert_eq!(
        validate_guest_open_exec_retained_log_response_for_request(&request, &substituted),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut wrong_selection = retained_log_stream_validator(&request, &response).unwrap();
    let mut selection = terminal::TerminalSelection::new();
    selection.set_retained_log(terminal::RetainedLogSelection {
        exec_handle: "exec-1".to_owned(),
        output: terminal::OutputStream::OUTPUT_STREAM_STDERR.into(),
        offset: 4,
        max_bytes: 4,
        ..Default::default()
    });
    assert_eq!(
        wrong_selection.accept(
            TerminalFrameDirection::ClientToServer,
            &frame(0, Frame::Select(selection))
        ),
        Err(ServiceContractError::InconsistentResponse)
    );
}

fn file_request() -> guest::GuestFileTransferRequest {
    guest::GuestFileTransferRequest {
        context: MessageField::some(context()),
        artifact: guest::GuestArtifactId::GUEST_ARTIFACT_ID_GUEST_CONFIG.into(),
        configured_intent_id: "guest-config".to_owned(),
        direction: guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST
            .into(),
        declared_size: 4,
        expected_digest: vec![0x77; 32],
        ..Default::default()
    }
}

fn file_frame(
    sequence: u64,
    frame: guest::guest_file_transfer_frame::Frame,
) -> guest::GuestFileTransferFrame {
    guest::GuestFileTransferFrame {
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        sequence,
        operation_id: "operation-1".to_owned(),
        resource_handle: "transfer-1".to_owned(),
        frame: Some(frame),
        ..Default::default()
    }
}

fn started_file_validator() -> FileTransferStreamValidator {
    use guest::guest_file_transfer_frame::Frame;
    let request = file_request();
    let mut validator =
        FileTransferStreamValidator::new(&request, &terminal_response("transfer-1")).unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                0,
                Frame::Start(guest::GuestFileTransferStart {
                    artifact: request.artifact,
                    configured_intent_id: request.configured_intent_id,
                    direction: request.direction,
                    declared_size: request.declared_size,
                    expected_digest: request.expected_digest,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
}

#[test]
fn file_transfer_frames_enforce_direction_offsets_size_digest_and_terminal_state() {
    use guest::guest_file_transfer_frame::Frame;
    let request = file_request();
    let response = terminal_response("transfer-1");
    validate_terminal_open_response_for_guest_context(request.context.as_ref().unwrap(), &response)
        .unwrap();
    let mut validator = FileTransferStreamValidator::new(&request, &response).unwrap();
    validator.accept_transport_credit(1024).unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                0,
                Frame::Start(guest::GuestFileTransferStart {
                    artifact: request.artifact,
                    configured_intent_id: request.configured_intent_id.clone(),
                    direction: request.direction,
                    declared_size: 4,
                    expected_digest: vec![0x77; 32],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                0,
                Frame::Credit(guest::GuestFileTransferCredit {
                    bytes: 4,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                1,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    data: b"data".to_vec(),
                    eof: true,
                    total_size: 4,
                    final_digest: vec![0x77; 32],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                1,
                Frame::Complete(guest::GuestFileTransferComplete {
                    total_size: 4,
                    digest: vec![0x77; 32],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert!(validator.is_terminal());
    validator.accept_transport_close().unwrap();
    validator.accept_transport_reset().unwrap();

    let mut no_credit = started_file_validator();
    assert_eq!(
        no_credit.accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                1,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    data: b"data".to_vec(),
                    eof: true,
                    total_size: 4,
                    final_digest: vec![0x77; 32],
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::BoundExceeded)
    );

    let mut over_credit = started_file_validator();
    over_credit
        .accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                0,
                Frame::Credit(guest::GuestFileTransferCredit {
                    bytes: 2,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert_eq!(
        over_credit.accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                1,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    data: b"data".to_vec(),
                    eof: true,
                    total_size: 4,
                    final_digest: vec![0x77; 32],
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::BoundExceeded)
    );

    let mut credit_overflow = started_file_validator();
    for sequence in 0..4 {
        credit_overflow
            .accept(
                GuestStreamDirection::ServerToClient,
                &file_frame(
                    sequence,
                    Frame::Credit(guest::GuestFileTransferCredit {
                        bytes: 64 * 1024,
                        ..Default::default()
                    }),
                ),
            )
            .unwrap();
    }
    assert_eq!(
        credit_overflow.accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                4,
                Frame::Credit(guest::GuestFileTransferCredit {
                    bytes: 1,
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::BoundExceeded)
    );

    let mut mismatched_completion = started_file_validator();
    mismatched_completion
        .accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                0,
                Frame::Credit(guest::GuestFileTransferCredit {
                    bytes: 4,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    mismatched_completion
        .accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                1,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    data: b"data".to_vec(),
                    eof: true,
                    total_size: 4,
                    final_digest: vec![0x77; 32],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert_eq!(
        mismatched_completion.accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                1,
                Frame::Complete(guest::GuestFileTransferComplete {
                    total_size: 4,
                    digest: vec![0x78; 32],
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut replay = started_file_validator();
    replay
        .accept(
            GuestStreamDirection::ServerToClient,
            &file_frame(
                0,
                Frame::Credit(guest::GuestFileTransferCredit {
                    bytes: 4,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    replay
        .accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                1,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    data: b"da".to_vec(),
                    total_size: 4,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert_eq!(
        replay.accept(
            GuestStreamDirection::ClientToServer,
            &file_frame(
                2,
                Frame::Chunk(guest::GuestFileTransferChunk {
                    offset: 0,
                    data: b"da".to_vec(),
                    total_size: 4,
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut oversized = file_frame(
        0,
        Frame::Chunk(guest::GuestFileTransferChunk {
            data: vec![0x55; 64 * 1024 + 1],
            total_size: 64 * 1024 + 1,
            ..Default::default()
        }),
    );
    assert_eq!(
        oversized.validate_wire(false),
        Err(ServiceContractError::BoundExceeded)
    );
    oversized.frame = None;
    assert_eq!(
        oversized.validate_wire(false),
        Err(ServiceContractError::MissingOperationInput)
    );
}

fn security_request() -> guest::GuestSecurityKeyRequest {
    guest::GuestSecurityKeyRequest {
        context: MessageField::some(context()),
        action: guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_START.into(),
        device_handle: "device-1".to_owned(),
        ceremony:
            guest::GuestSecurityKeyCeremonyKind::GUEST_SECURITY_KEY_CEREMONY_KIND_GET_ASSERTION
                .into(),
        approval_required: true,
        ..Default::default()
    }
}

fn security_frame(
    sequence: u64,
    frame: guest::guest_security_key_frame::Frame,
) -> guest::GuestSecurityKeyFrame {
    guest::GuestSecurityKeyFrame {
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        sequence,
        operation_id: "operation-1".to_owned(),
        resource_handle: "ceremony-1".to_owned(),
        frame: Some(frame),
        ..Default::default()
    }
}

fn opened_security_validator(
    request: &guest::GuestSecurityKeyRequest,
) -> SecurityKeyStreamValidator {
    use guest::guest_security_key_frame::Frame;
    let mut validator =
        SecurityKeyStreamValidator::new(request, &terminal_response("ceremony-1")).unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                0,
                Frame::Open(guest::GuestSecurityKeyOpen {
                    action: request.action,
                    device_handle: request.device_handle.clone(),
                    ceremony_handle: "ceremony-1".to_owned(),
                    ceremony: request.ceremony,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
}

#[test]
fn security_key_required_approval_cannot_be_bypassed() {
    use guest::guest_security_key_frame::Frame;
    let success = || {
        Frame::Complete(guest::GuestSecurityKeyComplete {
            outcome: guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_SUCCEEDED.into(),
            ..Default::default()
        })
    };
    let request = security_request();
    let mut immediate = opened_security_validator(&request);
    assert_eq!(
        immediate.accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(0, success())
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut denied = opened_security_validator(&request);
    denied
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                0,
                Frame::ApprovalRequest(guest::GuestSecurityKeyApprovalRequest {
                    approval: guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    denied
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                1,
                Frame::Approval(guest::GuestSecurityKeyApproval {
                    decision: guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_DENIED.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    for (direction, frame) in [
        (
            GuestStreamDirection::ServerToClient,
            security_frame(
                1,
                Frame::GuestReport(guest::GuestSecurityKeyReport {
                    report: vec![0x55; CTAPHID_REPORT_BYTES],
                    ..Default::default()
                }),
            ),
        ),
        (
            GuestStreamDirection::ClientToServer,
            security_frame(
                2,
                Frame::DeviceReport(guest::GuestSecurityKeyReport {
                    report: vec![0x66; CTAPHID_REPORT_BYTES],
                    ..Default::default()
                }),
            ),
        ),
        (
            GuestStreamDirection::ServerToClient,
            security_frame(
                1,
                Frame::ApprovalRequest(guest::GuestSecurityKeyApprovalRequest {
                    approval: guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE.into(),
                    ..Default::default()
                }),
            ),
        ),
        (
            GuestStreamDirection::ClientToServer,
            security_frame(
                2,
                Frame::Approval(guest::GuestSecurityKeyApproval {
                    decision: guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_APPROVED.into(),
                    ..Default::default()
                }),
            ),
        ),
    ] {
        assert_eq!(
            denied.accept(direction, &frame),
            Err(ServiceContractError::InconsistentResponse)
        );
    }
    assert_eq!(
        denied.accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(1, success())
        ),
        Err(ServiceContractError::InconsistentResponse)
    );
    denied
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                1,
                Frame::Complete(guest::GuestSecurityKeyComplete {
                    outcome: guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED
                        .into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert!(denied.is_terminal());

    let mut granted = opened_security_validator(&request);
    granted
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                0,
                Frame::ApprovalRequest(guest::GuestSecurityKeyApprovalRequest {
                    approval: guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    granted
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                1,
                Frame::Approval(guest::GuestSecurityKeyApproval {
                    decision: guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_APPROVED.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert_eq!(
        granted.accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                1,
                Frame::Complete(guest::GuestSecurityKeyComplete {
                    outcome: guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED
                        .into(),
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut no_approval_request = security_request();
    no_approval_request.approval_required = false;
    let mut not_required = opened_security_validator(&no_approval_request);
    assert_eq!(
        not_required.accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                0,
                Frame::Complete(guest::GuestSecurityKeyComplete {
                    outcome: guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED
                        .into(),
                    ..Default::default()
                })
            )
        ),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn security_key_frames_are_fixed_bounded_directional_and_terminal() {
    use guest::guest_security_key_frame::Frame;
    let request = security_request();
    let response = terminal_response("ceremony-1");
    let mut validator = SecurityKeyStreamValidator::new(&request, &response).unwrap();
    validator.accept_transport_credit(1024).unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                0,
                Frame::Open(guest::GuestSecurityKeyOpen {
                    action: request.action,
                    device_handle: request.device_handle.clone(),
                    ceremony_handle: "ceremony-1".to_owned(),
                    ceremony: request.ceremony,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                1,
                Frame::DeviceReport(guest::GuestSecurityKeyReport {
                    report: vec![0x55; CTAPHID_REPORT_BYTES],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                0,
                Frame::GuestReport(guest::GuestSecurityKeyReport {
                    report: vec![0x66; CTAPHID_REPORT_BYTES],
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                1,
                Frame::ApprovalRequest(guest::GuestSecurityKeyApprovalRequest {
                    approval: guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ClientToServer,
            &security_frame(
                2,
                Frame::Approval(guest::GuestSecurityKeyApproval {
                    decision: guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_APPROVED.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    validator
        .accept(
            GuestStreamDirection::ServerToClient,
            &security_frame(
                2,
                Frame::Complete(guest::GuestSecurityKeyComplete {
                    outcome: guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_SUCCEEDED
                        .into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();
    assert!(validator.is_terminal());
    validator.accept_transport_close().unwrap();

    let invalid = security_frame(
        0,
        Frame::GuestReport(guest::GuestSecurityKeyReport {
            report: vec![0; CTAPHID_REPORT_BYTES - 1],
            ..Default::default()
        }),
    );
    assert_eq!(
        invalid.validate_wire(false),
        Err(ServiceContractError::BoundExceeded)
    );
}

#[test]
fn shutdown_has_closed_accepted_and_final_shapes() {
    let request = guest::GuestShutdownRequest {
        context: MessageField::some(context()),
        action: guest::GuestPowerAction::GUEST_POWER_ACTION_POWER_OFF.into(),
        deadline_unix_ms: 4_000,
        ..Default::default()
    };
    let accepted = guest::GuestShutdownResponse {
        outcome: common::Outcome::OUTCOME_ACCEPTED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        phase: guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_ACCEPTED.into(),
        ..Default::default()
    };
    round_trip(&request, true);
    round_trip(&accepted, false);
    validate_guest_shutdown_response_for_request(&request, &accepted).unwrap();

    let final_response = guest::GuestShutdownResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        phase: guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL.into(),
        final_outcome: guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED
            .into(),
        ..Default::default()
    };
    validate_guest_shutdown_response_for_request(&request, &final_response).unwrap();
}

#[test]
fn generated_guest_debug_and_errors_are_redacted_and_unknown_fields_fail() {
    let request = file_request();
    assert_eq!(format!("{request:?}"), "GuestFileTransferRequest(REDACTED)");
    let frame = security_frame(
        0,
        guest::guest_security_key_frame::Frame::DeviceReport(guest::GuestSecurityKeyReport {
            report: vec![0x73; CTAPHID_REPORT_BYTES],
            ..Default::default()
        }),
    );
    assert_eq!(format!("{frame:?}"), "GuestSecurityKeyFrame(REDACTED)");
    assert!(!format!("{:?}", frame.frame).contains("115"));
    let file = file_frame(
        0,
        guest::guest_file_transfer_frame::Frame::Chunk(guest::GuestFileTransferChunk {
            data: b"private-file-bytes".to_vec(),
            total_size: 18,
            ..Default::default()
        }),
    );
    assert_eq!(format!("{file:?}"), "GuestFileTransferFrame(REDACTED)");
    assert!(!format!("{:?}", file.frame).contains("private-file-bytes"));

    let mut encoded = request.write_to_bytes().unwrap();
    encoded.extend_from_slice(&[0xf8, 0x07, 0x01]);
    assert_eq!(
        decode_strict::<guest::GuestFileTransferRequest>(&encoded, true),
        Err(ServiceContractError::UnknownField)
    );
}

#[test]
fn every_typed_guest_request_rejects_unknown_fields() {
    macro_rules! rejects_unknown {
        ($value:expr, $ty:ty, $idempotency:expr) => {{
            let mut encoded = $value.write_to_bytes().unwrap();
            encoded.extend_from_slice(&[0xf8, 0x07, 0x01]);
            assert_eq!(
                decode_strict::<$ty>(&encoded, $idempotency),
                Err(ServiceContractError::UnknownField)
            );
        }};
    }

    rejects_unknown!(
        guest::GuestBootstrapRequest {
            context: MessageField::some(context()),
            expected_generation: GENERATION,
            expected_parent_static_public_key_digest: vec![0x44; 32],
            ..Default::default()
        },
        guest::GuestBootstrapRequest,
        true
    );
    rejects_unknown!(
        guest::GuestReconnectRequest {
            context: MessageField::some(context()),
            expected_generation: GENERATION,
            guest_identity_handle: "guest-identity-1".to_owned(),
            expected_guest_static_public_key_digest: vec![0x55; 32],
            expected_parent_static_public_key_digest: vec![0x44; 32],
            ..Default::default()
        },
        guest::GuestReconnectRequest,
        true
    );
    let workload_terminal = || {
        terminal_request(common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            ..Default::default()
        })
    };
    rejects_unknown!(
        guest::GuestExecRequest {
            terminal: MessageField::some(workload_terminal()),
            ..Default::default()
        },
        guest::GuestExecRequest,
        true
    );
    rejects_unknown!(
        guest::GuestOpenShellRequest {
            terminal: MessageField::some(workload_terminal()),
            ..Default::default()
        },
        guest::GuestOpenShellRequest,
        true
    );
    rejects_unknown!(
        guest::GuestCancelExecRequest {
            context: MessageField::some(context()),
            resource_handle: "exec-1".to_owned(),
            control_sequence: 1,
            reason: guest::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED.into(),
            ..Default::default()
        },
        guest::GuestCancelExecRequest,
        true
    );
    rejects_unknown!(status_query(), guest::GuestInspectExecRequest, false);
    rejects_unknown!(
        guest::GuestOpenExecRetainedLogRequest {
            context: MessageField::some(context()),
            resource_handle: "exec-1".to_owned(),
            output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
            offset: 0,
            max_bytes: 4096,
            ..Default::default()
        },
        guest::GuestOpenExecRetainedLogRequest,
        true
    );
    rejects_unknown!(file_request(), guest::GuestFileTransferRequest, true);
    rejects_unknown!(security_request(), guest::GuestSecurityKeyRequest, true);
    rejects_unknown!(
        guest::GuestShutdownRequest {
            context: MessageField::some(context()),
            action: guest::GuestPowerAction::GUEST_POWER_ACTION_POWER_OFF.into(),
            deadline_unix_ms: 4_000,
            ..Default::default()
        },
        guest::GuestShutdownRequest,
        true
    );
}

#[test]
fn guest_proto_has_no_secret_path_map_or_json_escape_hatch() {
    let guest = include_str!("../proto/v2/guest.proto");
    let terminal = include_str!("../proto/v2/terminal.proto");
    for forbidden in [
        "psk",
        "private_key",
        "private key",
        "credential",
        " string path",
        "map<",
        "json",
        "environment",
        "string cwd",
    ] {
        assert!(!guest.contains(forbidden), "guest field: {forbidden}");
        assert!(!terminal.contains(forbidden), "terminal field: {forbidden}");
    }
    assert!(!guest.contains("OpenConsole"));
}
