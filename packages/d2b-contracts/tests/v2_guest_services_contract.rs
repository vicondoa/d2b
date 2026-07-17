#![cfg(feature = "v2-services")]

use d2b_contracts::v2_services::{
    CTAPHID_REPORT_BYTES, FileTransferStreamValidator, GuestStreamDirection,
    SecurityKeyStreamValidator, ServiceContractError, StrictWireMessage, common, decode_strict,
    encode_strict, guest, terminal, validate_guest_cancel_response_for_request,
    validate_guest_inspect_response_for_request, validate_guest_session_response_for_bootstrap,
    validate_guest_session_response_for_reconnect, validate_guest_shutdown_response_for_request,
    validate_terminal_open_response_for_guest_context,
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
fn retained_log_stream_is_server_allocated_and_query_bound() {
    let mut query = guest::GuestInspectExecQuery::new();
    query.set_open_retained_log(guest::GuestExecOpenRetainedLogQuery {
        resource_handle: "exec-1".to_owned(),
        output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
        offset: 4,
        max_bytes: 4096,
        ..Default::default()
    });
    let request = guest::GuestInspectExecRequest {
        context: MessageField::some(context()),
        query: MessageField::some(query),
        ..Default::default()
    };
    let mut response = guest::GuestInspectExecResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-1".to_owned(),
        session_generation: GENERATION,
        request_id: REQUEST_ID.to_vec(),
        ..Default::default()
    };
    response.set_retained_log_stream(guest::GuestExecRetainedLogStream {
        stream: MessageField::some(terminal_response("exec-1")),
        output: terminal::OutputStream::OUTPUT_STREAM_STDOUT.into(),
        start_offset: 4,
        end_offset: 8,
        ..Default::default()
    });
    validate_guest_inspect_response_for_request(&request, &response).unwrap();
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
