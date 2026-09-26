//! Concurrency primitives for the public-socket accept loop.
//!
//! Two independent concerns live here, both extracted behind small,
//! hermetically-testable types so the accept-loop and dispatch paths in
//! `lib.rs` stay thin:
//!
//! 1. `ConnSemaphore` - a non-blocking, bounded admission gate for
//!    in-flight connection-handler threads. The accept loop performs a
//!    NON-blocking `ConnSemaphore::try_acquire`; on a miss it refuses
//!    the connection immediately (typed-busy) instead of ever blocking
//!    `accept()`. The returned `ConnPermit` is an RAII token that is
//!    moved INTO the handler thread (and, for an attached exec session,
//!    into the owner closure) so the slot is released exactly when the
//!    handler - not the accept loop - finishes.
//!
//! 2. `OpLockManager` - per-VM and global in-process locks so a
//!    mutating lifecycle op (vm start/stop/restart, …) cannot race
//!    another op on the same VM, and a global op (host prepare, keys
//!    rotate, …) is mutually exclusive with every per-VM op. Read-only
//!    verbs take no lock and run fully in parallel. The single lock
//!    ordering (global-read THEN per-VM) is acyclic, so per-VM and
//!    global ops never deadlock. The lock is acquired ONCE at the
//!    dispatch boundary and held across the whole op (DAG + rollback +
//!    cleanup); inner stop/start helpers invoked by restart/rollback do
//!    NOT re-acquire it, so there is no nested self-deadlock.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

pub const DEFAULT_MAX_INFLIGHT_CONNECTIONS: usize = 64;

