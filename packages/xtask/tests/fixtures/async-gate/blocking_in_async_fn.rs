//! Fixture for the async-discipline gate (U13, KTD9): blocking calls inside
//! async contexts.
//!
//! Every `std::` call in the five async contexts below runs on a Tokio
//! runtime worker and must be flagged with the named violation
//! `blocking-call-in-async-context`. Since U13 the `spawn_blocking` call AND
//! its argument body are flagged too (KD2 bans the thread-per-call shape, so
//! the body is no longer exempt); only the synchronous function runs off the
//! worker and must not be flagged, and a comment mention must not flag either.

use std::future::Future;
use std::path::Path;

/// A handler that blocks the worker: `std::fs::read` parks the executor
/// thread for the whole read.
pub async fn load_config(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// A worker that sleeps: `std::thread::sleep` stalls every task on the
/// worker for the full duration.
pub async fn backoff() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}

/// A lock taken on the worker: `std::sync::Mutex::lock` parks the thread
/// until another thread releases the guard.
pub async fn touch(shared: &std::sync::Mutex<u32>) {
    let _guard = std::sync::Mutex::lock(shared).unwrap();
}

/// The same lock through the production method-call shape (U6, KTD4):
/// `shared.lock()` is invisible to the qualified path the deny list names,
/// so the conservative method-call shape must flag it.
pub async fn touch_method(shared: &std::sync::Mutex<u32>) {
    let _guard = shared.lock().unwrap();
}

/// An async block on the worker: the `async { }` body runs on the runtime.
pub fn write_free() -> impl Future<Output = ()> {
    async {
        std::fs::write("/tmp/out", b"x").unwrap();
    }
}

/// The banned thread-per-call adapter (plan KD2): the call itself and the
/// `std::fs::read` inside its argument body must BOTH be flagged since U13.
pub async fn via_spawn_blocking(path: &Path) -> Vec<u8> {
    tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .unwrap_or_default()
}

/// A synchronous path: not a runtime worker, must NOT be flagged.
pub fn sync_loader(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// A comment-only mention must NOT flag: `std::fs::read` here is prose, and
/// the call that follows is the sanctioned `tokio::fs::read`.
pub async fn documented() {
    // std::fs::read is the blocking form; tokio::fs::read is not.
    tokio::fs::read("/tmp/x").await.unwrap_or_default();
}