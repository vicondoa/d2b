#![forbid(unsafe_code)]

pub mod execute;
pub mod provider;

pub use execute::{
    BackendError, CHILD_STAGE_CODES, ChildIdentity, ContainmentBackend, ExecErrorRecord, ExecStop,
    ExecutionBackend, ExecutionRequest, ExecutionResult, GroupIdentity, HandoffError, InitialStop,
    LaunchCoordinator, MaskSnapshot, PTRACE_EVENT_EXEC, ProductionBackend, ProtocolError,
    ProtocolFrame, ProtocolReader, ProtocolState, RUST_PARENT_STAGE_CODES, SUPERVISOR_STAGE_CODES,
    SpawnReceipt, StatusDecoder, StatusFrame, StatusWriteError, StdioPolicy, SupervisorProtocol,
    TerminalStatus, classify_status_write, decode_exec_error, encode_status, execute_verified,
    helper_exit_before_executed, managed_signals, run_signal_handoff,
};
pub use provider::{
    ExecErrno, ProviderError, VerifiedExecutable, classify_exec_error, verify_provider,
};