pub fn resolve_max_inflight_connections() -> usize {
    std::env::var("D2BD_MAX_INFLIGHT_CONNECTIONS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|cap| *cap > 0)
        .unwrap_or(DEFAULT_MAX_INFLIGHT_CONNECTIONS)
}

/// Non-blocking, bounded admission gate for connection-handler threads.
///
/// Cheaply [`Clone`]able (shared atomic counter behind an `Arc`) so it
/// can live inside the `Clone` `ServerState`.
#[derive(Debug, Clone)]
pub struct ConnSemaphore {
    in_flight: Arc<AtomicUsize>,
    cap: usize,
}

/// RAII permit released on drop. Moved into the handler thread so the
/// in-flight slot is held for the lifetime of the handler, not the
/// accept loop.
#[derive(Debug)]
pub struct ConnPermit {
    in_flight: Arc<AtomicUsize>,
}

impl ConnSemaphore {
    /// Create a semaphore admitting at most `cap` concurrent handlers.
    /// A `cap` of zero is clamped to one so the daemon can always make
    /// forward progress on at least one connection.
    pub fn new(cap: usize) -> Self {
        Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            cap: cap.max(1),
        }
    }

    /// Try to reserve a slot WITHOUT blocking. Returns `None` when the
    /// cap is already saturated so the accept loop can refuse the
    /// connection immediately rather than block.
    pub fn try_acquire(&self) -> Option<ConnPermit> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.cap {
                return None;
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(ConnPermit {
                        in_flight: Arc::clone(&self.in_flight),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Lock class for a request, derived from its verb. Read-only verbs take
/// no lock; per-VM mutating verbs serialize on the named VM; global
/// mutating verbs are mutually exclusive with everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpLockClass {
    /// No lock - read-only / status / session-managed verbs.
    ReadOnly,
    /// Per-VM mutating verb; serialized against other ops on this VM.
    PerVm(String),
    /// Global mutating verb; mutually exclusive with all per-VM ops.
    Global,
}

/// Per-VM + global in-process op locks. Cheaply [`Clone`]able (all state
/// behind `Arc`) so it can live inside the `Clone` `ServerState`.
///
/// The locks are `tokio::sync` primitives (async purity, plan U17).
/// Production dispatch runs on dedicated `d2b-conn` handler threads, so
/// `acquire` parks them on the blocking seats; the `--once` serve path
/// dispatches inline on the accept loop's runtime worker, where those seats
/// panic, so `acquire` keeps the bounded try-lock spin there instead.
#[derive(Debug, Clone, Default)]
pub struct OpLockManager {
    /// A global op takes the write side (exclusive with every per-VM op);
    /// a per-VM op takes the read side (shared) plus its own per-VM lock.
    global: Arc<RwLock<()>>,
    per_vm: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

/// RAII guard for a held op lock. Holds the tokio guards so the lock is
/// released when the guard drops at the end of the op. The global guard
/// borrows the manager (tokio's `RwLock` has no owned blocking seat); the
/// per-VM guard is owned.
#[allow(dead_code)]
pub enum OpLockGuard<'a> {
    /// Read-only verb: nothing is held.
    None,
    /// Per-VM verb: shared-global guard + the per-VM exclusive guard.
    PerVm {
        global: tokio::sync::RwLockReadGuard<'a, ()>,
        vm: OwnedMutexGuard<()>,
    },
    /// Global verb: exclusive-global guard.
    Global(tokio::sync::RwLockWriteGuard<'a, ()>),
}

impl OpLockManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the lock appropriate to `class`. The op lock spans a
    /// synchronous critical section (never an await) and is held for the
    /// whole op, so the wait matters.
    ///
    /// Off a runtime the acquisition takes the tokio blocking seats, which
    /// park the calling thread; that is what production dispatch wants,
    /// because every production connection is handled on its own dedicated
    /// `d2b-conn` thread. The blocking seats panic on a runtime worker, and
    /// the `--once` serve path dispatches its single connection inline on
    /// the accept loop's runtime worker, so there the wait keeps the repo's
    /// `lock_sync` bounded try-lock spin (`authority_persistence` and the
    /// Zone activation guard take the same shape). The critical sections
    /// are single map ops, holders never await, and the single lock ordering
    /// is acyclic, so the wait is bounded and deadlock-free on either seat.
    pub fn acquire(&self, class: &OpLockClass) -> OpLockGuard<'_> {
        let blocking = tokio::runtime::Handle::try_current().is_err();
        match class {
            OpLockClass::ReadOnly => OpLockGuard::None,
            OpLockClass::PerVm(vm) => {
                // Lock ordering: global(read) THEN per-VM. A global op
                // takes global(write), so it cannot interleave with an
                // in-flight per-VM op, and the single ordering is acyclic.
                let global = wait_for_op_lock(
                    blocking,
                    || self.global.try_read().ok(),
                    || self.global.blocking_read(),
                );
                let vm_lock = {
                    let mut map = wait_for_op_lock(
                        blocking,
                        || self.per_vm.try_lock().ok(),
                        || self.per_vm.blocking_lock(),
                    );
                    Arc::clone(
                        map.entry(vm.clone())
                            .or_insert_with(|| Arc::new(Mutex::new(()))),
                    )
                };
                let vm = wait_for_op_lock(
                    blocking,
                    || vm_lock.clone().try_lock_owned().ok(),
                    || vm_lock.clone().blocking_lock_owned(),
                );
                OpLockGuard::PerVm { global, vm }
            }
            OpLockClass::Global => OpLockGuard::Global(wait_for_op_lock(
                blocking,
                || self.global.try_write().ok(),
                || self.global.blocking_write(),
            )),
        }
    }
}

/// Wait for one op-lock seat.
///
/// `blocking` names the seat `OpLockManager::acquire` chose: the blocking
/// seat parks the calling thread when the caller does not drive a runtime,
/// and the bounded try-lock spin (the `lock_sync` shape,
/// `authority_persistence`) carries the wait on a runtime worker, where
/// tokio's blocking seats panic. The free fast path runs first, so an
/// uncontended lock is taken without either wait.
fn wait_for_op_lock<T>(
    blocking: bool,
    try_acquire: impl Fn() -> Option<T>,
    blocking_acquire: impl FnOnce() -> T,
) -> T {
    if let Some(guard) = try_acquire() {
        return guard;
    }
    if blocking {
        return blocking_acquire();
    }
    loop {
        match try_acquire() {
            Some(guard) => return guard,
            None => std::hint::spin_loop(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::sync_channel;

    use super::*;

    #[test]
    fn semaphore_admits_up_to_cap_then_refuses() {
        let sem = ConnSemaphore::new(2);
        let p1 = sem.try_acquire().expect("first permit");
        let p2 = sem.try_acquire().expect("second permit");
        assert!(
            sem.try_acquire().is_none(),
            "cap-hit must refuse, not block"
        );
        drop(p1);
        let _p3 = sem.try_acquire().expect("slot freed after drop");
        drop(p2);
    }

    #[test]
    fn semaphore_cap_zero_clamps_to_one() {
        let sem = ConnSemaphore::new(0);
        let _p = sem.try_acquire().expect("at least one slot");
        assert!(sem.try_acquire().is_none());
    }

    #[test]
    fn semaphore_permit_released_on_handler_thread_exit() {
        let sem = ConnSemaphore::new(1);
        let permit = sem.try_acquire().expect("permit");
        // Two-barrier handshake: the handler signals (gate) only after the
        // permit is moved in, then HOLDS it until the main thread releases
        // it (release) after asserting - no timing window.
        let gate = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let handler_gate = Arc::clone(&gate);
        let handler_release = Arc::clone(&release);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                // The permit is owned by (and dropped at the end of) the
                // handler thread, mirroring the accept-loop move.
                let _moved = permit;
                handler_gate.wait();
                handler_release.wait();
            });
            gate.wait();
            // While the handler holds the permit the slot is unavailable.
            assert!(sem.try_acquire().is_none());
            release.wait();
        });
        assert!(
            sem.try_acquire().is_some(),
            "slot freed once the handler thread exits"
        );
    }

    #[test]
    fn read_only_class_takes_no_lock() {
        let mgr = OpLockManager::new();
        let _g1 = mgr.acquire(&OpLockClass::ReadOnly);
        // A second read-only acquire never blocks.
        let _g2 = mgr.acquire(&OpLockClass::ReadOnly);
    }

    #[test]
    fn same_vm_ops_serialize() {
        let mgr = OpLockManager::new();
        let (order_tx, order_rx) = sync_channel::<u8>(4);
        let entered = Arc::new(AtomicUsize::new(0));

        let guard = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
        let _ = order_tx.try_send(1);

        let mgr2 = mgr.clone();
        let order_tx2 = order_tx.clone();
        let entered2 = Arc::clone(&entered);
        // The main thread still holds the per-VM guard, so the second op
        // CANNOT have entered (its acquire blocks) - the assertion is
        // timing-free. Dropping the guard releases it and the scope join
        // waits for the op to finish.
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _g = mgr2.acquire(&OpLockClass::PerVm("work".to_owned()));
                entered2.fetch_add(1, Ordering::SeqCst);
                let _ = order_tx2.try_send(2);
            });
            assert_eq!(
                entered.load(Ordering::SeqCst),
                0,
                "second same-VM op must block until the first releases"
            );
            drop(guard);
        });
        let mut order = Vec::new();
        while let Ok(value) = order_rx.try_recv() {
            order.push(value);
        }
        assert_eq!(order, vec![1, 2], "ops ran in serialized order");
    }

    #[test]
    fn different_vm_ops_run_concurrently() {
        let mgr = OpLockManager::new();
        let _a = mgr.acquire(&OpLockClass::PerVm("alpha".to_owned()));
        // A different VM must not block while alpha is held.
        let entered = Arc::new(AtomicUsize::new(0));
        let mgr2 = mgr.clone();
        let entered2 = Arc::clone(&entered);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _b = mgr2.acquire(&OpLockClass::PerVm("beta".to_owned()));
                entered2.fetch_add(1, Ordering::SeqCst);
            });
        });
        assert_eq!(
            entered.load(Ordering::SeqCst),
            1,
            "different-VM op proceeds while another VM is locked"
        );
    }

    #[test]
    fn global_op_excludes_per_vm_op() {
        let mgr = OpLockManager::new();
        let global = mgr.acquire(&OpLockClass::Global);
        let entered = Arc::new(AtomicUsize::new(0));
        let mgr2 = mgr.clone();
        let entered2 = Arc::clone(&entered);
        // The global guard is held by the main thread, so the per-VM op
        // CANNOT have entered (its acquire blocks) - timing-free.
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _g = mgr2.acquire(&OpLockClass::PerVm("work".to_owned()));
                entered2.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(
                entered.load(Ordering::SeqCst),
                0,
                "per-VM op must wait for the global op to finish"
            );
            drop(global);
        });
        assert_eq!(entered.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn global_vs_per_vm_and_restart_no_deadlock() {
        // Models hostPrepare (global) vs. start (per-VM) vs. a restart
        // that internally does stop+start under the SAME already-held
        // per-VM guard (no re-acquire). Must terminate, not deadlock.
        let mgr = OpLockManager::new();

        // Models hostPrepare (global) vs. start (per-VM) vs. a restart
        // that internally does stop+start under the SAME already-held
        // per-VM guard (no re-acquire). Must terminate, not deadlock.
        std::thread::scope(|scope| {
            let restart = {
                let mgr = mgr.clone();
                scope.spawn(move || {
                    let _g = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
                    // Inner stop+start are plain calls under the SAME guard:
                    // they must NOT re-acquire the per-VM lock.
                })
            };
            let host_prepare = {
                let mgr = mgr.clone();
                scope.spawn(move || {
                    let _g = mgr.acquire(&OpLockClass::Global);
                })
            };
            let start = {
                let mgr = mgr.clone();
                scope.spawn(move || {
                    let _g = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
                })
            };

            // The scope join is the pass condition: every op terminates
            // (no deadlock) before the scope returns.
            restart.join().expect("restart op terminates");
            host_prepare.join().expect("host prepare op terminates");
            start.join().expect("start op terminates");
        });
    }
}
