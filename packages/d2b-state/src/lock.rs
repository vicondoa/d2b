use std::{
    fmt,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    thread,
    time::{Duration, Instant},
};

use d2b_contracts::v2_state::{
    AuthorityRef, CancellationPolicy, ContentionPolicy, FdTransferPolicy, LockKind, LockSpec,
    OwnershipEpoch, ResourceId,
};
use d2b_core::{
    contract_id::ContractId,
    storage::{ActorKind, DegradeScope, StorageJson, StoragePathKind, StoragePathSpec},
    sync::{
        FdPassingMechanism, InheritancePolicy, LockAdoptionPolicy, LockKind as GeneratedLockKind,
        LockSpec as GeneratedLockSpec, LockStalePolicy, LockTimeoutKind, SyncJson,
    },
};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, fcntl},
    libc,
};
use rustix::fs::{FileType, Mode, OFlags};

use crate::{
    AnchoredDir, AnchoredResource, Error, ErrorCode, MetadataExpectation, RelativePath, Result,
    path::{GeneratedResource, dup_cloexec},
};

/// Ceiling on how long a single bounded-wait poll iteration sleeps, used by
/// both [`LockSet::acquire_with_clock`] and
/// [`LockSet::acquire_from_generated_with_clock`]. Every sleep is computed
/// as `remaining_time_before_deadline.min(MAX_LOCK_POLL_BACKOFF)` — this is
/// a *cap*, never an unconditional sleep duration, so a short or
/// soon-to-expire deadline is never overshot by an invented fixed sleep.
const MAX_LOCK_POLL_BACKOFF: Duration = Duration::from_millis(2);

/// Sleeps for the remaining time before `deadline` (measured from
/// `started`), capped at [`MAX_LOCK_POLL_BACKOFF`], or fails closed with
/// [`ErrorCode::Deadline`] if `deadline` has already elapsed. Never sleeps
/// past what remains before the deadline: a sub-cap remaining duration
/// (for example, a lock with a 1ms bounded-wait timeout) sleeps for exactly
/// that remaining duration, never the full cap, so a short or nearly
/// expired deadline is never overshot by an invented fixed sleep.
fn poll_backoff_or_deadline<C: Clock + ?Sized>(
    clock: &C,
    started: Instant,
    deadline: Duration,
) -> Result<()> {
    let elapsed = clock.now().saturating_duration_since(started);
    if elapsed >= deadline {
        return Err(Error::Code(ErrorCode::Deadline));
    }
    let remaining = deadline - elapsed;
    clock.sleep(remaining.min(MAX_LOCK_POLL_BACKOFF));
    Ok(())
}

pub trait Cancellation {
    fn is_cancelled(&self) -> bool;

    fn acquisition_abandoned(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

pub struct OfdTransfer<'a> {
    guard: &'a mut LockGuard,
    policy: FdTransferPolicy,
}

impl fmt::Debug for OfdTransfer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OfdTransfer")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl OfdTransfer<'_> {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.guard.fd.as_fd()
    }

    pub fn policy(&self) -> FdTransferPolicy {
        self.policy
    }

    /// Returns a fresh `O_CLOEXEC` duplicate of the held descriptor,
    /// suitable for handing to `SCM_RIGHTS`/explicit-fd-mapping transport
    /// without exposing the guard's own descriptor value to the recipient
    /// (the recipient gets its own descriptor number referencing the same
    /// open-file-description, so it cannot be used to interfere with this
    /// guard's local descriptor). Always a CLOEXEC-safe
    /// `fcntl(F_DUPFD_CLOEXEC)` duplicate — never a bare `dup`, which would
    /// hand back a descriptor that survives `exec` in the recipient.
    pub fn duplicate(&self) -> Result<OwnedFd> {
        dup_cloexec(self.guard.fd.as_fd())
    }

    /// Commits a successful descriptor transfer. The local guard then closes
    /// its descriptor without issuing `F_UNLCK`; the recipient's duplicate
    /// open-file description remains authoritative for the lock.
    pub fn commit(self) {
        self.guard.held = false;
    }
}

pub struct LockGuard {
    fd: OwnedFd,
    lock_id: ResourceId,
    /// The lock file's own storage-row resource id (the file this guard's
    /// `fd` is actually opened/locked against). Distinct from
    /// [`Self::protected_resources`], which names the *protected state*
    /// resources this lock authorizes mutation of — never the same field
    /// doing double duty (see [`Self::validate_state_binding`] and
    /// [`Self::bind_protected_resource`]).
    lock_file_resource_id: ResourceId,
    owner: AuthorityRef,
    ownership_epoch: OwnershipEpoch,
    global_order: u32,
    transfer: FdTransferPolicy,
    held: bool,
    /// Storage resource ids this lock protects. For guards acquired via the
    /// legacy `v2_state::LockSpec`-based [`LockSet::acquire`]/
    /// `acquire_with_clock`, this is always exactly `[resource.resource_id]`
    /// (the single resource that API has ever protected), preserving that
    /// path's existing single-resource semantics. For guards acquired via
    /// [`LockSet::acquire_from_generated`], this is the caller-supplied,
    /// inventory-validated `protected_resource_ids` set — distinct from
    /// [`Self::lock_file_resource_id`].
    protected_resources: Vec<ResourceId>,
    /// Generated stale/adoption/degrade policy, carried through losslessly
    /// from the generated `sync.json` row. `None` for guards acquired via
    /// the legacy path, which predates and does not carry this policy.
    ///
    /// Stored as the exact `d2b_core` generated types (not re-wrapped in a
    /// new d2b-state type) so external callers can name them directly via
    /// `d2b_core::sync`/`d2b_core::storage` without depending on a
    /// re-export from this crate's private `lock` module.
    generated_stale_policy: Option<LockStalePolicy>,
    generated_adoption_policy: Option<LockAdoptionPolicy>,
    generated_degrade_scope: Option<DegradeScope>,
    /// Whether this call created the lock file (as opposed to opening an
    /// already-existing one). Always `false` for guards acquired via
    /// [`LockSet::acquire_from_generated`], which never creates a lock file
    /// (see that method's docs): a generated lock file is exclusively
    /// broker-created, and the generated acquire path only ever opens an
    /// existing one.
    created: bool,
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockGuard")
            .field("lock_id", &self.lock_id)
            .field("lock_file_resource_id", &self.lock_file_resource_id)
            .field("ownership_epoch", &self.ownership_epoch)
            .field("global_order", &self.global_order)
            .field("held", &self.held)
            .field("protected_resources", &self.protected_resources)
            .finish_non_exhaustive()
    }
}

impl LockGuard {
    pub fn lock_id(&self) -> &ResourceId {
        &self.lock_id
    }

    pub fn global_order(&self) -> u32 {
        self.global_order
    }

    /// The lock file's own storage-row resource id — i.e. the identity of
    /// the file this guard's held descriptor is actually locked against.
    /// This is *not* one of [`Self::protected_resources`] unless a caller
    /// explicitly requested the lock file itself as a protected resource.
    pub fn lock_file_resource_id(&self) -> &ResourceId {
        &self.lock_file_resource_id
    }

    pub fn owner(&self) -> &AuthorityRef {
        &self.owner
    }

