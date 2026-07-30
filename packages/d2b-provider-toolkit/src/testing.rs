//! Shared test helpers for Provider crates and their conformance suites.
//!
//! These helpers are hermetic by construction: nothing here opens a socket,
//! touches a filesystem, waits on wall time, or starts a runtime.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Drive a future to completion on the calling thread.
///
/// Every Provider effect port in this workspace is an async seam whose test
/// double is immediately ready, so a conformance suite needs a driver but
/// not an async runtime. Each Provider crate previously carried a private
/// copy of this function; this is that driver, once.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_future_completes_on_the_calling_thread() {
        assert_eq!(block_on(async { 7_u8 }), 7);
    }
}
