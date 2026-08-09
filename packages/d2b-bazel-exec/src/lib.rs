#![forbid(unsafe_code)]

pub mod execute;
pub mod provider;

pub use execute::{
    CHILD_STAGE_CODES, ChildIdentity, ChildStage, ContainmentBackend, ExecErrorRecord, ExecStop,
    ExecutionRequest, ExecutionResult, GroupIdentity, HELPER_ERROR_CODES, HandoffError,
    HelperStage, InitialStop, LaunchCoordinator, MaskSnapshot, PTRACE_EVENT_EXEC, ProtocolError,
    ProtocolFrame, ProtocolReader, ProtocolState, RUST_PARENT_STAGE_CODES, SUPERVISOR_ENVIRONMENT,
    SUPERVISOR_STAGE_CODES, StatusDecoder, StatusFrame, StatusWriteError, StdioPolicy,
    SupervisorProtocol, TerminalStatus, classify_status_write, decode_exec_error,
    decode_helper_error, encode_status, execute_verified, helper_exit_before_executed,
    managed_signals,
};
pub use provider::{ExecErrno, ProviderError, VerifiedExecutable, classify_exec_error};
