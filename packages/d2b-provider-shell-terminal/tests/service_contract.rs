use d2b_provider_shell_terminal::{
    AttachRequest, CONTROLLER_SERVICE, SUPERVISOR_SERVICE, TERMINAL_STREAM,
};

#[test]
fn service_names_and_terminal_stream_are_closed_contracts() {
    assert_eq!(CONTROLLER_SERVICE, "shell-terminal.v3");
    assert_eq!(SUPERVISOR_SERVICE, "shell-session-supervisor.v1");
    assert_eq!(TERMINAL_STREAM, "terminal");
    assert!(AttachRequest::new(1, 4096).is_ok());
    assert!(AttachRequest::new(1, 1_048_577).is_err());
}
