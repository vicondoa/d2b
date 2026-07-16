//! Concurrency primitives for the public-socket accept loop.
//!
//! Two independent concerns live here, both extracted behind small,
//! hermetically-testable types so the accept-loop and dispatch paths in
//! `lib.rs` stay thin:
//!
//! 1. [`ConnSemaphore`] — a non-blocking, bounded admission gate for
//!    in-flight connection-handler threads. The accept loop performs a
//!    NON-blocking [`ConnSemaphore::try_acquire`]; on a miss it refuses
//!    the connection immediately (typed-busy) instead of ever blocking
//!    `accept()`. The returned [`ConnPermit`] is an RAII token that is
//!    moved INTO the handler thread (and, for an attached exec session,
//!    into the owner closure) so the slot is released exactly when the
//!    handler — not the accept loop — finishes.
//!
//! 2. [`OpLockManager`] — per-VM and global in-process locks so a
//!    mutating lifecycle op (vm start/stop/restart, …) cannot race
//!    another op on the same VM, and a global op (host prepare, keys
//!    rotate, …) is mutually exclusive with every per-VM op. Read-only
//!    verbs take no lock and run fully in parallel. The single lock
//!    ordering (global-read THEN per-VM) is acyclic, so per-VM and
//!    global ops never deadlock. The lock is acquired ONCE at the
//!    dispatch boundary and held across the whole op (DAG + rollback +
//!    cleanup); inner stop/start helpers invoked by restart/rollback do
//!    NOT re-acquire it, so there is no nested self-deadlock.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{
    ArcMutexGuard, ArcRwLockReadGuard, ArcRwLockWriteGuard, Condvar, Mutex, RawMutex, RawRwLock,
    RwLock,
};

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

    /// Current number of in-flight permits. Test/observability helper.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Configured maximum.
    pub fn cap(&self) -> usize {
        self.cap
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
    /// No lock — read-only / status / session-managed verbs.
    ReadOnly,
    /// Per-VM mutating verb; serialized against other ops on this VM.
    PerVm(String),
    /// Global mutating verb; mutually exclusive with all per-VM ops.
    Global,
}

/// Per-VM + global in-process op locks. Cheaply [`Clone`]able (all state
/// behind `Arc`) so it can live inside the `Clone` `ServerState`.
#[derive(Debug, Clone, Default)]
pub struct OpLockManager {
    /// A global op takes the write side (exclusive with every per-VM op);
    /// a per-VM op takes the read side (shared) plus its own per-VM lock.
    global: Arc<RwLock<()>>,
    per_vm: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    mapped_lifecycle: Arc<MappedLifecycleState>,
}

#[derive(Debug, Default)]
struct MappedLifecycleState {
    active_vms: Mutex<HashSet<String>>,
    idle: Condvar,
    #[cfg(test)]
    admissions: AtomicUsize,
}

/// Send-owned authority retained by a detached mapped lifecycle worker.
///
/// Public request dispatch waits for this permit to drop after acquiring its
/// ordinary operation lock. This closes the interval between a timed-out
/// provider waiter and completion of daemon rollback or cleanup.
pub struct MappedLifecyclePermit {
    vm: String,
    state: Arc<MappedLifecycleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedLifecycleAdmissionError {
    Cancelled,
    DeadlineExpired,
}

impl fmt::Debug for MappedLifecyclePermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MappedLifecyclePermit")
            .finish_non_exhaustive()
    }
}

/// RAII guard for a held op lock. Holds the owned parking_lot guards so
/// the lock is released when the guard drops at the end of the op.
#[allow(dead_code)]
pub enum OpLockGuard {
    /// Read-only verb: nothing is held.
    None,
    /// Per-VM verb: shared-global guard + the per-VM exclusive guard.
    PerVm {
        global: ArcRwLockReadGuard<RawRwLock, ()>,
        vm: ArcMutexGuard<RawMutex, ()>,
    },
    /// Global verb: exclusive-global guard.
    Global(ArcRwLockWriteGuard<RawRwLock, ()>),
}

