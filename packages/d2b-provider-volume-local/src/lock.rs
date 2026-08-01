//! Ordered open-file-description lock discipline.
//!
//! The Provider validates total order and transfer policy. An injected core
//! adapter owns the actual `F_OFD_SETLK` descriptor and proves that it is
//! close-on-exec through [`OfdLockHandle`].

use std::fmt;

use d2b_contracts::v3::ResourceUid;
use d2b_contracts::v3::execution_policy::BoundedToken;

/// Maximum lock acquisition deadline admitted by this Provider.
pub const MAX_LOCK_DEADLINE_MS: u64 = 60_000;

/// A validated opaque lock identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LockId(BoundedToken);

impl LockId {
    /// Parse a bounded lock identity.
    pub fn parse(value: impl Into<String>) -> Result<Self, LockError> {
        BoundedToken::parse(value)
            .map(Self)
            .map_err(|_| LockError::InvalidSpec)
    }
}

impl fmt::Debug for LockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LockId(<redacted>)")
    }
}

/// Whether a held lock may leave its local owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockTransferPolicy {
    /// The lock descriptor must remain local.
    Never,
    /// One explicit ComponentSession attachment may receive it.
    ComponentSessionAttachment,
}

/// A validated OFD lock request.
#[derive(Clone, PartialEq, Eq)]
pub struct LockSpec {
    lock_id: LockId,
    resource_uid: ResourceUid,
    global_order: u32,
    acquire_after: Vec<LockId>,
    deadline_ms: u64,
    transfer: LockTransferPolicy,
}

impl LockSpec {
    /// Construct a bounded lock request.
    pub fn new(
        lock_id: LockId,
        resource_uid: ResourceUid,
        global_order: u32,
        acquire_after: Vec<LockId>,
        deadline_ms: u64,
        transfer: LockTransferPolicy,
    ) -> Result<Self, LockError> {
        if deadline_ms == 0
            || deadline_ms > MAX_LOCK_DEADLINE_MS
            || acquire_after
                .iter()
                .any(|dependency| dependency == &lock_id)
        {
            return Err(LockError::InvalidSpec);
        }
        Ok(Self {
            lock_id,
            resource_uid,
            global_order,
            acquire_after,
            deadline_ms,
            transfer,
        })
    }

    /// Borrow the opaque lock identity.
    pub const fn lock_id(&self) -> &LockId {
        &self.lock_id
    }

    /// Borrow the predecessor identities that must already be held.
    pub fn acquire_after(&self) -> &[LockId] {
        &self.acquire_after
    }

    /// Borrow the Volume resource identity protected by the lock.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    /// Return the total acquisition order.
    pub const fn global_order(&self) -> u32 {
        self.global_order
    }

    /// Return the bounded acquisition deadline.
    pub const fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }
}

impl fmt::Debug for LockSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockSpec")
            .field("global_order", &self.global_order)
            .field("deadline_ms", &self.deadline_ms)
            .field("transfer", &self.transfer)
            .finish_non_exhaustive()
    }
}

/// A closed lock failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockError {
    /// A request violated a fixed lock bound.
    InvalidSpec,
    /// The requested lock would violate total acquisition order.
    OrderViolation,
    /// A required predecessor is not held.
    DependencyMissing,
    /// The adapter refused or failed lock acquisition.
    AcquisitionFailed,
    /// The lock is no longer held.
    Released,
    /// The lock protects a different Volume.
    ResourceMismatch,
    /// Descriptor transfer was not explicitly permitted.
    TransferDenied,
    /// Releasing or transferring the adapter-owned lock failed.
    AdapterFailed,
}

impl LockError {
    /// Return the stable, detail-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSpec => "volume-lock-spec-invalid",
            Self::OrderViolation => "volume-lock-order-invalid",
            Self::DependencyMissing => "volume-lock-dependency-missing",
            Self::AcquisitionFailed => "volume-lock-acquisition-failed",
            Self::Released => "volume-lock-released",
            Self::ResourceMismatch => "volume-lock-resource-mismatch",
            Self::TransferDenied => "volume-lock-transfer-denied",
            Self::AdapterFailed => "volume-lock-adapter-failed",
        }
    }
}

impl fmt::Display for LockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for LockError {}

/// Adapter-owned proof of one held OFD lock.
///
/// Implementations retain the descriptor, release it from [`Drop`], and detach
/// local ownership only after a successful explicit transfer.
pub trait OfdLockHandle: Send {
    /// Release the held OFD lock. Repeated calls must be harmless.
    fn release(&mut self) -> Result<(), LockError>;

    /// Transfer the descriptor and detach local ownership atomically.
    fn commit_transfer(&mut self) -> Result<(), LockError>;
}

