//! Bounded workers for loader work whose kernel path has no async API.
//!
//! Bundle resolution and the host check read files, verify hashes, and (for
//! the host check) run `nft`/`systemctl` subprocesses. None of that has an
//! async form in this crate, and the daemon calls the loaders from async
//! handlers: running them inline parked a runtime worker for seconds per
//! call.
//!
//! The work runs on dedicated worker threads behind bounded queues, one seat
//! per class of work: [`run`] is the bundle-load seat and [`run_probe`] the
//! host-check probe seat. Both admit without blocking the caller's executor
//! and await the outcome; a saturated queue refuses with
//! [`LoaderRefusal::Busy`] instead of growing threads or parking the caller,
//! and a worker that never started refuses with [`LoaderRefusal::Unavailable`].
//!
//! The seats are separate on purpose. A probe can hang for good - an `nft`
//! or `systemctl` subprocess that never returns, or a read on a wedged
//! filesystem - and the jobs have no deadline, because a deadline on the
//! waiter cannot preempt a job already running on a serial worker. On one
//! shared seat that hung probe would hold the worker every bundle load
//! queues behind, so a single hung probe could starve the load path. On its
//! own seat it blocks only later probe jobs; bundle loads keep being
//! admitted and served.
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

/// The bound on admitted-but-unstarted loader jobs, per seat.
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

/// One dedicated worker thread with its own bounded queue.
struct LoaderWorker {
    sender: SyncSender<Job>,
}

/// Start one named worker with its own bounded queue.
///
/// `None` records a worker that could not start, so every later call refuses
/// with [`LoaderRefusal::Unavailable`] instead of retrying a failing spawn.
fn start_worker(name: &str) -> Option<LoaderWorker> {
    let (sender, receiver) = sync_channel::<Job>(MAX_LOADER_QUEUE_DEPTH);
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            // The sanctioned R4 channel boundary: a blocking `sync_channel`
            // recv on the worker's own dedicated thread, with
            // `tokio::sync::oneshot` replies (plan R4 / KTD3).
            #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
            while let Ok(job) = receiver.recv() {
                job();
            }
        })
        .ok()
        .map(|_| LoaderWorker { sender })
}

/// The bundle-load seat, started on first use.
static LOAD_WORKER: LazyLock<Option<LoaderWorker>> = LazyLock::new(|| start_worker("d2b-loader"));

/// The host-check probe seat, started on first use.
///
/// Separate from the load seat so a probe that never returns cannot occupy
/// the worker every bundle load queues behind.
static PROBE_WORKER: LazyLock<Option<LoaderWorker>> =
    LazyLock::new(|| start_worker("d2b-loader-probe"));

/// Run one bundle-load job on the bounded loader seat.
///
/// The caller's executor is never parked: admission is a non-blocking
/// `try_send` and the result is awaited from the worker.
pub async fn run<T, F>(job: F) -> Result<T, LoaderRefusal>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_on(&LOAD_WORKER, job).await
}

/// Run one host-check probe job on its own bounded seat.
///
/// The probe reads the bundle, host, and closure files and runs the
/// `nft`/`systemctl` checks; a hung probe occupies only this seat, so it can
/// refuse later probes with [`LoaderRefusal::Busy`] but never starve bundle
/// loads.
pub async fn run_probe<T, F>(job: F) -> Result<T, LoaderRefusal>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_on(&PROBE_WORKER, job).await
}

async fn run_on<T, F>(
    worker: &'static LazyLock<Option<LoaderWorker>>,
    job: F,
) -> Result<T, LoaderRefusal>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (reply, outcome) = tokio::sync::oneshot::channel();
    let admitted = worker
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
