//! Contract tests for the bounded loader worker.
//!
//! Every test here is a plain `#[test]`: the worker must be callable from a
//! thread with no ambient runtime, so [`block_on`] drives each future with a
//! no-op-waker poll loop and nothing in this file constructs a runtime. A `run`
//! that reached for `tokio::task::spawn_blocking` (or `Handle::current`) could
//! not pass here, and one that needed a reactor or timer to wake its waiter
//! would never complete.
//!
//! The workers are process-wide singletons and each seat's queue is shared by
//! every caller, so a test that parks a worker or fills a queue holds
//! [`worker_lock`] for as long as it occupies queue space.

use std::{
    future::Future,
    pin::Pin,
    sync::{Mutex, MutexGuard, mpsc},
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use d2b_core::loader_worker::{self, LoaderRefusal, MAX_LOADER_QUEUE_DEPTH};
use d2b_core::test_support::block_on;

/// Longest a refusal may take before the test declares the caller parked.
const REFUSAL_DEADLINE: Duration = Duration::from_secs(10);

/// The queue is process-wide: without this lock a test that parks a worker or
/// fills a queue would make a concurrent test's job be refused as `Busy`.
static WORKER_LOCK: Mutex<()> = Mutex::new(());

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn worker_lock() -> MutexGuard<'static, ()> {
    WORKER_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Await a `run` future on a scratch thread: a regression that parks the caller
/// (a blocking send, a waiter stranded on a dead worker) then fails this test
/// with a diagnostic instead of wedging the whole suite.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn await_with_deadline<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(block_on(future));
    });
    receiver
        .recv_timeout(REFUSAL_DEADLINE)
        .expect("the loader worker parked its caller instead of answering")
}

/// Poll a `run` future once. `run` is an `async fn`, so a job is admitted on the
/// first poll; polling once is how the queue is filled without awaiting the
/// queued jobs.
fn poll_noop<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}

/// Releases the parked job even when an assertion unwinds first, so a failing
/// test cannot leave the shared worker thread parked for the next one.
struct ReleaseOnDrop(mpsc::Sender<()>);

