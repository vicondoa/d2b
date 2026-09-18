//! The bounded seat for nested kernel legs.
//!
//! [`d2b_contracts_broker::kernel_client::envelope_invoke_kernel`] is a
//! blocking seqpacket RPC over the broker's origination socket with no async
//! form in the tree. Running it inline from an async handler would park the
//! executor worker for the whole leg, and `tokio::task::spawn_blocking`
//! would reach the runtime's unbounded blocking pool once per call (plan KD2
//! ban).
//!
//! The seat is a fixed set of dedicated threads - never a growing pool - so
//! admission can never pick the process's thread count. Each worker owns its
//! own bounded `std::sync::mpsc::sync_channel` (plan R4's channel boundary: a
//! blocking `recv` on the worker's own thread) and answers through
//! `tokio::sync::oneshot`; admission is a non-blocking `try_send` scan, so a
//! caller is refused rather than queued when every worker is saturated.
//!
//! One worker is not enough. The nested legs fan out from concurrent family
//! handlers - every controller/worker launch and adoption probe issues its own
//! `open-pidfd`/`spawn-process`/`observe-process`/`deregister-pidfd` leg - and
//! a single seat serializes them: in the host-integration wave sixteen legs in
//! flight behind one worker put the tail leg past the broker's 10s io budget
//! on every retry, so no launch ever converged. The fan-out is bounded by the
//! number of concurrently reconciled process rows, which is what
//! [`WORKERS`] times [`QUEUE_PER_WORKER`] sizes the seat for.
//!
//! A saturated seat refuses with [`KernelRefusal::Busy`] (retryable) and a
//! seat that never started refuses with [`KernelRefusal::Unavailable`].
//!
//! The seat is job-shape agnostic: the process family seats its
//! `envelope_invoke_kernel` round trips and the network family its own, both
//! behind the same bounded worker set. The jobs it carries are opaque
//! closures, so it holds no broker type and lives beside the daemon
//! [`crate::loader_worker`] seat it is shaped after.

use std::{
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread,
};

use tokio::sync::oneshot;


/// Dedicated worker threads behind the seat.
///
/// The nested legs are cheap RPC round trips whose concurrency is bounded by
/// the number of process rows reconciling at once; eight workers keep that
/// wave under the broker's per-leg io budget instead of serializing it.
pub const WORKERS: usize = 8;

/// Jobs one worker admits before its callers are refused.
pub const QUEUE_PER_WORKER: usize = 4;

/// Why a kernel job was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelRefusal {
    /// Every worker's bounded queue is full: the caller must retry rather
    /// than wait.
    Busy,
    /// The seat is not running.
    Unavailable,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The fixed worker set, one bounded queue per worker.
struct KernelSeat {
    senders: Vec<SyncSender<Job>>,
    next: AtomicUsize,
}

impl KernelSeat {
    /// Start every worker. `None` records a seat that could not start, so
    /// every later call refuses with [`KernelRefusal::Unavailable`] instead of
    /// retrying a failing spawn.
    fn start() -> Option<Self> {
        let mut senders = Vec::with_capacity(WORKERS);
        for index in 0..WORKERS {
            let (sender, receiver) = sync_channel::<Job>(QUEUE_PER_WORKER);
            thread::Builder::new()
                .name(format!("d2b-kernel-invoker-{index}"))
                .spawn(move || {
                    // The sanctioned R4 channel boundary: a blocking
                    // `sync_channel` recv on the worker's own dedicated
                    // thread, with `tokio::sync::oneshot` replies.
                    #[allow(
                        clippy::disallowed_methods,
                        reason = "dedicated bounded worker per plan R4"
                    )]
                    while let Ok(job) = receiver.recv() {
                        job();
                    }
                })
                .ok()?;
            senders.push(sender);
        }
        Some(Self {
            senders,
            next: AtomicUsize::new(0),
        })
    }

    /// Admit one job on the first worker with room, starting at the
    /// round-robin cursor so one busy worker cannot pin every later caller to
    /// its queue. `try_send` hands the job back on refusal, so the same job is
    /// offered to each worker in turn and the seat only refuses once every
    /// worker's queue is full.
    fn admit(&self, job: Job) -> Result<(), KernelRefusal> {
        let start = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let mut pending = job;
        let mut saw_full = false;
        let mut saw_disconnected = false;
        for offset in 0..self.senders.len() {
            let sender = &self.senders[(start + offset) % self.senders.len()];
            match sender.try_send(pending) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    saw_full = true;
                    pending = returned;
                }
                Err(TrySendError::Disconnected(returned)) => {
                    saw_disconnected = true;
                    pending = returned;
                }
            }
        }
        // A live-but-saturated worker is retryable backpressure; a seat with
        // no live worker at all is gone.
        drop(pending);
        if saw_full || !saw_disconnected {
            Err(KernelRefusal::Busy)
        } else {
            Err(KernelRefusal::Unavailable)
        }
    }
}

/// The kernel-invocation seat, started on first use. An invocation uses a
/// per-job socket, so the seat is safe to share across zones and families;
/// the kernel leg's io timeout bounds each job on its worker.
static KERNEL_SEAT: LazyLock<Option<KernelSeat>> = LazyLock::new(KernelSeat::start);

/// Run one kernel invocation job on the bounded seat.
///
/// The caller's executor is never parked: admission is a non-blocking
/// `try_send`, and the result is awaited from the worker.
pub async fn run<T, F>(job: F) -> Result<T, KernelRefusal>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (reply, outcome) = oneshot::channel();
    let seat = KERNEL_SEAT.as_ref().ok_or(KernelRefusal::Unavailable)?;
    seat.admit(Box::new(move || {
        // A panicking job drops the reply sender, so the waiter sees
        // `Unavailable` instead of hanging on a dead worker.
        let _ = reply.send(job());
    }))?;
    outcome.await.map_err(|_| KernelRefusal::Unavailable)
}
