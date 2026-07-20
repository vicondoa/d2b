use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn assert_contains_all(source: &str, required: &[&str]) {
    for needle in required {
        assert!(source.contains(needle), "missing final contract {needle:?}");
    }
}

#[test]
fn fixed_user_manager_endpoints_use_component_session_activation() {
    let user = read("nixos-modules/user-services.nix");
    assert_contains_all(
        &user,
        &[
            "systemd.user.sockets.d2b-userd",
            "ListenSequentialPacket = \"/run/d2b/u/%U/userd.sock\"",
            "FileDescriptorName = \"user-agent\"",
            "SocketMode = \"0600\"",
            "Service = \"d2b-userd.service\"",
        ],
    );
    let user_service = read("packages/d2b-userd/src/services/user/mod.rs");
    assert_contains_all(
        &user_service,
        &[
            "pub const SERVICE_PACKAGE: &str = \"d2b.user.v2\"",
            "pub const ENDPOINT_PURPOSE: &str = \"user-agent\"",
            "pub const ENDPOINT_ROLE: &str = \"user-agent\"",
        ],
    );

    let runtime = read("nixos-modules/unsafe-local-helper.nix");
    assert_contains_all(
        &runtime,
        &[
            "systemd.user.sockets.d2b-runtime-systemd-user",
            "ListenSequentialPacket = \"/run/d2b/u/%U/runtime-agent.sock\"",
            "FileDescriptorName = \"runtime-systemd-user\"",
            "SocketMode = \"0600\"",
            "Service = \"d2b-runtime-systemd-user.service\"",
        ],
    );

    let activation = read("packages/d2b-session-unix/src/systemd.rs");
    assert_contains_all(
        &activation,
        &[
            "LISTEN_PID",
            "LISTEN_FDS",
            "LISTEN_FDNAMES",
            "AddressFamily::UNIX",
            "SocketType::SEQPACKET",
            "get_socket_acceptconn",
            "FdFlags::CLOEXEC",
        ],
    );
    let server = read("packages/d2b-session/src/server.rs");
    assert_contains_all(
        &server,
        &[
            "pub async fn serve_ttrpc_services(",
            ".receive_ttrpc()",
            ".send_ttrpc(frame)",
            "MAX_LOGICAL_MESSAGE_BYTES",
        ],
    );

    let user_entrypoint = read("packages/d2b-userd/src/main.rs");
    assert_contains_all(
        &user_entrypoint,
        &[
            "d2b_userd::runtime::run_production()",
            "process::exit(error.exit_code())",
            "Some(_) => process::exit(78)",
        ],
    );
    let user_runtime = read("packages/d2b-userd/src/services/user/runtime.rs");
    assert_contains_all(
        &user_runtime,
        &[
            "pub async fn run_production() -> Result<(), UserdRuntimeError>",
            "ActivatedSeqpacketListeners::from_systemd(&[LISTENER_NAME])",
            "Self::Composition | Self::Activation => 78,",
        ],
    );

    let runtime_entrypoint = read("packages/d2b-unsafe-local-helper/src/main.rs");
    assert_contains_all(
        &runtime_entrypoint,
        &[
            "d2b_unsafe_local_helper::server::run().await",
            "std::process::exit(1)",
        ],
    );
    let runtime_server = read("packages/d2b-unsafe-local-helper/src/server.rs");
    assert_contains_all(
        &runtime_server,
        &[
            "pub async fn run() -> Result<(), ServerError>",
            "ActivatedSeqpacketListeners::from_systemd(&[ACTIVATED_LISTENER_NAME])",
        ],
    );
}

#[test]
fn wayland_open_display_accepts_exact_authenticated_descriptors() {
    let service = read("packages/d2b-wayland-proxy/src/services/wayland/mod.rs");
    assert_contains_all(
        &service,
        &[
            "let [upstream, listener] = request.descriptors.as_slice() else",
            "upstream.validate(0, DescriptorPurpose::Wayland, request)?",
            "listener.validate(1, DescriptorPurpose::Listener, request)",
            "self.request_id != request.request_id",
            "self.operation_id != request.operation_id",
            "self.reconnect_generation != request.session_generation",
            "self.method_id != OPEN_DISPLAY_METHOD_ID",
            "self.credit_classes != DESCRIPTOR_CREDIT_CLASSES",
        ],
    );

    let entrypoint = read("packages/d2b-wayland-proxy/src/main.rs");
    assert_contains_all(
        &entrypoint,
        &[
            "fcntl_dupfd_cloexec(std::io::stdin(), 3)",
            "fcntl_dupfd_cloexec(std::io::stdout(), 3)",
            "ControlMethod::OpenDisplay",
        ],
    );
}

#[test]
fn clipboard_composition_owns_generation_bound_transfer_descriptors() {
    let services = read("packages/d2b-clipd/src/services/mod.rs");
    assert_contains_all(
        &services,
        &[
            "validate_session(",
            "bridge_session.generation() != generation",
            "picker_session.generation() != generation",
            "picker_session.attachments_present()",
            "transfer_fds: BTreeMap<TransferHandle, OwnedFd>",
            "validate_component_session_transfer_fd",
            "PickerConfirmationRequired",
            "SessionUnavailable",
        ],
    );

    let picker = read("packages/d2b-clipd/src/services/picker/mod.rs");
    assert_contains_all(
        &picker,
        &[
            "pub const SERVICE_PACKAGE: &str = \"d2b.clipboard.picker.v2\"",
            "pub const ENDPOINT_PURPOSE: &str = \"clipboard-picker\"",
            "if attachments_present",
            "PickerServiceError::AttachmentDenied",
        ],
    );
}

#[test]
fn security_key_reports_use_an_attachment_free_named_stream() {
    let service = read("packages/d2b-sk-frontend/src/services/security_key/mod.rs");
    assert_contains_all(
        &service,
        &[
            "pub const SERVICE_PACKAGE: &str = \"d2b.security-key.v2\"",
            "pub const ENDPOINT_PURPOSE: &str = \"security-key\"",
            "pub const ENDPOINT_ROLE: &str = \"security-key-frontend\"",
            "AttachmentPolicyKind::Disabled",
            "max_per_session: 0",
            "open_named_stream(stream, REPORT_STREAM_CREDIT, REPORT_STREAM_CREDIT)",
            "send_named_stream(self.stream, report.to_vec())",
            "grant_named_stream_credit(self.stream, CTAPHID_REPORT_LEN as u32)",
        ],
    );

    let transport = read("packages/d2b-sk-frontend/src/vsock.rs");
    assert_contains_all(
        &transport,
        &[
            "if !packet.attachments().is_empty()",
            "TransportError::InvalidAttachment",
            "supports_attachments: false",
        ],
    );
}

#[test]
fn retained_runtime_helper_requires_same_uid_authenticated_authority() {
    let runtime = read("packages/d2b-unsafe-local-helper/src/services/runtime_systemd_user/mod.rs");
    assert_contains_all(
        &runtime,
        &[
            "pub const SERVICE_PACKAGE: &str = \"d2b.runtime.systemd-user.v2\"",
            "pub const ENDPOINT_PURPOSE: &str = \"runtime-systemd-user\"",
            "pub const ENDPOINT_ROLE: &str = \"runtime-systemd-user-agent\"",
            "if !established.is_authenticated()",
            "uid != established.process_uid()",
            "uid != nix::unistd::getuid().as_raw()",
            "pub struct AuthenticatedTerminalAttachment",
            "pub connected_stream: bool",
            "pub cloexec: bool",
        ],
    );
}