    pub fn ownership_epoch(&self) -> OwnershipEpoch {
        self.ownership_epoch
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Storage resource ids protected by this lock. See the field docs on
    /// [`LockGuard::protected_resources`] for exactly what this contains
    /// for each acquisition path.
    pub fn protected_resources(&self) -> &[ResourceId] {
        &self.protected_resources
    }

    /// The generated stale-lock policy for this lock, if it was acquired via
    /// [`LockSet::acquire_from_generated`].
    pub fn generated_stale_policy(&self) -> Option<&LockStalePolicy> {
        self.generated_stale_policy.as_ref()
    }

    /// The generated adoption policy for this lock, if it was acquired via
    /// [`LockSet::acquire_from_generated`].
    pub fn generated_adoption_policy(&self) -> Option<LockAdoptionPolicy> {
        self.generated_adoption_policy
    }

    /// The generated degrade scope for this lock, if it was acquired via
    /// [`LockSet::acquire_from_generated`].
    pub fn generated_degrade_scope(&self) -> Option<DegradeScope> {
        self.generated_degrade_scope
    }

    /// Whether this call created the lock file rather than opening an
    /// already-existing one.
    pub fn created(&self) -> bool {
        self.created
    }

    /// The exact `(dev, ino)` identity of the held lock-file descriptor,
    /// queried fresh via `fstat` on every call. Because this reads the
    /// already-held descriptor rather than re-resolving the filesystem
    /// path, it reports the identity of the file this guard actually locked
    /// even if the path was later unlinked and replaced by something else —
    /// it can never be fooled by a path-level replacement race.
    pub fn fd_identity(&self) -> Result<(u64, u64)> {
        let stat = rustix::fs::fstat(&self.fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        Ok((stat.st_dev, stat.st_ino))
    }

    /// Opens one of the resources this held guard authorizes mutation for,
    /// resolved fresh via a trusted-root + `openat2` walk against `storage`'s
    /// validated inventory and `anchor`/`anchor_path` — never an inferred
    /// parent/path relationship, and never a caller-supplied resource
    /// accepted on trust. `resource_id` must both:
    ///
    /// - resolve to exactly one row in `storage`'s validated inventory
    ///   (missing or duplicate ids fail closed), and
    /// - be a member of [`Self::protected_resources`] as bound at
    ///   acquisition time — a resource this specific guard does not
    ///   authorize is rejected even if it exists and is otherwise
    ///   well-formed.
    ///
    /// On success, returns the legacy [`AnchoredResource`] shape so
    /// existing atomic/audit consumers can use it without any change to
    /// their own code; the resolution backing it is exactly as
    /// non-forgeable as [`LockSet::acquire_from_generated`]'s own lock-file
    /// resolution — the returned value is a fresh capability, not a clone
    /// of anything the caller supplied.
    pub fn bind_protected_resource(
        &self,
        storage: &StorageJson,
        resource_id: &ContractId,
        anchor: &AnchoredDir,
        anchor_path: &std::path::Path,
        metadata: MetadataExpectation,
    ) -> Result<AnchoredResource> {
        if !self.held {
            return Err(Error::Code(ErrorCode::LockReleased));
        }
        metadata.validate()?;
        storage
            .validate_unique_ids()
            .map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;
        let row = find_unique_storage_row(storage, resource_id)?;
        if row.kind != StoragePathKind::RegularFile {
            return Err(Error::Code(ErrorCode::InvalidSchema));
        }
        let encoded = encode_resource_id(row.id.as_str())?;
        if !self.protected_resources.contains(&encoded) {
            return Err(Error::Code(ErrorCode::LockMismatch));
        }
        let row_mode =
            u32::from_str_radix(&row.mode, 8).map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;
        if metadata.mode != row_mode {
            return Err(Error::Code(ErrorCode::MetadataMismatch));
        }
        let generated = GeneratedResource::resolve(
            anchor,
            anchor_path,
            encoded,
            std::path::Path::new(row.path_template.as_str()),
        )?;
        Ok(generated.into_anchored_resource())
    }

    pub fn authorize_transfer(&mut self, requested: FdTransferPolicy) -> Result<OfdTransfer<'_>> {
        if !self.held
            || requested == FdTransferPolicy::Never
            || requested != self.transfer
            || requested != FdTransferPolicy::ComponentSessionAttachment
        {
            return Err(Error::Code(ErrorCode::TransferDenied));
        }
        Ok(OfdTransfer {
            guard: self,
            policy: requested,
        })
    }

    pub fn release(mut self) -> Result<()> {
        self.unlock()
    }

    pub fn release_in_place(&mut self) -> Result<()> {
        self.unlock()
    }

    pub(crate) fn validate_state_binding(
        &self,
        lock_id: &ResourceId,
        resource_id: &ResourceId,
        owner: &AuthorityRef,
        ownership_epoch: OwnershipEpoch,
    ) -> Result<()> {
        if !self.held {
            return Err(Error::Code(ErrorCode::LockReleased));
        }
        if &self.lock_id != lock_id
            || !self.protected_resources.contains(resource_id)
            || &self.owner != owner
            || self.ownership_epoch != ownership_epoch
            || self.transfer != FdTransferPolicy::Never
        {
            return Err(Error::Code(ErrorCode::LockMismatch));
        }
        Ok(())
    }

    fn unlock(&mut self) -> Result<()> {
        if self.held {
            set_ofd_lock(&self.fd, libc::F_UNLCK as i16).map_err(|error| Error::Os {
                code: ErrorCode::Io,
                errno: Some(error as i32),
            })?;
            self.held = false;
        }
        Ok(())
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.unlock();
    }
}

#[derive(Debug, Default)]
pub struct LockSet {
    guards: Vec<LockGuard>,
}

impl LockSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn held(&self, lock_id: &ResourceId) -> bool {
        self.guards
            .iter()
            .any(|guard| guard.held && &guard.lock_id == lock_id)
    }

    pub fn acquire(
        &mut self,
        spec: &LockSpec,
        resource: &AnchoredResource,
        metadata: MetadataExpectation,
        ownership_epoch: OwnershipEpoch,
        cancellation: &(impl Cancellation + ?Sized),
    ) -> Result<&mut LockGuard> {
        self.acquire_with_clock(
            spec,
            resource,
            metadata,
            ownership_epoch,
            cancellation,
            &SystemClock,
        )
    }

