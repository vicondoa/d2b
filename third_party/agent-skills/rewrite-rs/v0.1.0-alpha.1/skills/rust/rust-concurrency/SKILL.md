---
name: rust-concurrency
description: Pick the concurrency model from the workload shape — rayon for data parallelism, scoped threads for borrowed stack data, channels for handoff, shared state last — and use the weakest correct atomic ordering. Use when writing or reviewing threaded Rust, when Mutex, RwLock, atomics, or manual Send/Sync appear, when a deadlock or data race is suspected, or when the user asks how to parallelize Rust code.
---

# Rust Concurrency

The shape of the workload picks the model. This skill owns that pick for
threads, and it stops where task concurrency begins — runtime, executors,
`spawn_blocking`, and cancellation live in `/async-rust`.

## The shape picks the model

- The same operation over many independent items: data parallelism, `rayon`.
- Independent units of work that are mostly waiting: task concurrency,
  `/async-rust`, not here.
- State several threads read and write: shared state — the one to reach for
  last, because it is the only one of the three that can deadlock.

## Data parallelism

`par_iter()` is a one-word change to an iterator chain that already exists —
exactly why the chain was worth writing. The precondition: the per-item work
has to be large enough to pay for the scheduling. A `par_iter` over a million
cheap closures is often slower, and is the standard disappointment.

## Scoped threads

`std::thread::scope` lets a thread borrow stack data, because the scope
guarantees the join before the stack unwinds — which is what removes the
`'static` bound that otherwise forces an `Arc`:

```rust
fn sum_halves(data: &[u64]) -> u64 {
    let (left, right) = data.split_at(data.len() / 2);
    std::thread::scope(|s| {
        let a = s.spawn(|| left.iter().sum::<u64>());
        let b = s.spawn(|| right.iter().sum::<u64>());
        a.join().unwrap() + b.join().unwrap()
    })
}
```

Reach for a scope before an `Arc` or a clone buys the second thread.

## Channels for handoff

A channel moves ownership with the message — no lock to hold, nothing to
poison. Sender-drop is the shutdown signal: `recv` reports disconnection.
Bounded versus unbounded is backpressure: an unbounded channel silently
converts a throughput problem into a memory problem.

## Shared state

`Mutex` is the default; `RwLock` pays off only with genuinely read-heavy
access, and can starve writers. A `Mutex` whose holder panicked hands back
an `Err` — propagate it, or `into_inner()` with a written reason, never a
reflexive `unwrap`. Hold the guard for the shortest possible scope, and
never across a call into code that might lock again.

## Atomics and orderings

The rule: `Relaxed` for a counter nobody synchronises on, `Acquire`/`Release`
for a paired handoff where one thread publishes and another observes,
`SeqCst` when unsure and the cost is acceptable. The pair is the one that is
wrong most often:

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn main() {
    let value = Arc::new(AtomicU64::new(0));
    let ready = Arc::new(AtomicBool::new(false));
    let (v, r) = (Arc::clone(&value), Arc::clone(&ready));
    let publisher = std::thread::spawn(move || {
        v.store(42, Ordering::Relaxed);
        r.store(true, Ordering::Release);
    });
    while !ready.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    assert_eq!(value.load(Ordering::Relaxed), 42);
    publisher.join().unwrap();
}
```

The `Release` store orders the write to `value` before `ready` flips, and
the `Acquire` load pairs with it. If the ordering argument cannot be
written down in two sentences, the code wants a `Mutex`.

## Manual `Send`/`Sync` is an unsafe claim

`unsafe impl Send for T` asserts what the compiler could not prove; the
`// SAFETY:` comment must hold an actual argument. The soundness review
goes to `/unsafe-rust` — this skill only insists the claim is written.

## `thread_local!` over `static mut`

`static mut` is effectively unusable in current editions and was never
sound to share; `thread_local!` is the per-thread answer. Passing state
beats hiding it in a global — a global is a lifetime nobody wrote down.

## Deferrals

Task concurrency, executors, `spawn_blocking`, and cancellation are
`/async-rust`. Whether the state should be shared at all is
`/ownership-not-clone`. The soundness of an `unsafe impl` is `/unsafe-rust`.
Throughput of the parallel version is `/rust-performance`.

## Verification

```bash
cargo test                    # including the ignored-by-default stress tests, if the repo has any
cargo clippy --all-targets    # at the level the repo configures
cargo miri test               # where atomics or unsafe Send/Sync are involved
```

Clippy runs at the lint level the target repo configures; `-D warnings`
is the fallback only when the repo configures no lints at all. Lock-free
structures and non-trivial ordering arguments want `loom` — it explores
interleavings exhaustively rather than hoping the scheduler picks the
bad one; it runs under its own cfg, and the model has to be small.
