use d2b_bazel_exec::{ExecutionRequest, ExecutionResult, HandoffError, VerifiedExecutable};

/// Pass an already verified executable to the dependency-leaf consumer.
///
/// The runner does not resolve a path or select a helper. The execution crate
/// owns descriptor mapping, stdio setup, and the immutable consumer boundary.
pub fn execute(
    executable: VerifiedExecutable,
    request: ExecutionRequest,
) -> Result<ExecutionResult, HandoffError> {
    d2b_bazel_exec::execute_verified(executable, request)
}