    pub fn acquire_with_clock<C: Clock + ?Sized>(
        &mut self,
        spec: &LockSpec,
        resource: &AnchoredResource,
        metadata: MetadataExpectation,
        ownership_epoch: OwnershipEpoch,
        cancellation: &(impl Cancellation + ?Sized),
        clock: &C,
    ) -> Result<&mut LockGuard> {
        validate_lock_spec(spec, resource, metadata)?;
        if self.held(&spec.lock_id)
            || self
                .guards
                .iter()
                .filter(|guard| guard.held)
                .any(|guard| guard.global_order >= spec.global_order)
            || spec
                .acquire_after
                .iter()
                .any(|dependency| !self.held(dependency))
        {
            return Err(Error::Code(ErrorCode::LockOrder));
        }

        let path = RelativePath::from_components([resource.leaf.as_str()])?;
        let (fd, created) = match resource.directory.open_beneath(
            &path,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(metadata.mode),
        ) {
            Ok(fd) => (fd, true),
            Err(error) if error.code() == ErrorCode::AlreadyExists => (
                resource
                    .directory
                    .open_beneath(&path, OFlags::RDWR, Mode::empty())?,
                false,
            ),
            Err(error) => return Err(error),
        };
        if created {
            rustix::fs::fchmod(&fd, Mode::from_raw_mode(metadata.mode))
                .map_err(|error| Error::io(ErrorCode::Io, error))?;
        }
        validate_metadata(&fd, metadata)?;

        let started = clock.now();
        let deadline = Duration::from_millis(u64::from(spec.deadline_ms));
        loop {
            if cancellation.acquisition_abandoned()
                || (spec.cancellation == CancellationPolicy::Cancellable
                    && cancellation.is_cancelled())
            {
                return Err(Error::Code(ErrorCode::Cancelled));
            }
            match set_ofd_lock(&fd, libc::F_WRLCK as i16) {
                Ok(()) => break,
                Err(Errno::EAGAIN | Errno::EACCES) => {
                    if spec.contention == ContentionPolicy::FailFast {
                        return Err(Error::Code(ErrorCode::LockContended));
                    }
                    poll_backoff_or_deadline(clock, started, deadline)?;
                }
                Err(error) => {
                    return Err(Error::Os {
                        code: ErrorCode::Io,
                        errno: Some(error as i32),
                    });
                }
            }
        }

        self.guards.push(LockGuard {
            fd,
            lock_id: spec.lock_id.clone(),
            lock_file_resource_id: spec.key.resource_id.clone(),
            owner: spec.owner.clone(),
            ownership_epoch,
            global_order: spec.global_order,
            transfer: spec.fd_transfer,
            held: true,
            protected_resources: vec![resource.resource_id.clone()],
            generated_stale_policy: None,
            generated_adoption_policy: None,
            generated_degrade_scope: None,
            created,
        });
        Ok(self
            .guards
            .last_mut()
            .expect("guard was inserted immediately above"))
    }

    pub fn release_last(&mut self) -> Result<()> {
        let guard = self.guards.pop().ok_or(Error::Code(ErrorCode::LockOrder))?;
        guard.release()
    }

    pub fn last(&self) -> Option<&LockGuard> {
        self.guards.last()
    }

    pub fn last_mut(&mut self) -> Option<&mut LockGuard> {
        self.guards.last_mut()
    }

    /// Acquires a lock driven directly by the generated `sync.json` contract
    /// (`d2b_core::sync::LockSpec`), rather than the hand-built
    /// `v2_state::LockSpec` used by [`Self::acquire`]. This is the
    /// canonical bridge: every field the generated contract carries is
    /// consumed as-is (order, timeout/contention, stale/adoption/degrade
    /// policy, fd-transfer mechanism, protected-resource binding); nothing
    /// is invented, defaulted, or reinterpreted, and any field this
    /// function cannot validate causes it to fail closed.
    ///
    /// Callers never hand in a detached `&GeneratedLockSpec`/
    /// `&StoragePathSpec`/`&AnchoredResource` — a prior shape of this API
    /// did, which let a caller substitute an arbitrary same-id row or
    /// resource that was never actually looked up from the trusted
    /// document. Instead:
    ///
    /// - `sync`/`storage` are the full trusted generated documents. This
    ///   function resolves every row it needs — the lock, its own
    ///   lock-file storage row, and each protected resource's storage row —
    ///   directly from these documents by opaque id.
    ///   `sync.validate_lock_order()` and `storage.validate_unique_ids()`
    ///   are re-run here so a caller cannot pass an already-invalid
    ///   document and have a stale or duplicate row silently resolved.
    /// - `lock_id` is the opaque generated id of the lock to acquire; it
    ///   must resolve to exactly one row in `sync.locks` (a missing or
    ///   duplicate id fails closed).
    /// - `protected_resource_ids` are the opaque ids of the storage
    ///   resources this acquisition protects. There is no generated
    ///   parent/child field linking a lock to the resources it protects,
    ///   so the caller supplies them; each one must resolve to exactly one
    ///   row in `storage`, must not repeat, and that row's `scope` is
    ///   checked against the lock's own `scope` so an unrelated resource
    ///   cannot be smuggled in as "protected" by this lock. The lock file's
    ///   own storage row (named by `lock.resource_id`) is a distinct
    ///   concept from a protected resource; see [`LockGuard`]'s field docs.
    /// - `anchor`/`anchor_path` are the trusted pre-opened root and its
    ///   absolute path. The lock file's own storage row is resolved
    ///   beneath `anchor` via [`GeneratedResource::resolve`] only *after*
    ///   every other validation succeeds, directly from the row's own id —
    ///   there is no longer a separate caller-supplied resource parameter
    ///   that could be mismatched with the wrong lock/row.
    /// - The generated lock file is opened, never created (`OFlags::RDWR`
    ///   only — no `CREATE`/`EXCL`). A generated lock file is exclusively
    ///   broker-created; if it is missing, this fails closed with
    ///   [`ErrorCode::Missing`] and the caller must trigger broker
    ///   reconciliation rather than have this function silently create
    ///   one.
    /// - `owner` is rendered to the exact `(ActorKind, value)` convention
    ///   the Nix generator uses and checked against `lock.owner_process`;
    ///   since this API accepts only one authority, `lock.owner_process`
    ///   and `lock.release_authority` must be structurally identical, or
    ///   this fails closed rather than picking one arbitrarily.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_from_generated(
        &mut self,
        sync: &SyncJson,
        storage: &StorageJson,
        lock_id: &ContractId,
        protected_resource_ids: &[ContractId],
        anchor: &AnchoredDir,
        anchor_path: &std::path::Path,
        owner: AuthorityRef,
        ownership_epoch: OwnershipEpoch,
        metadata: MetadataExpectation,
        cancellation: &(impl Cancellation + ?Sized),
    ) -> Result<&mut LockGuard> {
        self.acquire_from_generated_with_clock(
            sync,
            storage,
            lock_id,
            protected_resource_ids,
            anchor,
            anchor_path,
            owner,
            ownership_epoch,
            metadata,
            cancellation,
            &SystemClock,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire_from_generated_with_clock<C: Clock + ?Sized>(
        &mut self,
        sync: &SyncJson,
        storage: &StorageJson,
        lock_id: &ContractId,
        protected_resource_ids: &[ContractId],
        anchor: &AnchoredDir,
        anchor_path: &std::path::Path,
        owner: AuthorityRef,
        ownership_epoch: OwnershipEpoch,
        metadata: MetadataExpectation,
        cancellation: &(impl Cancellation + ?Sized),
        clock: &C,
    ) -> Result<&mut LockGuard> {
        sync.validate_lock_order()
            .map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;
        storage
            .validate_unique_ids()
            .map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;

        let lock = find_unique_lock(sync, lock_id)?;
        let lock_resource_id = lock
            .resource_id
            .as_ref()
            .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
        let lock_row = find_unique_storage_row(storage, lock_resource_id)?;

        let mut seen_requested = std::collections::BTreeSet::new();
        let mut protected_rows = Vec::with_capacity(protected_resource_ids.len());
        for id in protected_resource_ids {
            if !seen_requested.insert(id.clone()) {
                return Err(Error::Code(ErrorCode::InvalidSchema));
            }
            protected_rows.push(find_unique_storage_row(storage, id)?);
        }

        let binding =
            validate_generated_lock(sync, lock, lock_row, &protected_rows, &owner, metadata)?;

        if self.held(&binding.lock_id)
            || self
                .guards
                .iter()
                .filter(|guard| guard.held)
                .any(|guard| guard.global_order >= binding.global_order)
        {
            return Err(Error::Code(ErrorCode::LockOrder));
        }

        let resource = GeneratedResource::resolve(
            anchor,
            anchor_path,
            binding.lock_file_resource_id.clone(),
            std::path::Path::new(lock_row.path_template.as_str()),
        )?;

        // Generated lock files are exclusively broker-created: open only,
        // never create. A missing file fails closed with
        // `ErrorCode::Missing` (naturally surfaced by `open_beneath`'s
        // NOENT mapping) rather than being silently conjured here.
        let fd = resource.open(OFlags::RDWR, Mode::empty())?;
        validate_metadata(&fd, metadata)?;

        // `lock.cloexec_required` was already checked true by
        // `validate_generated_lock`; confirm the fd we actually hold is
        // CLOEXEC rather than trusting the requested open flags alone.
        let fd_flags =
            rustix::fs::fcntl_getfd(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if !fd_flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err(Error::Code(ErrorCode::InvalidSchema));
        }

        let started = clock.now();
        loop {
            // The generated contract has no distinct cancellation-policy
            // field (unlike `v2_state::LockSpec::cancellation`); honouring
            // cancellation unconditionally is the strictly safer choice
            // and is never less correct than any generated policy could
            // require, so it is not an invented value.
            if cancellation.acquisition_abandoned() || cancellation.is_cancelled() {
                return Err(Error::Code(ErrorCode::Cancelled));
            }
            match set_ofd_lock(&fd, libc::F_WRLCK as i16) {
                Ok(()) => break,
                Err(Errno::EAGAIN | Errno::EACCES) => {
                    if binding.fail_fast {
                        return Err(Error::Code(ErrorCode::LockContended));
                    }
                    let deadline = binding
                        .deadline
                        .expect("bounded-wait binding always carries a validated deadline");
                    poll_backoff_or_deadline(clock, started, deadline)?;
                }
                Err(error) => {
                    return Err(Error::Os {
                        code: ErrorCode::Io,
                        errno: Some(error as i32),
                    });
                }
            }
        }

        self.guards.push(LockGuard {
            fd,
            lock_id: binding.lock_id,
            lock_file_resource_id: binding.lock_file_resource_id,
            owner,
            ownership_epoch,
            global_order: binding.global_order,
            transfer: binding.fd_transfer,
            held: true,
            protected_resources: binding.protected_resources,
            generated_stale_policy: Some(lock.stale_policy.clone()),
            generated_adoption_policy: Some(lock.adoption_policy),
            generated_degrade_scope: Some(lock.degrade_scope),
            // The generated acquisition path never creates a lock file
            // (see this method's docs): it only ever opens an
            // already-existing, broker-created one.
            created: false,
        });
        Ok(self
            .guards
            .last_mut()
            .expect("guard was inserted immediately above"))
    }
}

/// Looks up exactly one lock row in `sync.locks` by opaque id. A missing or
/// duplicate id fails closed rather than silently picking the first/last
/// match.
fn find_unique_lock<'a>(sync: &'a SyncJson, lock_id: &ContractId) -> Result<&'a GeneratedLockSpec> {
    let mut matches = sync.locks.iter().filter(|lock| &lock.id == lock_id);
    let found = matches
        .next()
        .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
    if matches.next().is_some() {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    Ok(found)
}

