//! The CLI's process runtime and its explicit sync/async boundary.
//!
//! Parsing stays synchronous: clap owns argv, help, and completions, and none
//! of that is on an async path. Execution is asynchronous: the transport is
//! [`crate::context::CliSocket`], readiness over the same non-blocking
//! seqpacket descriptor the CLI has always spoken its envelope on;
//! interactive terminal input is readiness-driven on a duplicate of stdin;
//! and every network-shaped wait is bounded and named. A command body is still
//! a synchronous function - it owns the terminal and prints the result - so it
//! crosses the boundary exactly once per transport unit through [`block_on`].
//! Nothing below that boundary may block on I/O: the futures drive readiness
//! through the runtime's reactor, and the only thing a command body blocks on
//! is "run this future to completion here".

use std::{future::Future, sync::LazyLock};
use tokio::runtime::{Builder, Handle, Runtime};

/// The process runtime.
///
/// One current-thread reactor, created on first use: a CLI has a single
/// command in flight, so worker threads would idle, and a `--help`-shaped
/// invocation pays nothing for a runtime it never drives. `enable_all` arms
/// the timer and I/O drivers the transport and the interactive loop need.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the CLI runtime must build")
});

/// Drive `future` to completion on the process runtime.
///
/// This is the sync/async boundary. It must not be called from inside a
/// future the runtime is already driving (that would panic); teardown paths
/// that can run there check [`inside_runtime`] first.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.block_on(future)
}

/// Whether the caller is already executing inside the process runtime.
pub(crate) fn inside_runtime() -> bool {
    Handle::try_current().is_ok()
}
