use std::sync::{Arc, Mutex};

use d2b_contracts::v3::ResourceUid;
use d2b_provider_volume_local::lock::{
    LockError, LockId, LockSet, LockSpec, LockTransferPolicy, OfdLockBackend, OfdLockHandle,
};

#[derive(Default)]
struct Backend {
    releases: Arc<Mutex<u32>>,
    transfers: Arc<Mutex<u32>>,
}

struct Handle {
    releases: Arc<Mutex<u32>>,
    transfers: Arc<Mutex<u32>>,
}

impl OfdLockHandle for Handle {
    fn release(&mut self) -> Result<(), LockError> {
        *self.releases.lock().unwrap() += 1;
        Ok(())
    }

    fn commit_transfer(&mut self) -> Result<(), LockError> {
        *self.transfers.lock().unwrap() += 1;
        Ok(())
    }
}

impl OfdLockBackend for Backend {
    fn acquire(&self, _spec: &LockSpec) -> Result<Box<dyn OfdLockHandle>, LockError> {
        Ok(Box::new(Handle {
            releases: Arc::clone(&self.releases),
            transfers: Arc::clone(&self.transfers),
        }))
    }
}

fn uid() -> ResourceUid {
    ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap()
}

fn spec(id: &str, order: u32, after: Vec<LockId>, transfer: LockTransferPolicy) -> LockSpec {
    LockSpec::new(
        LockId::parse(id).unwrap(),
        uid(),
        order,
        after,
        100,
        transfer,
    )
    .unwrap()
}

#[test]
fn locks_require_predecessors_and_strict_total_order() {
    let backend = Backend::default();
    let first = spec("first", 10, vec![], LockTransferPolicy::Never);
    let second = spec(
        "second",
        20,
        vec![first.lock_id().clone()],
        LockTransferPolicy::Never,
    );
    let mut locks = LockSet::new();

    assert_eq!(
        locks.acquire(&backend, &second).unwrap_err(),
        LockError::DependencyMissing
    );
    locks.acquire(&backend, &first).unwrap();
    locks.acquire(&backend, &second).unwrap();
    assert_eq!(
        locks
            .acquire(
                &backend,
                &spec("lower", 15, vec![], LockTransferPolicy::Never)
            )
            .unwrap_err(),
        LockError::OrderViolation
    );
}

#[test]
fn transfer_is_explicit_and_detaches_local_release() {
    let backend = Backend::default();
    let transferable = spec(
        "transferable",
        10,
        vec![],
        LockTransferPolicy::ComponentSessionAttachment,
    );
    let mut locks = LockSet::new();
    locks.acquire(&backend, &transferable).unwrap();
    locks
        .last()
        .expect("held lock")
        .validate_resource(&uid())
        .unwrap();
    locks
        .last_mut()
        .expect("held lock")
        .authorize_transfer()
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(*backend.transfers.lock().unwrap(), 1);
    assert_eq!(*backend.releases.lock().unwrap(), 0);
}