/// Looks up exactly one storage row in `storage.paths` by opaque id. A
/// missing or duplicate id fails closed rather than silently picking the
/// first/last match.
fn find_unique_storage_row<'a>(
    storage: &'a StorageJson,
    id: &ContractId,
) -> Result<&'a StoragePathSpec> {
    let mut matches = storage.paths.iter().filter(|row| &row.id == id);
    let found = matches
        .next()
        .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
    if matches.next().is_some() {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    Ok(found)
}

/// The fully-validated, ready-to-acquire projection of a generated
/// `d2b_core::sync::LockSpec`, produced by [`validate_generated_lock`].
/// Every field here is either copied verbatim from the generated contract
/// or deterministically derived from it (never invented).
struct GeneratedLockBinding {
    lock_id: ResourceId,
    /// The lock-file's own storage-row resource id, encoded from
    /// `lock_row.id`. Kept distinct from [`Self::protected_resources`] —
    /// see [`LockGuard`]'s field docs for why the lock file and the
    /// resources it protects are never conflated.
    lock_file_resource_id: ResourceId,
    global_order: u32,
    protected_resources: Vec<ResourceId>,
    fd_transfer: FdTransferPolicy,
    /// `true` for `FailFast`/`NoWait` (a single non-blocking attempt with no
    /// stored deadline value at all — not even a synthetic 1ms one); `false`
    /// for `BoundedWait`, in which case `deadline` is always `Some`.
    fail_fast: bool,
    deadline: Option<Duration>,
}