impl OpLockManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the lock appropriate to `class`, blocking the CALLING
    /// (worker) thread — never the accept loop — until it is available.
    pub fn acquire(&self, class: &OpLockClass) -> OpLockGuard {
        match class {
            OpLockClass::ReadOnly => OpLockGuard::None,
            OpLockClass::PerVm(vm) => {
                // Lock ordering: global(read) THEN per-VM. A global op
                // takes global(write), so it cannot interleave with an
                // in-flight per-VM op, and the single ordering is acyclic.
                let global = self.global.read_arc();
                let vm_lock = {
                    let mut map = self.per_vm.lock();
                    Arc::clone(
                        map.entry(vm.clone())
                            .or_insert_with(|| Arc::new(Mutex::new(()))),
                    )
                };
                let vm = vm_lock.lock_arc();
                OpLockGuard::PerVm { global, vm }
            }
            OpLockClass::Global => OpLockGuard::Global(self.global.write_arc()),
        }
    }

    /// Wait for detached mapped lifecycle work that conflicts with `class`.
    ///
    /// Call this after acquiring the ordinary operation lock. The worker does
    /// not need that lock after its public waiter returns, so waiting while
    /// holding the lock cannot block worker completion. Acquiring in this
    /// order also closes the race where a mapped worker becomes active just
    /// before the originating request releases its operation lock.
    pub fn wait_for_mapped_lifecycle(&self, class: &OpLockClass) {
        let mut active = self.mapped_lifecycle.active_vms.lock();
        match class {
            OpLockClass::ReadOnly => {}
            OpLockClass::PerVm(vm) => {
                while active.contains(vm) {
                    self.mapped_lifecycle.idle.wait(&mut active);
                }
            }
            OpLockClass::Global => {
                while !active.is_empty() {
                    self.mapped_lifecycle.idle.wait(&mut active);
                }
            }
        }
    }

    /// Acquire exclusive detached-lifecycle authority for one VM.
    ///
    /// The returned permit is `Send` and is moved into the owned worker. A
    /// direct provider call that bypasses public request dispatch still
    /// serializes here rather than starting a second mutation.
    #[cfg(test)]
    pub fn begin_mapped_lifecycle(&self, vm: &str) -> MappedLifecyclePermit {
        self.begin_mapped_lifecycle_until(vm, Instant::now() + Duration::from_secs(60), || false)
            .expect("uncontended test lifecycle admission")
    }

    /// Acquire detached-lifecycle authority without indefinitely occupying the
    /// caller's executor thread.
    ///
    /// Callers run this blocking wait on a dedicated blocking adapter. The
    /// bounded poll is necessary because cancellation does not signal this
    /// condition variable.
    pub fn begin_mapped_lifecycle_until<F>(
        &self,
        vm: &str,
        deadline: Instant,
        is_cancelled: F,
    ) -> Result<MappedLifecyclePermit, MappedLifecycleAdmissionError>
    where
        F: Fn() -> bool,
    {
        const CANCELLATION_POLL: Duration = Duration::from_millis(10);
        let mut active = self.mapped_lifecycle.active_vms.lock();
        loop {
            if is_cancelled() {
                return Err(MappedLifecycleAdmissionError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(MappedLifecycleAdmissionError::DeadlineExpired);
            }
            if !active.contains(vm) {
                active.insert(vm.to_owned());
                #[cfg(test)]
                self.mapped_lifecycle
                    .admissions
                    .fetch_add(1, Ordering::AcqRel);
                return Ok(MappedLifecyclePermit {
                    vm: vm.to_owned(),
                    state: Arc::clone(&self.mapped_lifecycle),
                });
            }
            self.mapped_lifecycle
                .idle
                .wait_for(&mut active, remaining.min(CANCELLATION_POLL));
        }
    }

    #[cfg(test)]
    pub fn mapped_lifecycle_active(&self, vm: &str) -> bool {
        self.mapped_lifecycle.active_vms.lock().contains(vm)
    }

    #[cfg(test)]
    pub fn mapped_lifecycle_admission_count(&self) -> usize {
        self.mapped_lifecycle.admissions.load(Ordering::Acquire)
    }
}

impl Drop for MappedLifecyclePermit {
    fn drop(&mut self) {
        let mut active = self.state.active_vms.lock();
        active.remove(&self.vm);
        drop(active);
        self.state.idle.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn semaphore_admits_up_to_cap_then_refuses() {
        let sem = ConnSemaphore::new(2);
        let p1 = sem.try_acquire().expect("first permit");
        let p2 = sem.try_acquire().expect("second permit");
        assert_eq!(sem.in_flight(), 2);
        assert!(
            sem.try_acquire().is_none(),
            "cap-hit must refuse, not block"
        );
        drop(p1);
        assert_eq!(sem.in_flight(), 1, "permit drop releases the slot");
        let _p3 = sem.try_acquire().expect("slot freed after drop");
        assert_eq!(sem.in_flight(), 2);
        drop(p2);
    }

    #[test]
    fn semaphore_cap_zero_clamps_to_one() {
        let sem = ConnSemaphore::new(0);
        assert_eq!(sem.cap(), 1);
        let _p = sem.try_acquire().expect("at least one slot");
        assert!(sem.try_acquire().is_none());
    }

    #[test]
    fn semaphore_permit_released_on_handler_thread_exit() {
        let sem = ConnSemaphore::new(1);
        let permit = sem.try_acquire().expect("permit");
        let handle = thread::spawn(move || {
            // The permit is owned by (and dropped at the end of) the
            // handler thread, mirroring the accept-loop move.
            let _moved = permit;
            thread::sleep(Duration::from_millis(20));
        });
        // While the handler holds the permit the slot is unavailable.
        assert!(sem.try_acquire().is_none());
        handle.join().expect("join handler");
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
        let order = Arc::new(Mutex::new(Vec::<u8>::new()));
        let entered = Arc::new(AtomicUsize::new(0));

        let guard = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
        order.lock().push(1);

        let mgr2 = mgr.clone();
        let order2 = Arc::clone(&order);
        let entered2 = Arc::clone(&entered);
        let handle = thread::spawn(move || {
            let _g = mgr2.acquire(&OpLockClass::PerVm("work".to_owned()));
            entered2.fetch_add(1, Ordering::SeqCst);
            order2.lock().push(2);
        });

        // Give the second thread time to (try to) acquire; it must block.
        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "second same-VM op must block until the first releases"
        );
        drop(guard);
        handle.join().expect("join second op");
        assert_eq!(*order.lock(), vec![1, 2], "ops ran in serialized order");
    }

