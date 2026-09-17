use std::future::Future;
use std::os::fd::{OwnedFd, RawFd};
use std::sync::LazyLock;

use tokio::runtime::{Handle, Runtime};

use crate::typed_error::TypedError;
use crate::unix_transport::duplicate_fd_cloexec;
use sha2::Digest;

pub fn duplicate_received_fd(
    received_fds: &[RawFd],
    fd_index: u32,
    context: &str,
) -> Result<OwnedFd, TypedError> {
    let Some(fd_slot) = usize::try_from(fd_index)
        .ok()
        .filter(|index| *index < received_fds.len())
    else {
        return Err(TypedError::InternalIo {
            context: context.to_owned(),
            detail: format!("missing SCM_RIGHTS fd at index {fd_index}"),
        });
    };
    duplicate_fd_cloexec(received_fds[fd_slot], context)
}

/// The process-wide runtime for synchronous seats that have no runtime.
///
/// The daemon owns the runtime it serves on; every synchronous seat that
/// drives async work should borrow that one ([`block_on_future_with`]).
/// This seat exists for the sync entry points written before a runtime was
/// in scope, and it is started once per process rather than built per call:
/// a runtime per call spawns a worker set and a driver per call, and those
/// threads then contend with the zone the call is serving. Reaching here is
/// recorded, so a seat that still needs the daemon's runtime is visible
/// instead of silent.
static FALLBACK_RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tracing::warn!(
        "synchronous seat without an ambient tokio runtime: driving async work on the \
         process-wide fallback runtime; give this seat the daemon's runtime instead"
    );
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("d2b-block-on-fallback")
        .build()
        .expect("build the fallback tokio runtime")
});

/// Drive one future to completion on a runtime the caller names.
///
/// This is the seat a synchronous caller takes when it holds the daemon's
/// runtime: the future is driven on that runtime from the calling thread, so
/// no runtime is built and no worker is parked behind an unrelated one.
#[allow(clippy::disallowed_methods, reason = "deleted at U15")]
pub fn block_on_future_with<T>(runtime: &Handle, future: impl Future<Output = T>) -> T {
    runtime.block_on(future)
}

/// Drive one future to completion from a synchronous seat.
///
/// The seat is explicit about which runtime drives the future, because
/// synchronous code driving async work is the thing this crate keeps out of
/// the async path:
///
/// - inside a runtime, the future is driven on the ambient runtime from a
///   blocking section, so the parked thread is that runtime's own (on a
///   current-thread runtime there is no worker to park and this is a hard
///   error; the caller is the one that has to become async);
/// - with no ambient runtime, the future runs on the process-wide
///   `FALLBACK_RUNTIME`, not on a runtime built for this call.
#[allow(clippy::disallowed_methods, reason = "deleted at U15")]
pub fn block_on_future<T>(future: impl Future<Output = T>) -> T {
    match Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => FALLBACK_RUNTIME.block_on(future),
    }
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn projection_digest_bytes(value: &str) -> Option<[u8; 32]> {
    (!value.is_empty()).then(|| sha2::Sha256::digest(value.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The no-runtime seat reuses one runtime per process.
    ///
    /// A runtime built per call ends with the call, so a task spawned onto it
    /// from a synchronous seat never reaches its own completion; the
    /// process-wide fallback keeps running and does.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "deleted at U15")]
    fn a_synchronous_seat_without_a_runtime_reuses_one_fallback_runtime() {
        let (done, completed) = std::sync::mpsc::channel();
        block_on_future(async move {
            tokio::spawn(async move {
                let _ = done.send(());
            });
        });
        completed
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the fallback runtime outlives the call that spawned onto it");
    }

    /// A seat that holds a runtime drives its future on that one.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "deleted at U15")]
    fn a_seat_that_names_a_runtime_uses_it() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        assert_eq!(block_on_future_with(runtime.handle(), async { 7 }), 7);
    }
}
