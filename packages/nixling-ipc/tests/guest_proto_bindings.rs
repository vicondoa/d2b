use nixling_ipc::guest_proto as pb;
use nixling_ipc::guest_wire::{
    GUEST_CONTROL_PROTOCOL_VERSION, GUEST_CONTROL_VSOCK_PORT, HARD_MAX_CHUNK_BYTES,
};
use protobuf::{EnumOrUnknown, Message, MessageField};

fn round_trip<M>(message: M)
where
    M: Message + Default + PartialEq + std::fmt::Debug,
{
    let encoded = message.write_to_bytes().expect("message encodes");
    let decoded = M::parse_from_bytes(&encoded).expect("message decodes");
    assert_eq!(decoded, message);
}

#[test]
fn generated_health_round_trips_with_guest_wire_constants() {
    let mut health = pb::HealthResponse::new();
    health.origin = EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
    health.state = EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
    health.reason = EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
    health.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
    health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
    health.capabilities.push(EnumOrUnknown::new(
        pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
    ));

    round_trip(health);
}

#[test]
fn generated_hello_and_capabilities_shapes_compile() {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = "corp-vm".to_owned();
    metadata.request_id = "req-1".to_owned();
    metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut hello = pb::HelloRequest::new();
    hello.metadata = MessageField::some(metadata.clone());
    hello.host_nonce = vec![1; 32];
    hello.transcript_version = 1;
    round_trip(hello);

    let mut hello_response = pb::HelloResponse::new();
    hello_response.guest_nonce = vec![2; 32];
    hello_response.guest_boot_id = "boot-id".to_owned();
    hello_response.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
    round_trip(hello_response);

    let mut auth = pb::AuthenticateRequest::new();
    auth.metadata = MessageField::some(metadata.clone());
    auth.host_nonce = vec![1; 32];
    auth.guest_nonce = vec![2; 32];
    auth.guest_boot_id = "boot-id".to_owned();
    auth.transcript_version = 1;
    auth.host_auth_tag = vec![3; 32];
    round_trip(auth);

    let mut health = pb::HealthResponse::new();
    health.origin = EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
    health.state = EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
    health.reason = EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
    health.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
    health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut limits = pb::GuestEffectiveLimits::new();
    limits.max_chunk_bytes = HARD_MAX_CHUNK_BYTES;
    limits.max_recv_message_bytes = 4 * 1024 * 1024;

    let mut caps = pb::CapabilitiesResponse::new();
    caps.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
    caps.capabilities.push(EnumOrUnknown::new(
        pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
    ));
    caps.limits = MessageField::some(limits);
    round_trip(caps.clone());

    let mut authenticated = pb::AuthenticateResponse::new();
    authenticated.guest_auth_tag = Some(vec![4; 32]);
    authenticated.capabilities_hash = Some("capabilities-sha256".to_owned());
    authenticated.health = MessageField::some(health);
    authenticated.capabilities = MessageField::some(caps);
    round_trip(authenticated);
}

#[test]
fn generated_exec_message_shapes_compile_without_service_stubs() {
    let mut common = pb::RequestMetadata::new();
    common.vm_id = "corp-vm".to_owned();
    common.request_id = "req-2".to_owned();
    common.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut exec_metadata = pb::ExecRequestMetadata::new();
    exec_metadata.common = MessageField::some(common.clone());
    exec_metadata.exec_id = "exec-1".to_owned();
    exec_metadata.guest_boot_id = "boot-1".to_owned();

    let mut create = pb::ExecCreateRequest::new();
    create.metadata = MessageField::some(common);
    create.argv = vec!["/bin/true".to_owned()];
    create.user = Some("alice".to_owned());
    create.tty = false;
    round_trip(create);

    let mut logs = pb::ReadOutputRequest::new();
    logs.metadata = MessageField::some(exec_metadata.clone());
    logs.stream = EnumOrUnknown::new(pb::OutputStream::OUTPUT_STREAM_STDOUT);
    logs.max_len = HARD_MAX_CHUNK_BYTES;
    round_trip(logs);

    let mut status = pb::TerminalStatus::new();
    status.outcome = Some(pb::terminal_status::Outcome::ExitCode(0));
    round_trip(status);

    let mut signal = pb::ExecSignalRequest::new();
    signal.metadata = MessageField::some(exec_metadata);
    signal.signal = 15;
    signal.target = EnumOrUnknown::new(pb::SignalTarget::SIGNAL_TARGET_PROCESS_TREE);
    round_trip(signal);
}

#[test]
fn generated_transport_schema_constants_match_guest_wire() {
    assert_eq!(GUEST_CONTROL_VSOCK_PORT, 14_318);
    assert_eq!(
        pb::GuestCapability::GUEST_CAPABILITY_HEALTH as i32,
        1,
        "generated enum discriminant changed"
    );
    assert_eq!(
        pb::HealthState::HEALTH_STATE_HEALTHY as i32,
        1,
        "generated health enum discriminant changed"
    );
}

#[test]
fn generated_exec_expired_error_kind_matches_guest_wire() {
    // The additive `ExecExpired` wire kind is value 37 and must stay in
    // lockstep with the `guest_wire::GuestControlErrorKind` enum.
    assert_eq!(
        pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_EXPIRED as i32,
        37,
        "generated ExecExpired discriminant changed"
    );
    assert_eq!(
        serde_json::to_string(&nixling_ipc::guest_wire::GuestControlErrorKind::ExecExpired)
            .unwrap(),
        "\"exec-expired\"",
    );
}

#[test]
fn generated_exec_list_shapes_round_trip() {
    let mut common = pb::RequestMetadata::new();
    common.vm_id = "corp-vm".to_owned();
    common.request_id = "req-list".to_owned();
    common.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut request = pb::ExecListRequest::new();
    request.metadata = MessageField::some(common);
    request.guest_boot_id = "boot-1".to_owned();
    round_trip(request);

    let mut entry = pb::ExecListEntry::new();
    entry.exec_id = "00112233".to_owned();
    entry.slot = 7;
    entry.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_RUNNING);
    entry.create_time_unix = 1_700_000_000;
    entry.argv_sha256 = "a".repeat(64);
    entry.stdout_truncated = true;
    entry.stderr_truncated = false;
    entry.dropped_bytes = 4096;

    let mut response = pb::ExecListResponse::new();
    response.entries.push(entry);
    round_trip(response);
}