impl Drop for ReleaseOnDrop {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// The value of a finished job comes back to a caller that has no runtime to
/// park and no executor to re-enter.
#[test]
fn successful_job_returns_its_value_without_an_ambient_runtime() {
    let _lock = worker_lock();
    assert_eq!(
        await_with_deadline(loader_worker::run(|| 7usize)),
        Ok(7),
        "a job's value must come back to a bare awaiting thread"
    );
}

/// Work leaves the caller's thread and lands on the one dedicated worker thread
/// the worker promises - not the awaiting thread, and not a thread per call.
#[test]
fn jobs_run_on_one_dedicated_worker_thread() {
    let _lock = worker_lock();
    let caller = thread::current().id();
    let where_it_ran = || {
        (
            thread::current().id(),
            thread::current().name().map(str::to_owned),
        )
    };
    let (first_thread, first_name) =
        await_with_deadline(loader_worker::run(where_it_ran)).expect("first job admitted");
    let (second_thread, second_name) =
        await_with_deadline(loader_worker::run(where_it_ran)).expect("second job admitted");
    assert_ne!(
        first_thread, caller,
        "the job must not run on the awaiting thread"
    );
    // The worker thread cannot exit while this binary still holds the worker's
    // sender, so a repeated id is a repeated thread and not a recycled one.
    assert_eq!(
        (first_thread, first_name),
        (second_thread, second_name),
        "both jobs must run on the same named worker thread, not one thread per call"
    );
}

/// A probe job parked in `nft`/`systemctl` - or on a wedged filesystem - holds
/// only the probe seat: a bundle load admitted after it still runs, and the
/// probe seat's own queue refuses probes past its bound instead of letting a
/// hung probe starve the load path.
#[test]
fn hung_probe_job_does_not_starve_bundle_loads() {
    let _lock = worker_lock();

    // Park the probe seat inside one job: this is the hung `nft`/`systemctl`
    // probe. On one shared seat the load queue would sit behind it forever.
    let (started_sender, started) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let release_guard = ReleaseOnDrop(release_sender);
    let mut parked = Box::pin(loader_worker::run_probe(move || {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        started_sender
            .send((
                thread::current().id(),
                thread::current().name().map(str::to_owned),
            ))
            .expect("signal the parked probe");
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        release.recv().expect("await the release signal");
        "parked probe"
    }));
    assert!(
        poll_noop(parked.as_mut()).is_pending(),
        "the probe job must be admitted and left waiting for the probe worker"
    );
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    let (probe_thread, probe_name) = started
        .recv_timeout(REFUSAL_DEADLINE)
        .expect("the probe worker must start the parked probe job");

    // The probe is still hung: the load seat must admit and complete a job
    // anyway, on its own dedicated thread.
    let (load_thread, load_name) = await_with_deadline(loader_worker::run(|| {
        (
            thread::current().id(),
            thread::current().name().map(str::to_owned),
        )
    }))
    .expect("a bundle load must be admitted while the probe seat is hung");
    assert_ne!(
        (probe_thread, probe_name),
        (load_thread, load_name),
        "the probe and the bundle load must run on different seats, not one shared worker"
    );

    // The probe seat refuses past its own bound rather than growing: one hung
    // probe starves later probe jobs at worst, never the bundle loads.
    let mut queued = Vec::with_capacity(MAX_LOADER_QUEUE_DEPTH);
    for depth in 0..MAX_LOADER_QUEUE_DEPTH {
        let mut job = Box::pin(loader_worker::run_probe(move || depth));
        assert!(
            poll_noop(job.as_mut()).is_pending(),
            "the probe queue admits exactly MAX_LOADER_QUEUE_DEPTH jobs"
        );
        queued.push(job);
    }
    assert_eq!(
        await_with_deadline(loader_worker::run_probe(|| MAX_LOADER_QUEUE_DEPTH)),
        Err(LoaderRefusal::Busy),
        "the probe over its bound must refuse with the named Busy refusal"
    );
    assert_eq!(
        await_with_deadline(loader_worker::run(|| "load behind a refused probe")),
        Ok("load behind a refused probe"),
        "a refused probe must not refuse bundle loads"
    );

    drop(release_guard);
    for (depth, job) in queued.into_iter().enumerate() {
        assert_eq!(
            block_on(job).expect("queued probe completes once the probe worker drains"),
            depth
        );
    }
    assert_eq!(
        block_on(parked).expect("parked probe completes once released"),
        "parked probe"
    );
}

/// A saturated queue refuses with the named `Busy` refusal rather than parking
/// the caller, and the refusal is transient: once the queue drains, the same
/// call is admitted again.
#[test]
fn full_queue_refuses_with_named_busy_refusal() {
    let _lock = worker_lock();

    // Park the worker inside one job so the queue below holds exactly the
    // admitted-but-unstarted jobs and the worker cannot drain it behind them.
    let (started_sender, started) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let release_guard = ReleaseOnDrop(release_sender);
    let mut parked = Box::pin(loader_worker::run(move || {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        started_sender.send(()).expect("signal the parked job");
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        release.recv().expect("await the release signal");
        "parked"
    }));
    assert!(
        poll_noop(parked.as_mut()).is_pending(),
        "the parked job must be admitted and left waiting for the worker"
    );
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    started
        .recv_timeout(REFUSAL_DEADLINE)
        .expect("the worker must start the parked job");

    let mut queued = Vec::with_capacity(MAX_LOADER_QUEUE_DEPTH);
    for depth in 0..MAX_LOADER_QUEUE_DEPTH {
        let mut job = Box::pin(loader_worker::run(move || depth));
        assert!(
            poll_noop(job.as_mut()).is_pending(),
            "the queue admits exactly MAX_LOADER_QUEUE_DEPTH jobs"
        );
        queued.push(job);
    }

    assert_eq!(
        await_with_deadline(loader_worker::run(|| MAX_LOADER_QUEUE_DEPTH)),
        Err(LoaderRefusal::Busy),
        "the job over the bound must refuse with the named Busy refusal, not wait"
    );

    drop(release_guard);
    for (depth, job) in queued.into_iter().enumerate() {
        assert_eq!(
            block_on(job).expect("queued job completes once the worker drains"),
            depth
        );
    }
    assert_eq!(
        block_on(parked).expect("parked job completes once released"),
        "parked"
    );
    assert_eq!(
        await_with_deadline(loader_worker::run(|| MAX_LOADER_QUEUE_DEPTH)),
        Ok(MAX_LOADER_QUEUE_DEPTH),
        "the refusal is transient: the drained queue must admit jobs again"
    );
}
