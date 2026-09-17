//! Bounded worker for the nested kernel leg.
//!
//! [`d2b_contracts_broker::kernel_client::envelope_invoke_kernel`] is a
//! blocking seqpacket RPC over the broker's origination socket with no async
//! form in the tree. Running it inline from an async handler would park the
//! executor worker for the whole leg, and `tokio::task::spawn_blocking` would
//! reach the runtime's unbounded blocking pool once per call (plan KD2
//! ban). A single dedicated bounded worker thread serves every nested kernel
//! leg in this crate, exactly the loader_worker shape: a blocking
//! `std::sync::mpsc::sync_channel` recv on the worker's own dedicated thread
//! (plan R4), `tokio::sync::oneshot` replies, non-blocking `try_send`
//! admission so a caller can never grow the pool. A saturated queue
//! refuses with [`KernelRefusal::Busy`] and a worker that never started or
//! died refuses with [`KernelRefusal::Unavailable`].

use std::{
    sync::{
        LazyLock,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
};

use tokio::sync::oneshot;

use d2b_contracts_broker::kernel_client::{KernelInvokeError, KernelReply};

/// The bound on admitted-but-unstarted kernel jobs per seat.
pub const MAX_KERNEL_QUEUE_DEPTH: usize = 16;

/// Why a kernel job was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelRefusal {
    /// The bounded queue is full: the caller must retry rather than wait.
    Busy,
    /// The kernel worker is not running.
    Unavailable,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// One dedicated worker thread with its own bounded queue.
struct KernelWorker {
    sender: SyncSender<Job>,
}

/// Start the worker with its own bounded queue; `None` records a worker that
/// could not start, so every later call refuses with
/// [`KernelRefusal::Unavailable`] instead of retrying a failing spawn.
fn start_worker() -> Option<KernelWorker> {
    let (sender, receiver) = sync_channel::<Job>(MAX_KERNEL_QUEUE_DEPTH);
    thread::Builder::new()
        .name("d2b-kernel-invoker".into())
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
        .map(|_| KernelWorker { sender })
}

/// The kernel-invocation seat, started on first use. An invocation uses a
/// per-job socket, so the seat is safe to share across zones; the kernel
/// leg's io timeout bounds each job on the worker.
static KERNEL_WORKER: LazyLock<Option<KernelWorker>> = LazyLock::new(start_worker);

/// Run one kernel invocation job on the bounded kernel seat.
///
/// The caller's executor is never parked: admission is a non-blocking
/// `try_send` and the result is awaited from the worker.
pub async fn run<F>(job: F) -> Result<Result<KernelReply, KernelInvokeError>, KernelRefusal>
where
    F: FnOnce() -> Result<KernelReply, KernelInvokeError> + Send + 'static,
{
    let (reply, outcome) = oneshot::channel();
    let admitted = KERNEL_WORKER
        .as_ref()
        .ok_or(KernelRefusal::Unavailable)?
        .sender
        .try_send(Box::new(move || {
            // A panicking job drops the reply sender, so the waiter sees
            // `Unavailable` instead of hanging on a dead worker.
            let _ = reply.send(job());
        }));
    match admitted {
        Ok(()) => outcome.await.map_err(|_| KernelRefusal::Unavailable),
        Err(TrySendError::Full(_)) => Err(KernelRefusal::Busy),
        Err(TrySendError::Disconnected(_)) => Err(KernelRefusal::Unavailable),
    }
}