/// Adapter seam that performs the actual nonblocking OFD lock operation.
pub trait OfdLockBackend: Send + Sync {
    /// Acquire an OFD lock on a close-on-exec descriptor.
    fn acquire(&self, spec: &LockSpec) -> Result<Box<dyn OfdLockHandle>, LockError>;
}

/// One held lock with an identity and total-order binding.
pub struct LockGuard {
    handle: Option<Box<dyn OfdLockHandle>>,
    spec: LockSpec,
}

impl LockGuard {
    /// Return whether the adapter-owned lock remains held locally.
    pub fn is_held(&self) -> bool {
        self.handle.is_some()
    }

    /// Borrow the lock identity.
    pub const fn lock_id(&self) -> &LockId {
        self.spec.lock_id()
    }

    /// Return the acquisition order.
    pub const fn global_order(&self) -> u32 {
        self.spec.global_order()
    }

    /// Verify that this held lock protects the supplied Volume.
    pub fn validate_resource(&self, resource_uid: &ResourceUid) -> Result<(), LockError> {
        if !self.is_held() {
            return Err(LockError::Released);
        }
        if self.spec.resource_uid() != resource_uid {
            return Err(LockError::ResourceMismatch);
        }
        Ok(())
    }

    /// Begin one explicitly authorized descriptor transfer.
    pub fn authorize_transfer(&mut self) -> Result<OfdTransfer<'_>, LockError> {
        if self.spec.transfer != LockTransferPolicy::ComponentSessionAttachment {
            return Err(LockError::TransferDenied);
        }
        if self.handle.is_none() {
            return Err(LockError::Released);
        }
        Ok(OfdTransfer { guard: self })
    }

    fn release_in_place(&mut self) -> Result<(), LockError> {
        let Some(mut handle) = self.handle.take() else {
            return Ok(());
        };
        handle.release()
    }
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockGuard")
            .field("global_order", &self.global_order())
            .field("held", &self.is_held())
            .finish_non_exhaustive()
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.release_in_place();
    }
}

/// A single-use explicit OFD descriptor transfer authorization.
pub struct OfdTransfer<'a> {
    guard: &'a mut LockGuard,
}

impl OfdTransfer<'_> {
    /// Commit transfer through the adapter and detach local ownership.
    pub fn commit(self) -> Result<(), LockError> {
        let handle = self.guard.handle.as_mut().ok_or(LockError::Released)?;
        handle.commit_transfer()?;
        self.guard.handle = None;
        Ok(())
    }
}

impl fmt::Debug for OfdTransfer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OfdTransfer(<redacted>)")
    }
}

/// A total-order-checked set of held locks.
#[derive(Default)]
pub struct LockSet {
    guards: Vec<LockGuard>,
}

impl LockSet {
    /// Build an empty lock set.
    pub const fn new() -> Self {
        Self { guards: Vec::new() }
    }

    /// Return whether a lock identity is currently held.
    pub fn held(&self, lock_id: &LockId) -> bool {
        self.guards
            .iter()
            .any(|guard| guard.is_held() && guard.lock_id() == lock_id)
    }

    /// Validate order and acquire through the injected OFD adapter.
    pub fn acquire<B: OfdLockBackend>(
        &mut self,
        backend: &B,
        spec: &LockSpec,
    ) -> Result<&mut LockGuard, LockError> {
        if self.held(spec.lock_id())
            || self
                .guards
                .iter()
                .filter(|guard| guard.is_held())
                .any(|guard| guard.global_order() >= spec.global_order())
        {
            return Err(LockError::OrderViolation);
        }
        if spec
            .acquire_after()
            .iter()
            .any(|dependency| !self.held(dependency))
        {
            return Err(LockError::DependencyMissing);
        }
        let handle = backend.acquire(spec)?;
        self.guards.push(LockGuard {
            handle: Some(handle),
            spec: spec.clone(),
        });
        self.guards.last_mut().ok_or(LockError::AcquisitionFailed)
    }

    /// Release the most recently acquired lock.
    pub fn release_last(&mut self) -> Result<(), LockError> {
        let mut guard = self.guards.pop().ok_or(LockError::OrderViolation)?;
        guard.release_in_place()
    }

    /// Borrow the most recently acquired lock.
    pub fn last(&self) -> Option<&LockGuard> {
        self.guards.last()
    }

    /// Mutably borrow the most recently acquired lock.
    pub fn last_mut(&mut self) -> Option<&mut LockGuard> {
        self.guards.last_mut()
    }
}

impl fmt::Debug for LockSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockSet")
            .field(
                "held_count",
                &self.guards.iter().filter(|guard| guard.is_held()).count(),
            )
            .finish()
    }
}
