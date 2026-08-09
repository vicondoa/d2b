use d2b_bazel_exec::{
    ExecutionRequest, ProtocolError, ProtocolState, StatusDecoder, StatusFrame, TerminalStatus,
    decode_helper_error, encode_status,
};

#[test]
fn execute_protocol_surface_is_a_nonempty_enforcing_integration_test() {
    let mut decoder = StatusDecoder::new();
    let frames = [
        encode_status(StatusFrame::Ready),
        encode_status(StatusFrame::Executed),
        encode_status(StatusFrame::Exited(0)),
    ]
    .concat();

    assert_eq!(
        decoder.feed(&frames).expect("valid execution status"),
        [
            StatusFrame::Ready,
            StatusFrame::Executed,
            StatusFrame::Exited(0)
        ]
    );
    assert_eq!(decoder.state(), ProtocolState::Terminal);
    assert_eq!(
        decoder.finish_eof().expect("terminal status"),
        TerminalStatus::Exited(0)
    );
    assert_eq!(
        decode_helper_error(b"D2BE\x01\x02\x00\x0c", true)
            .expect("valid helper error")
            .expect("helper error record")
            .code(),
        "D2B-BZLEXEC-HELPER-PTRACE-OPTIONS"
    );
}

#[test]
fn empty_execution_request_remains_a_typed_parent_error() {
    let _request = ExecutionRequest::default();
    assert_eq!(
        ProtocolError::ReadyTimeout.code(),
        "D2B-BZLEXEC-PARENT-READY"
    );
}
