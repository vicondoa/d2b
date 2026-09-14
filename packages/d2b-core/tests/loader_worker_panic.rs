//! The loader worker's death refusal.
//!
//! This scenario gets its own test binary on purpose: the worker is a
//! process-wide singleton, and a panicking job unwinds out of the worker's job
//! loop, so its thread is gone for the rest of the process. Sharing a binary
//! with the admission tests in `loader_worker.rs` would make them depend on
//! test order.

use std::{
    future::Future,
    pin::pin,
    sync::mpsc,
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

use d2b_core::loader_worker::{self, LoaderRefusal};

/// Longest a refusal may take before the test declares the caller stranded.
const REFUSAL_DEADLINE: Duration = Duration::from_secs(10);

/// Drive a future with no executor, reactor, or timer: a no-op waker and a
/// hand-rolled poll loop, as the other tests in this workspace do.
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::yield_now(),
        }
    }
}

/// Await a `run` future on a scratch thread: a panicking job that stranded its
/// waiter would be a wedged daemon, so it must fail this test with a diagnostic
/// instead of hanging the suite.
fn await_with_deadline<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(block_on(future));
    });
    receiver
        .recv_timeout(REFUSAL_DEADLINE)
        .expect("the panicking job must refuse its waiter instead of hanging it")
}

/// A job that panics refuses its waiter with the named `Unavailable` refusal
/// instead of hanging it, and the death it leaves behind keeps refusing by that
/// same name.
#[test]
fn panicked_job_refuses_with_named_unavailable_refusal() {
    assert_eq!(
        await_with_deadline(loader_worker::run(|| -> usize {
            panic!("loader job under test panicked");
        })),
        Err(LoaderRefusal::Unavailable),
        "a panicked job must refuse with the named Unavailable refusal, not hang"
    );

    // The worker thread unwound with the job, so later calls reach a dead
    // worker: they must refuse by the same name, must not hang, and must not
    // report the queue-full `Busy` refusal.
    assert_eq!(
        block_on(loader_worker::run(|| 1usize)),
        Err(LoaderRefusal::Unavailable),
        "a dead worker must refuse with the named Unavailable refusal"
    );

    // The load seat's death must not take the probe seat with it: the host
    // check keeps running while the bundle-load seat is gone.
    assert_eq!(
        block_on(loader_worker::run_probe(|| 9usize)),
        Ok(9),
        "the probe seat must keep serving after the load seat dies"
    );
}