#[allow(clippy::too_many_arguments)]
fn validate_generated_lock(
    sync: &SyncJson,
    lock: &GeneratedLockSpec,
    lock_row: &StoragePathSpec,
    protected: &[&StoragePathSpec],
    owner: &AuthorityRef,
    metadata: MetadataExpectation,
) -> Result<GeneratedLockBinding> {
    metadata.validate()?;

    if lock.kind != GeneratedLockKind::Ofd || !lock.cloexec_required {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    let inheritance_consistent = matches!(
        (lock.inheritance_policy, lock.fd_passing_policy.mechanism),
        (InheritancePolicy::CloseOnExec, FdPassingMechanism::None)
            | (
                InheritancePolicy::ScmRightsOnly,
                FdPassingMechanism::ScmRights
            )
            | (
                InheritancePolicy::ExplicitFdMappingOnly,
                FdPassingMechanism::ExplicitFdMapping
            )
    );
    if !inheritance_consistent {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    if !lock.allowed_holders.contains(&lock.owner_process) {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }

    let declared_resource = lock
        .resource_id
        .as_ref()
        .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
    if *declared_resource != lock_row.id || lock.scope != lock_row.scope {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    if lock_row.kind != StoragePathKind::RegularFile {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    if let Some(template) = &lock.path_template
        && *template != lock_row.path_template
    {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }

    let row_mode = u32::from_str_radix(&lock_row.mode, 8)
        .map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;
    if metadata.mode != row_mode {
        return Err(Error::Code(ErrorCode::MetadataMismatch));
    }

    if lock.owner_process != lock.release_authority {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    let (rendered_kind, rendered_value) = render_authority(owner)?;
    if lock.owner_process.kind != rendered_kind
        || lock.owner_process.value.as_str() != rendered_value
    {
        return Err(Error::Code(ErrorCode::LockMismatch));
    }

    let lock_id = encode_resource_id(lock.id.as_str())?;
    let lock_file_resource_id = encode_resource_id(lock_row.id.as_str())?;
    let mut seen = std::collections::BTreeSet::new();
    if !seen.insert(lock_id.clone()) || !seen.insert(lock_file_resource_id.clone()) {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    let mut protected_resources = Vec::with_capacity(protected.len());
    for row in protected {
        if row.scope != lock.scope {
            return Err(Error::Code(ErrorCode::InvalidSchema));
        }
        let encoded = encode_resource_id(row.id.as_str())?;
        if !seen.insert(encoded.clone()) {
            return Err(Error::Code(ErrorCode::InvalidSchema));
        }
        protected_resources.push(encoded);
    }

    let fd_transfer = match lock.fd_passing_policy.mechanism {
        FdPassingMechanism::None => FdTransferPolicy::Never,
        FdPassingMechanism::ScmRights => FdTransferPolicy::ScmRightsLeaseHandoff,
        FdPassingMechanism::ExplicitFdMapping => FdTransferPolicy::ExplicitFdMapping,
    };

    let (fail_fast, deadline) = match lock.timeout_policy.kind {
        LockTimeoutKind::FailFast | LockTimeoutKind::NoWait => {
            if lock.timeout_policy.timeout_ms.is_some() {
                return Err(Error::Code(ErrorCode::InvalidSchema));
            }
            (true, None)
        }
        LockTimeoutKind::BoundedWait => {
            let timeout_ms = lock
                .timeout_policy
                .timeout_ms
                .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
            if timeout_ms == 0
                || timeout_ms > u64::from(d2b_contracts::v2_state::MAX_LOCK_DEADLINE_MS)
            {
                return Err(Error::Code(ErrorCode::InvalidSchema));
            }
            (false, Some(Duration::from_millis(timeout_ms)))
        }
    };

    let global_order = sync
        .global_order_rank(&lock.id)
        .ok_or(Error::Code(ErrorCode::InvalidSchema))?;
    let global_order =
        u32::try_from(global_order).map_err(|_| Error::Code(ErrorCode::InvalidSchema))?;

    Ok(GeneratedLockBinding {
        lock_id,
        lock_file_resource_id,
        global_order,
        protected_resources,
        fd_transfer,
        fail_fast,
        deadline,
    })
}

/// Encodes a generated `ContractId` (charset `[A-Za-z0-9._:/@+-]`, up to
/// 160 bytes) into the runtime `ResourceId` charset
/// (`^[a-z][a-z0-9-]{0,63}$`). Namespace separators (`:`/`/`) and other
/// punctuation collapse to `-`, consecutive separators collapse to one,
/// and leading/trailing separators are trimmed — a deterministic
/// structural transform, not a substitute value. Any uppercase byte is
/// treated as unrepresentable (the target charset has no case
/// distinction, so silently downcasing could collide two structurally
/// distinct generated ids) and fails closed rather than being coerced.
/// Callers combining multiple encoded ids in one call MUST additionally
/// check for collisions across the encoded set, since this encoding is
/// not proven injective in general (see the `BTreeSet` check in
/// [`validate_generated_lock`]).
fn encode_resource_id(raw: &str) -> Result<ResourceId> {
    if raw.is_empty() || raw.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    let mut out = String::with_capacity(raw.len());
    let mut last_was_sep = false;
    for byte in raw.bytes() {
        let mapped = match byte {
            b'a'..=b'z' | b'0'..=b'9' => byte as char,
            b'-' | b':' | b'/' | b'.' | b'_' | b'@' | b'+' => '-',
            _ => return Err(Error::Code(ErrorCode::InvalidSchema)),
        };
        if mapped == '-' {
            if last_was_sep {
                continue;
            }
            last_was_sep = true;
        } else {
            last_was_sep = false;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    ResourceId::parse(trimmed).map_err(|_| Error::Code(ErrorCode::InvalidSchema))
}

/// Renders an `AuthorityRef` to the exact `(ActorKind, value)` pair the Nix
/// generator uses for the corresponding `d2b_core::storage::ActorRef` (see
/// `controllerActor`/`brokerActor` in `nixos-modules/realm-storage-rows.nix`).
/// Only the two authority kinds the generated realm-controller/broker locks
/// actually use are supported; every other `AuthorityRef` variant has no
/// Nix-side naming convention today and fails closed rather than guessing
/// one.
fn render_authority(authority: &AuthorityRef) -> Result<(ActorKind, String)> {
    match authority {
        AuthorityRef::RealmController { realm_id } => {
            Ok((ActorKind::Daemon, format!("d2bd-r-{}", realm_id.as_str())))
        }
        AuthorityRef::RealmBroker { realm_id } => {
            Ok((ActorKind::Broker, format!("d2bbr-r-{}", realm_id.as_str())))
        }
        _ => Err(Error::Code(ErrorCode::InvalidSchema)),
    }
}

fn validate_lock_spec(
    spec: &LockSpec,
    resource: &AnchoredResource,
    metadata: MetadataExpectation,
) -> Result<()> {
    metadata.validate()?;
    if spec.kind != LockKind::Ofd
        || !spec.cloexec
        || spec.key.resource_id != resource.resource_id
        || !matches!(
            spec.fd_transfer,
            FdTransferPolicy::Never | FdTransferPolicy::ComponentSessionAttachment
        )
        || spec.deadline_ms == 0
        || spec.deadline_ms > d2b_contracts::v2_state::MAX_LOCK_DEADLINE_MS
    {
        return Err(Error::Code(ErrorCode::InvalidSchema));
    }
    Ok(())
}

fn validate_metadata(fd: &OwnedFd, expected: MetadataExpectation) -> Result<()> {
    let stat = rustix::fs::fstat(fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != expected.uid
        || stat.st_gid != expected.gid
        || stat.st_mode & 0o7777 != expected.mode
    {
        return Err(Error::Code(ErrorCode::MetadataMismatch));
    }
    Ok(())
}

fn set_ofd_lock(fd: &OwnedFd, lock_type: i16) -> nix::Result<()> {
    let lock = libc::flock {
        l_type: lock_type,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(fd.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use d2b_contracts::v2_identity::{RealmId, RealmPath};
    use d2b_core::{
        contract_id::{ContractId, PathTemplate},
        storage::{
            ActorRef, CleanupPolicy, DegradedReason, LeaseClass, PrincipalKind, PrincipalRef,
            RepairPolicy, SensitivityClass, StorageAdoptionPolicy, StorageLifecycle,
            StoragePersistence, StorageRestartPolicy,
        },
        sync::{
            FdPassingPolicy, LockAcquireOrder, LockScopeClass, LockStaleKind, LockTimeoutPolicy,
        },
    };

    use super::*;
    use crate::AnchoredDir;

    static SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

    /// The fixed, fake absolute directory every test's `StoragePathSpec`
    /// rows declare their `path_template` beneath (see
    /// [`valid_storage_row`]). [`GeneratedResource::resolve`] only ever
    /// uses `anchor_path` for a pure string `strip_prefix` computation
    /// against the row's declared path — it never checks that `anchor`
    /// (the real, opened scratch-directory descriptor used for every
    /// actual filesystem operation) truly lives at this path on disk — so
    /// tests can use one fixed, human-readable `anchor_path` constant
    /// while still exercising the real trusted-root `openat2` resolution
    /// against a real per-test scratch directory.
    const ANCHOR_PATH: &str = "/var/lib/d2b/r/realm/controller";

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("d2b-state-lock-tests")
                .join(format!(
                    "{name}-{}-{}",
                    std::process::id(),
                    SCRATCH_ID.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// `RealmId` is an opaque 20-character derived short id, not a
    /// free-form label; derive a stable one from a realm-path label so
    /// every test call site for the same `label` produces the identical
    /// `RealmId`.
    fn test_realm_id(label: &str) -> RealmId {
        RealmId::derive(&RealmPath::parse(format!("{label}.local-root")).unwrap())
    }

    fn realm_controller(label: &str) -> AuthorityRef {
        AuthorityRef::RealmController {
            realm_id: test_realm_id(label),
        }
    }

    fn controller_actor(label: &str) -> ActorRef {
        ActorRef {
            kind: ActorKind::Daemon,
            value: ContractId::parse(format!("d2bd-r-{}", test_realm_id(label).as_str())).unwrap(),
        }
    }

    /// A fully-populated, schema-valid `GeneratedLockSpec` mirroring the
    /// exact shape `nixos-modules/realm-storage-rows.nix`'s `mkOfdLock`
    /// renders for a controller-owned realm lock. Individual fields are
    /// overridden per test to exercise a single failure mode at a time.
    fn valid_generated_lock(id: &str, resource_id: &str, owner: ActorRef) -> GeneratedLockSpec {
        GeneratedLockSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: None,
            resource_id: Some(ContractId::parse(resource_id).unwrap()),
            kind: GeneratedLockKind::Ofd,
            owner_process: owner.clone(),
            allowed_holders: vec![owner.clone()],
            inheritance_policy: InheritancePolicy::CloseOnExec,
            fd_passing_policy: FdPassingPolicy {
                mechanism: FdPassingMechanism::None,
                lease_transfer_record_required: false,
            },
            acquire_order: LockAcquireOrder {
                scope_class: LockScopeClass::Host,
                anchored_root: ContractId::parse("state").unwrap(),
                normalized_path: ContractId::parse(id).unwrap(),
                lock_id: ContractId::parse(id).unwrap(),
            },
            timeout_policy: LockTimeoutPolicy {
                kind: LockTimeoutKind::FailFast,
                timeout_ms: None,
            },
            stale_policy: LockStalePolicy {
                kind: LockStaleKind::PidfdProofRequired,
                degraded_reason: DegradedReason::LockOwnerAmbiguous,
            },
            adoption_policy: LockAdoptionPolicy::ReacquireAfterProof,
            degrade_scope: DegradeScope::Realm,
            release_authority: owner,
            cloexec_required: true,
        }
    }

    /// A fully-populated, schema-valid regular-file `StoragePathSpec`
    /// mirroring the paired lock-file/protected-resource rows added to
    /// `nixos-modules/realm-storage-rows.nix` (mode `0600`, process-scoped,
    /// not adoptable, no follow). `leaf` names the file beneath a per-test
    /// scratch directory so distinct rows in one test never collide.
    fn valid_storage_row(id: &str, leaf: &str, owner: ActorRef) -> StoragePathSpec {
        StoragePathSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: PathTemplate::parse(format!("/var/lib/d2b/r/realm/controller/{leaf}"))
                .unwrap(),
            kind: StoragePathKind::RegularFile,
            lifecycle: StorageLifecycle::ProcessScoped,
            persistence: StoragePersistence::ProcessScoped,
            owner: PrincipalRef {
                kind: PrincipalKind::Role,
                value: ContractId::parse("d2bd").unwrap(),
            },
            group: PrincipalRef {
                kind: PrincipalKind::Role,
                value: ContractId::parse("d2bd").unwrap(),
            },
            mode: "0600".to_owned(),
            access_acl: Vec::new(),
            default_acl: Vec::new(),
            creator: owner.clone(),
            writers: vec![owner.clone()],
            readers: vec![owner],
            cleanup_policy: CleanupPolicy::Never,
            repair_policy: RepairPolicy::None,
            restart_policy: StorageRestartPolicy::NotApplicable,
            adoption_policy: StorageAdoptionPolicy::NotAdoptable,
            lease_class: LeaseClass::None,
            sensitivity: SensitivityClass::Private,
            no_follow: true,
            recursive: false,
            invariants: Vec::new(),
        }
    }

    fn sync_of(locks: Vec<GeneratedLockSpec>) -> SyncJson {
        SyncJson {
            schema_version: "1".to_owned(),
            locks,
        }
    }

    fn storage_of(paths: Vec<StoragePathSpec>) -> StorageJson {
        StorageJson {
            schema_version: "1".to_owned(),
            roots: Vec::new(),
            paths,
            restart_policies: Vec::new(),
            degraded_states: Vec::new(),
            remediations: Vec::new(),
        }
    }

    fn host_metadata(path: &Path, mode: u32) -> MetadataExpectation {
        use std::os::unix::fs::MetadataExt;
        let parent = fs::metadata(path).unwrap();
        MetadataExpectation {
            uid: parent.uid(),
            gid: parent.gid(),
            mode,
        }
    }

    /// A fully-scripted [`Clock`] for precisely testing
    /// [`poll_backoff_or_deadline`]'s capped-backoff arithmetic without any
    /// dependency on real wall-clock timing. `now()` returns each queued
    /// instant in order (panicking if exhausted, so a test that scripts too
    /// few calls fails loudly rather than silently reusing a stale value);
    /// every `sleep()` call is recorded verbatim, never actually sleeping.
    struct FakeClock {
        base: Instant,
        nows: std::cell::RefCell<VecDeque<Duration>>,
        sleeps: std::cell::RefCell<Vec<Duration>>,
    }

    impl FakeClock {
        fn new(nows: impl IntoIterator<Item = Duration>) -> Self {
            Self {
                base: Instant::now(),
                nows: std::cell::RefCell::new(nows.into_iter().collect()),
                sleeps: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn recorded_sleeps(&self) -> Vec<Duration> {
            self.sleeps.borrow().clone()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            let offset = self
                .nows
                .borrow_mut()
                .pop_front()
                .expect("FakeClock::now() called more times than scripted");
            self.base + offset
        }

        fn sleep(&self, duration: Duration) {
            self.sleeps.borrow_mut().push(duration);
        }
    }

    #[test]
    fn encode_resource_id_collapses_separators_and_trims() {
        assert_eq!(
            encode_resource_id("path:realm-controller-lock:my-realm")
                .unwrap()
                .as_str(),
            "path-realm-controller-lock-my-realm"
        );
        assert_eq!(
            encode_resource_id("provider/foo/state").unwrap().as_str(),
            "provider-foo-state"
        );
    }

    #[test]
    fn encode_resource_id_rejects_uppercase_and_empty() {
        assert!(encode_resource_id("Path:Bad").is_err());
        assert!(encode_resource_id("").is_err());
    }

    #[test]
    fn render_authority_supports_only_realm_controller_and_broker() {
        assert!(render_authority(&realm_controller("my-realm")).is_ok());
        assert!(
            render_authority(&AuthorityRef::RealmBroker {
                realm_id: test_realm_id("my-realm"),
            })
            .is_ok()
        );
        assert!(render_authority(&AuthorityRef::Pid1).is_err());
    }

    /// Precisely exercises [`poll_backoff_or_deadline`]'s capped-backoff
    /// arithmetic with a fully-scripted fake clock: a short (sub-cap)
    /// remaining duration sleeps for exactly that remaining duration, never
    /// the fixed [`MAX_LOCK_POLL_BACKOFF`] cap, and an elapsed deadline
    /// fails closed without sleeping at all.
    #[test]
    fn poll_backoff_or_deadline_caps_to_remaining_never_overshoots() {
        // Deadline is 3ms. First poll: 0ms elapsed -> 3ms remaining, capped
        // at the 2ms MAX_LOCK_POLL_BACKOFF -> sleeps exactly 2ms (the cap).
        // Second poll: 2ms elapsed -> 1ms remaining, *below* the cap ->
        // sleeps exactly 1ms (proving "cap to remaining" is distinct from
        // "always sleep the cap").
        let deadline = Duration::from_millis(3);
        let clock = FakeClock::new([
            Duration::from_millis(0),
            Duration::from_millis(0),
            Duration::from_millis(2),
        ]);
        let started = clock.now();
        poll_backoff_or_deadline(&clock, started, deadline).unwrap();
        poll_backoff_or_deadline(&clock, started, deadline).unwrap();
        assert_eq!(
            clock.recorded_sleeps(),
            vec![Duration::from_millis(2), Duration::from_millis(1)]
        );
    }

    #[test]
    fn poll_backoff_or_deadline_handles_sub_millisecond_remaining() {
        // Deadline is 1ms; 900us have already elapsed, leaving only 100us
        // remaining — far below the 2ms cap. The sleep must be exactly the
        // 100us remaining, never rounded up to the cap.
        let deadline = Duration::from_millis(1);
        let clock = FakeClock::new([Duration::from_micros(0), Duration::from_micros(900)]);
        let started = clock.now();
        poll_backoff_or_deadline(&clock, started, deadline).unwrap();
        assert_eq!(clock.recorded_sleeps(), vec![Duration::from_micros(100)]);
    }

    #[test]
    fn poll_backoff_or_deadline_fails_closed_without_sleeping_when_elapsed() {
        let deadline = Duration::from_millis(1);
        let clock = FakeClock::new([Duration::from_millis(0), Duration::from_millis(1)]);
        let started = clock.now();
        let err = poll_backoff_or_deadline(&clock, started, deadline).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Deadline);
        assert!(clock.recorded_sleeps().is_empty());
    }

    #[test]
    fn acquire_from_generated_round_trips_authority_order_and_protected_resource() {
        let scratch = Scratch::new("generated-ok");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            "state.lock",
            owner.clone(),
        );
        // A distinct resource this lock protects — never the lock file
        // itself — sharing the lock's scope, matching the real
        // `lock:workload-state:<id>` / `workload-state-data` pairing in
        // `nixos-modules/realm-storage-rows.nix`.
        let protected_row =
            valid_storage_row("path:realm-workload-state:my-realm", "state.json", owner);
        // The generated adapter only ever opens a lock file that already
        // exists (broker-created); the protected resource is a plain file
        // the guard authorizes, not something this API touches directly.
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("state.json"), b"{}").unwrap();
        fs::set_permissions(
            scratch.0.join("state.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row.clone(), protected_row.clone()]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                std::slice::from_ref(&protected_row.id),
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        assert!(guard.is_held());
        assert!(!guard.created());
        assert_eq!(guard.global_order(), 0);
        assert_eq!(
            guard.lock_file_resource_id().as_str(),
            "path-realm-controller-lock-my-realm"
        );
        assert_eq!(
            guard.protected_resources(),
            &[ResourceId::parse("path-realm-workload-state-my-realm").unwrap()]
        );
        assert_eq!(
            guard.generated_adoption_policy(),
            Some(LockAdoptionPolicy::ReacquireAfterProof)
        );
        assert_eq!(guard.generated_degrade_scope(), Some(DegradeScope::Realm));
        assert!(guard.generated_stale_policy().is_some());

        // The fd we actually hold must be CLOEXEC.
        let flags = rustix::fs::fcntl_getfd(&guard.fd).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));

        // `validate_state_binding` authorizes the *protected* resource, not
        // the lock file's own resource id.
        assert!(
            guard
                .validate_state_binding(
                    guard.lock_id(),
                    &ResourceId::parse("path-realm-workload-state-my-realm").unwrap(),
                    guard.owner(),
                    guard.ownership_epoch(),
                )
                .is_ok()
        );
        assert_eq!(
            guard
                .validate_state_binding(
                    guard.lock_id(),
                    guard.lock_file_resource_id(),
                    guard.owner(),
                    guard.ownership_epoch(),
                )
                .unwrap_err()
                .code(),
            ErrorCode::LockMismatch
        );

        // A guard-bound resolution of the protected resource: opaque,
        // non-forgeable, derived only from the held guard's authorized set
        // plus a fresh trusted-root walk.
        let bound = guard
            .bind_protected_resource(
                &storage,
                &protected_row.id,
                &anchor,
                Path::new(ANCHOR_PATH),
                host_metadata(&scratch.0, 0o600),
            )
            .unwrap();
        assert_eq!(
            bound.resource_id.as_str(),
            "path-realm-workload-state-my-realm"
        );
    }

    #[test]
    fn acquire_from_generated_missing_lock_file_fails_closed_without_creating() {
        let scratch = Scratch::new("generated-missing-lock-file");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        // Deliberately do NOT create `state.lock` on disk: a generated
        // lock file is exclusively broker-created, so the adapter must
        // open-only and fail closed rather than conjuring one.
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Missing);
        assert!(!scratch.0.join("state.lock").exists());
    }

    #[test]
    fn acquire_from_generated_rejects_unknown_lock_id() {
        let scratch = Scratch::new("generated-unknown-lock");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &ContractId::parse("lock:does-not-exist").unwrap(),
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn acquire_from_generated_rejects_unknown_protected_resource_id() {
        let scratch = Scratch::new("generated-unknown-protected");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[ContractId::parse("path:does-not-exist").unwrap()],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn acquire_from_generated_rejects_duplicate_requested_protected_resource_ids() {
        let scratch = Scratch::new("generated-duplicate-protected");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            "state.lock",
            owner.clone(),
        );
        let protected_row =
            valid_storage_row("path:realm-workload-state:my-realm", "state.json", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("state.json"), b"{}").unwrap();
        fs::set_permissions(
            scratch.0.join("state.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row, protected_row.clone()]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        // The same protected id requested twice in one call — a caller
        // smuggling a duplicate — must be rejected even though the id
        // itself is a valid, unique row in `storage`.
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[protected_row.id.clone(), protected_row.id],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn protected_resource_scope_mismatch_fails_closed() {
        let scratch = Scratch::new("protected-scope-mismatch");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            "state.lock",
            owner.clone(),
        );
        let mut foreign =
            valid_storage_row("path:realm-workloads:my-realm", "workloads.json", owner);
        foreign.scope = ContractId::parse("vm").unwrap();
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("workloads.json"), b"{}").unwrap();
        fs::set_permissions(
            scratch.0.join("workloads.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row, foreign.clone()]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[foreign.id],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn bind_protected_resource_rejects_resource_not_authorized_by_this_guard() {
        let scratch = Scratch::new("bind-unauthorized");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            "state.lock",
            owner.clone(),
        );
        // Exists in the trusted inventory and on disk, but is never listed
        // as a protected resource for this acquisition.
        let unauthorized =
            valid_storage_row("path:realm-workload-state:my-realm", "state.json", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("state.json"), b"{}").unwrap();
        fs::set_permissions(
            scratch.0.join("state.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row, unauthorized.clone()]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        let err = guard
            .bind_protected_resource(
                &storage,
                &unauthorized.id,
                &anchor,
                Path::new(ANCHOR_PATH),
                host_metadata(&scratch.0, 0o600),
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::LockMismatch);
    }

    #[test]
    fn bind_protected_resource_rejects_after_release() {
        let scratch = Scratch::new("bind-after-release");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            "state.lock",
            owner.clone(),
        );
        let protected_row =
            valid_storage_row("path:realm-workload-state:my-realm", "state.json", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("state.json"), b"{}").unwrap();
        fs::set_permissions(
            scratch.0.join("state.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row, protected_row.clone()]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        set.acquire_from_generated(
            &sync,
            &storage,
            &lock.id,
            std::slice::from_ref(&protected_row.id),
            &anchor,
            Path::new(ANCHOR_PATH),
            realm_controller("my-realm"),
            OwnershipEpoch::new(1).unwrap(),
            metadata,
            &NeverCancelled,
        )
        .unwrap();
        set.last_mut()
            .expect("guard was inserted immediately above")
            .release_in_place()
            .unwrap();
        let guard = set.last().expect("released guard remains in the set");

        let err = guard
            .bind_protected_resource(
                &storage,
                &protected_row.id,
                &anchor,
                Path::new(ANCHOR_PATH),
                host_metadata(&scratch.0, 0o600),
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::LockReleased);
    }

    #[test]
    fn acquire_from_generated_enforces_total_order_across_two_locks() {
        let scratch = Scratch::new("generated-total-order");
        let owner = controller_actor("my-realm");
        // "aaa" sorts before "bbb" under the acquire-order key, so `first`
        // must always be acquired before `second`.
        let mut first = valid_generated_lock(
            "lock:aaa-realm-controller:my-realm",
            "path:aaa-realm-controller-lock:my-realm",
            owner.clone(),
        );
        first.acquire_order.normalized_path = ContractId::parse("aaa").unwrap();
        first.acquire_order.lock_id = first.id.clone();
        let mut second = valid_generated_lock(
            "lock:bbb-realm-controller:my-realm",
            "path:bbb-realm-controller-lock:my-realm",
            owner.clone(),
        );
        second.acquire_order.normalized_path = ContractId::parse("bbb").unwrap();
        second.acquire_order.lock_id = second.id.clone();
        let first_row = valid_storage_row(
            "path:aaa-realm-controller-lock:my-realm",
            "aaa.lock",
            owner.clone(),
        );
        let second_row =
            valid_storage_row("path:bbb-realm-controller-lock:my-realm", "bbb.lock", owner);
        fs::write(scratch.0.join("aaa.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("aaa.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::write(scratch.0.join("bbb.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("bbb.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![first.clone(), second.clone()]);
        let storage = storage_of(vec![first_row, second_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        // Acquiring the lower-ranked `second` while holding nothing yet is
        // fine on its own, but acquiring `first` (a *lower* rank) after
        // `second` (a *higher* rank) is already held must fail: total
        // order forbids acquiring backwards.
        set.acquire_from_generated(
            &sync,
            &storage,
            &second.id,
            &[],
            &anchor,
            Path::new(ANCHOR_PATH),
            realm_controller("my-realm"),
            OwnershipEpoch::new(1).unwrap(),
            metadata,
            &NeverCancelled,
        )
        .unwrap();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &first.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::LockOrder);
    }

    #[test]
    fn fd_identity_is_immune_to_path_replacement_race() {
        let scratch = Scratch::new("fd-identity-race");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        let original_identity = guard.fd_identity().unwrap();

        // Simulate a replacement race: unlink the path and create a fresh
        // file there while the guard is still held.
        let path = scratch.0.join("state.lock");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replaced").unwrap();
        let replaced_identity = {
            use std::os::unix::fs::MetadataExt;
            let stat = fs::metadata(&path).unwrap();
            (stat.dev(), stat.ino())
        };

        // The held descriptor's identity is unaffected by the path-level
        // replacement: it still reports the *original* file it locked.
        assert_eq!(guard.fd_identity().unwrap(), original_identity);
        assert_ne!(original_identity, replaced_identity);
    }

    #[test]
    fn cloexec_required_false_fails_closed() {
        let scratch = Scratch::new("cloexec-false");
        let owner = controller_actor("my-realm");
        let mut lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        lock.cloexec_required = false;
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn release_authority_mismatch_fails_closed() {
        let scratch = Scratch::new("release-mismatch");
        let owner = controller_actor("my-realm");
        let mut lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        lock.release_authority = ActorRef {
            kind: ActorKind::Broker,
            value: ContractId::parse("d2bbr-r-my-realm").unwrap(),
        };
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn bounded_wait_without_timeout_fails_closed() {
        let scratch = Scratch::new("bounded-wait-missing");
        let owner = controller_actor("my-realm");
        let mut lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        lock.timeout_policy = LockTimeoutPolicy {
            kind: LockTimeoutKind::BoundedWait,
            timeout_ms: None,
        };
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn fail_fast_with_extraneous_timeout_fails_closed() {
        let scratch = Scratch::new("fail-fast-extraneous");
        let owner = controller_actor("my-realm");
        let mut lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        lock.timeout_policy.timeout_ms = Some(5);
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        fs::write(scratch.0.join("state.lock"), b"").unwrap();
        fs::set_permissions(
            scratch.0.join("state.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn bounded_wait_contention_times_out_against_a_real_conflicting_descriptor() {
        let scratch = Scratch::new("bounded-wait-real-contention");
        let owner = controller_actor("my-realm");
        let mut lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner.clone(),
        );
        lock.timeout_policy = LockTimeoutPolicy {
            kind: LockTimeoutKind::BoundedWait,
            timeout_ms: Some(15),
        };
        let lock_row =
            valid_storage_row("path:realm-controller-lock:my-realm", "state.lock", owner);
        let lock_path = scratch.0.join("state.lock");
        fs::write(&lock_path, b"").unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).unwrap();

        // Hold a conflicting OFD write lock via an independent open file
        // description for the whole test, so every poll iteration inside
        // `acquire_from_generated` observes real contention. The `OwnedFd`
        // must stay alive for the entire test — dropping it (even as an
        // unbound temporary) closes the descriptor and releases the OFD
        // lock immediately, since OFD locks are tied to the open file
        // description's last referencing descriptor, not to the process.
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let _contender_fd = OwnedFd::from(contender);
        set_ofd_lock(&_contender_fd, libc::F_WRLCK as i16).unwrap();

        let sync = sync_of(vec![lock.clone()]);
        let storage = storage_of(vec![lock_row]);
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &storage,
                &lock.id,
                &[],
                &anchor,
                Path::new(ANCHOR_PATH),
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Deadline);
    }
}
