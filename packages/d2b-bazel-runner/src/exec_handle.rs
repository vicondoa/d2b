use d2b_bazel_exec::{
    ExecutionBackend, ExecutionRequest, ExecutionResult, HandoffError, VerifiedExecutable,
};

/// Pass an already verified executable to the dependency-leaf consumer.
///
/// The runner does not resolve a path or select a helper. The execution crate
/// owns descriptor mapping, stdio setup, and the immutable consumer boundary.
pub fn execute<B: ExecutionBackend>(
    executable: VerifiedExecutable,
    request: ExecutionRequest,
    backend: &B,
) -> Result<ExecutionResult, HandoffError> {
    d2b_bazel_exec::execute_verified(executable, request, backend)
}