    #[test]
    fn mapped_lifecycle_permit_outlives_request_lock_and_blocks_conflicting_ops() {
        let manager = OpLockManager::new();
        let originating_lock = manager.acquire(&OpLockClass::PerVm("work".to_owned()));
        let lifecycle = manager.begin_mapped_lifecycle("work");
        drop(originating_lock);

        let entered = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for class in [
            OpLockClass::PerVm("work".to_owned()),
            OpLockClass::PerVm("work".to_owned()),
            OpLockClass::Global,
        ] {
            let manager = manager.clone();
            let entered = Arc::clone(&entered);
            handles.push(thread::spawn(move || {
                let _lock = manager.acquire(&class);
                manager.wait_for_mapped_lifecycle(&class);
                entered.fetch_add(1, Ordering::SeqCst);
            }));
        }

        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "same-VM and global operations must remain excluded after request timeout"
        );
        drop(lifecycle);
        for handle in handles {
            handle.join().expect("join mapped lifecycle waiter");
        }
        assert_eq!(entered.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn mapped_lifecycle_permit_does_not_block_another_vm() {
        let manager = OpLockManager::new();
        let _lifecycle = manager.begin_mapped_lifecycle("work");
        let class = OpLockClass::PerVm("personal".to_owned());
        let _lock = manager.acquire(&class);
        manager.wait_for_mapped_lifecycle(&class);
    }

    #[test]
    fn different_vm_ops_run_concurrently() {
        let mgr = OpLockManager::new();
        let _a = mgr.acquire(&OpLockClass::PerVm("alpha".to_owned()));
        // A different VM must not block while alpha is held.
        let entered = Arc::new(AtomicUsize::new(0));
        let mgr2 = mgr.clone();
        let entered2 = Arc::clone(&entered);
        let handle = thread::spawn(move || {
            let _b = mgr2.acquire(&OpLockClass::PerVm("beta".to_owned()));
            entered2.fetch_add(1, Ordering::SeqCst);
        });
        handle.join().expect("join beta op");
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
        let handle = thread::spawn(move || {
            let _g = mgr2.acquire(&OpLockClass::PerVm("work".to_owned()));
            entered2.fetch_add(1, Ordering::SeqCst);
        });
        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "per-VM op must wait for the global op to finish"
        );
        drop(global);
        handle.join().expect("join per-VM op");
        assert_eq!(entered.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn global_vs_per_vm_and_restart_no_deadlock() {
        // Models hostPrepare (global) vs. start (per-VM) vs. a restart
        // that internally does stop+start under the SAME already-held
        // per-VM guard (no re-acquire). Must terminate, not deadlock.
        let mgr = OpLockManager::new();

        let restart = {
            let mgr = mgr.clone();
            thread::spawn(move || {
                let _g = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
                // Inner stop+start are plain calls under the SAME guard:
                // they must NOT re-acquire the per-VM lock.
                thread::sleep(Duration::from_millis(10));
            })
        };
        let host_prepare = {
            let mgr = mgr.clone();
            thread::spawn(move || {
                let _g = mgr.acquire(&OpLockClass::Global);
                thread::sleep(Duration::from_millis(10));
            })
        };
        let start = {
            let mgr = mgr.clone();
            thread::spawn(move || {
                let _g = mgr.acquire(&OpLockClass::PerVm("work".to_owned()));
            })
        };

        restart.join().expect("restart op terminates");
        host_prepare.join().expect("host prepare op terminates");
        start.join().expect("start op terminates");
    }
}
