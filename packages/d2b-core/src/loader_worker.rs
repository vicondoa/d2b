//! Bounded worker for loader work whose kernel path has no async API.
//!
//! Bundle resolution and the host check read files, verify hashes, and (for
//! the host check) run `nft`/`systemctl` subprocesses. None of that has an
//! async form in this crate, and the daemon calls the loaders from async
//! handlers: running them inline parked a runtime worker for seconds per
//! call.
//!
//! The work now runs on one dedicated worker thread behind a bounded queue.
//! [`run`] admits without blocking the caller's executor and awaits the
//! outcome; a saturated queue refuses with [`LoaderRefusal::Busy`] instead of
//! growing threads or parking the caller, and a worker that never started
//! refuses with [`LoaderRefusal::Unavailable`].
//!
//! `tokio::task::spawn_blocking` is deliberately not used: it reaches the
//! runtime's shared blocking pool once per call, which is the thread-per-call
//! shape this replaces, and the pool is only as bounded as the runtime's own
//! configuration.

use std::{
    sync::{
        LazyLock,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
};

/// The bound on admitted-but-unstarted loader jobs.
pub const MAX_LOADER_QUEUE_DEPTH: usize = 16;

/// Why a loader job was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderRefusal {
    /// The bounded queue is full: the caller must retry rather than wait.
    Busy,
    /// The loader worker is not running.
    Unavailable,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

struct LoaderWorker {
    sender: SyncSender<Job>,
}

/// The single loader worker, started on first use.
///
/// `None` records a worker that could not start, so every later call refuses
/// with [`LoaderRefusal::Unavailable`] instead of retrying a failing spawn.
static WORKER: LazyLock<Option<LoaderWorker>> = LazyLock::new(|| {
    let (sender, receiver) = sync_channel::<Job>(MAX_LOADER_QUEUE_DEPTH);
    thread::Builder::new()
        .name("d2b-loader".to_owned())
        .spawn(move || {
            while let Ok(job) = receiver.recv() {
                job();
            }
        })
        .ok()
        .map(|_| LoaderWorker { sender })
});

/// Run one loader job on the bounded blocking worker.
///
/// The caller's executor is never parked: admission is a non-blocking
/// `try_send` and the result is awaited from the worker.
pub async fn run<T, F>(job: F) -> Result<T, LoaderRefusal>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (reply, outcome) = tokio::sync::oneshot::channel();
    let admitted = WORKER
        .as_ref()
        .ok_or(LoaderRefusal::Unavailable)?
        .sender
        .try_send(Box::new(move || {
            // A panicking job drops the reply sender, so the waiter sees
            // `Unavailable` instead of hanging on a dead worker.
            let _ = reply.send(job());
        }));
    match admitted {
        Ok(()) => outcome.await.map_err(|_| LoaderRefusal::Unavailable),
        Err(TrySendError::Full(_)) => Err(LoaderRefusal::Busy),
        Err(TrySendError::Disconnected(_)) => Err(LoaderRefusal::Unavailable),
    }
}
