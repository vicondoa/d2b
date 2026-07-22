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
    storage::{ActorKind, DegradeScope, StoragePathKind, StoragePathSpec},
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
    AnchoredResource, Error, ErrorCode, MetadataExpectation, RelativePath, Result,
    path::dup_cloexec,
};

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
    resource_id: ResourceId,
    owner: AuthorityRef,
    ownership_epoch: OwnershipEpoch,
    global_order: u32,
    transfer: FdTransferPolicy,
    held: bool,
    /// Storage resource ids this lock protects, beyond the lock file's own
    /// `resource_id`. Empty for guards acquired via the legacy
    /// `v2_state::LockSpec`-based [`LockSet::acquire`]/`acquire_with_clock`,
    /// which has no generated notion of a protected-resource set.
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
    /// `(dev, ino)` of the resource's containing directory at bind time (see
    /// [`AnchoredResource::directory_identity`]), used by
    /// [`LockGuard::verify_binding`] to reject a resource whose containing
    /// directory was replaced after resolution.
    directory_identity: Option<(u64, u64)>,
    /// Whether this call created the lock file (as opposed to opening an
    /// already-existing one).
    created: bool,
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockGuard")
            .field("lock_id", &self.lock_id)
            .field("resource_id", &self.resource_id)
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

    pub fn resource_id(&self) -> &ResourceId {
        &self.resource_id
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

    /// Storage resource ids protected by this lock (beyond the lock file's
    /// own `resource_id`), as bound at acquisition time by
    /// [`LockSet::acquire_from_generated`]. Empty for legacy-path guards.
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

    /// Verifies that `resource` is exactly the resource this guard is bound
    /// to: same `resource_id`, and — for resources resolved via
    /// [`AnchoredResource::resolve_generated`] — the same containing
    /// directory identity captured at bind time. Rejects a resource whose
    /// `resource_id` matches by coincidence/forgery but whose directory was
    /// replaced (or that was never bound through the guarded resolution
    /// path when the guard requires it).
    pub fn verify_binding(&self, resource: &AnchoredResource) -> Result<()> {
        if resource.resource_id != self.resource_id {
            return Err(Error::Code(ErrorCode::LockMismatch));
        }
        if let Some(expected) = self.directory_identity
            && resource.directory_identity() != Some(expected)
        {
            return Err(Error::Code(ErrorCode::LockMismatch));
        }
        Ok(())
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
            || &self.resource_id != resource_id
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
                    if clock.now().saturating_duration_since(started) >= deadline {
                        return Err(Error::Code(ErrorCode::Deadline));
                    }
                    clock.sleep(Duration::from_millis(1));
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
            resource_id: spec.key.resource_id.clone(),
            owner: spec.owner.clone(),
            ownership_epoch,
            global_order: spec.global_order,
            transfer: spec.fd_transfer,
            held: true,
            protected_resources: Vec::new(),
            generated_stale_policy: None,
            generated_adoption_policy: None,
            generated_degrade_scope: None,
            directory_identity: resource.directory_identity(),
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
    /// - `sync` is the full generated document (needed to derive the exact
    ///   total acquire order via [`SyncJson::global_order_rank`]).
    /// - `lock` is the specific generated lock row being acquired.
    /// - `lock_row` is the generated storage row for `lock`'s own lock
    ///   file (`lock.resource_id` must name it); it supplies the lock
    ///   file's declared mode, kind, and path template, cross-checked
    ///   against `resource` and `metadata` rather than trusted blindly.
    /// - `protected` are the generated storage rows this lock is being
    ///   used to protect. The caller supplies them (there is no generated
    ///   parent/child field linking a lock to the resources it protects);
    ///   each row's `scope` is checked against `lock.scope` so an
    ///   unrelated resource cannot be smuggled in as "protected" by this
    ///   lock.
    /// - `resource` must already be bound via
    ///   [`AnchoredResource::resolve_generated`] against `lock_row`; this
    ///   function independently re-derives `lock_row`'s resource id and
    ///   rejects a mismatch, so a caller cannot pair an unrelated
    ///   `AnchoredResource` with a given `lock`/`lock_row`.
    /// - `owner` is rendered to the exact `(ActorKind, value)` convention
    ///   the Nix generator uses and checked against `lock.owner_process`;
    ///   since this API accepts only one authority, `lock.owner_process`
    ///   and `lock.release_authority` must be structurally identical, or
    ///   this fails closed rather than picking one arbitrarily.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_from_generated(
        &mut self,
        sync: &SyncJson,
        lock: &GeneratedLockSpec,
        lock_row: &StoragePathSpec,
        protected: &[StoragePathSpec],
        resource: &AnchoredResource,
        owner: AuthorityRef,
        ownership_epoch: OwnershipEpoch,
        metadata: MetadataExpectation,
        cancellation: &(impl Cancellation + ?Sized),
    ) -> Result<&mut LockGuard> {
        self.acquire_from_generated_with_clock(
            sync,
            lock,
            lock_row,
            protected,
            resource,
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
        lock: &GeneratedLockSpec,
        lock_row: &StoragePathSpec,
        protected: &[StoragePathSpec],
        resource: &AnchoredResource,
        owner: AuthorityRef,
        ownership_epoch: OwnershipEpoch,
        metadata: MetadataExpectation,
        cancellation: &(impl Cancellation + ?Sized),
        clock: &C,
    ) -> Result<&mut LockGuard> {
        let binding =
            validate_generated_lock(sync, lock, lock_row, protected, resource, &owner, metadata)?;

        if self.held(&binding.lock_id)
            || self
                .guards
                .iter()
                .filter(|guard| guard.held)
                .any(|guard| guard.global_order >= binding.global_order)
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
                    if clock.now().saturating_duration_since(started) >= deadline {
                        return Err(Error::Code(ErrorCode::Deadline));
                    }
                    clock.sleep(Duration::from_millis(1));
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
            resource_id: resource.resource_id.clone(),
            owner,
            ownership_epoch,
            global_order: binding.global_order,
            transfer: binding.fd_transfer,
            held: true,
            protected_resources: binding.protected_resources,
            generated_stale_policy: Some(lock.stale_policy.clone()),
            generated_adoption_policy: Some(lock.adoption_policy),
            generated_degrade_scope: Some(lock.degrade_scope),
            directory_identity: resource.directory_identity(),
            created,
        });
        Ok(self
            .guards
            .last_mut()
            .expect("guard was inserted immediately above"))
    }
}

/// The fully-validated, ready-to-acquire projection of a generated
/// `d2b_core::sync::LockSpec`, produced by [`validate_generated_lock`].
/// Every field here is either copied verbatim from the generated contract
/// or deterministically derived from it (never invented).
struct GeneratedLockBinding {
    lock_id: ResourceId,
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
    protected: &[StoragePathSpec],
    resource: &AnchoredResource,
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
    let encoded_resource = encode_resource_id(lock_row.id.as_str())?;
    if encoded_resource != resource.resource_id {
        return Err(Error::Code(ErrorCode::LockMismatch));
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
    let mut seen = std::collections::BTreeSet::new();
    if !seen.insert(lock_id.clone()) || !seen.insert(resource.resource_id.clone()) {
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
        fs,
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
    /// mirroring the paired lock-file rows added to
    /// `nixos-modules/realm-storage-rows.nix` (mode `0600`, process-scoped,
    /// not adoptable, no follow).
    fn valid_storage_row(id: &str, owner: ActorRef) -> StoragePathSpec {
        StoragePathSpec {
            id: ContractId::parse(id).unwrap(),
            scope: ContractId::parse("host").unwrap(),
            path_template: PathTemplate::parse("/var/lib/d2b/r/realm/controller/state.lock")
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

    fn sync_of(lock: &GeneratedLockSpec) -> SyncJson {
        SyncJson {
            schema_version: "1".to_owned(),
            locks: vec![lock.clone()],
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

    /// Resolves `resource_id`/`leaf` beneath `scratch` exactly the way a
    /// real caller would: via [`AnchoredResource::resolve_generated`]
    /// against a trusted anchor, never a raw struct literal.
    fn resolve(scratch: &Path, resource_id: &str, leaf: &str) -> (AnchoredDir, AnchoredResource) {
        let anchor = AnchoredDir::open_trusted(scratch).unwrap();
        let row_path = scratch.join(leaf);
        let resource = AnchoredResource::resolve_generated(
            &anchor,
            scratch,
            ResourceId::parse(resource_id).unwrap(),
            &row_path,
        )
        .unwrap();
        (anchor, resource)
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

    #[test]
    fn acquire_from_generated_round_trips_authority_order_and_policy() {
        let scratch = Scratch::new("generated-ok");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        assert!(guard.is_held());
        assert!(guard.created());
        assert_eq!(guard.global_order(), 0);
        assert_eq!(guard.resource_id(), &resource.resource_id);
        assert!(guard.protected_resources().is_empty());
        assert_eq!(
            guard.generated_adoption_policy(),
            Some(LockAdoptionPolicy::ReacquireAfterProof)
        );
        assert_eq!(guard.generated_degrade_scope(), Some(DegradeScope::Realm));
        assert!(guard.generated_stale_policy().is_some());

        // The fd we actually hold must be CLOEXEC.
        let flags = rustix::fs::fcntl_getfd(&guard.fd).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));

        assert!(guard.verify_binding(&resource).is_ok());
    }

    #[test]
    fn acquire_from_generated_rejects_mismatched_resource() {
        let scratch = Scratch::new("generated-mismatch");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        // Bound against a DIFFERENT resource id than the lock/row declare.
        let (_anchor, resource) = resolve(&scratch.0, "totally-unrelated-resource", "state.lock");
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::LockMismatch);
    }

    #[test]
    fn verify_binding_rejects_wrong_resource_id() {
        let scratch = Scratch::new("verify-wrong-id");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        let (_other_anchor, forged) = resolve(&scratch.0, "some-other-resource", "state.lock");
        assert_eq!(
            guard.verify_binding(&forged).unwrap_err().code(),
            ErrorCode::LockMismatch
        );
    }

    #[test]
    fn verify_binding_rejects_replaced_directory() {
        let scratch = Scratch::new("verify-replaced-dir");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap();

        // Same resource id, but resolved against a *different* directory:
        // simulates the containing directory being replaced out from under
        // an earlier resolution.
        let elsewhere = Scratch::new("verify-replaced-dir-elsewhere");
        fs::write(elsewhere.0.join("state.lock"), b"").unwrap();
        let (_other_anchor, replaced) = resolve(
            &elsewhere.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        assert_eq!(
            guard.verify_binding(&replaced).unwrap_err().code(),
            ErrorCode::LockMismatch
        );
    }

    #[test]
    fn fd_identity_is_immune_to_path_replacement_race() {
        let scratch = Scratch::new("fd-identity-race");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let guard = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
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
            owner,
        );
        lock.cloexec_required = false;
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
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
            owner,
        );
        lock.release_authority = ActorRef {
            kind: ActorKind::Broker,
            value: ContractId::parse("d2bbr-r-my-realm").unwrap(),
        };
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
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
            owner,
        );
        lock.timeout_policy = LockTimeoutPolicy {
            kind: LockTimeoutKind::BoundedWait,
            timeout_ms: None,
        };
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
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
            owner,
        );
        lock.timeout_policy.timeout_ms = Some(5);
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[],
                &resource,
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
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        let mut foreign =
            valid_storage_row("path:realm-workloads:my-realm", lock.owner_process.clone());
        foreign.scope = ContractId::parse("vm").unwrap();
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[foreign],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }

    #[test]
    fn protected_resource_id_collision_fails_closed() {
        let scratch = Scratch::new("protected-collision");
        let owner = controller_actor("my-realm");
        let lock = valid_generated_lock(
            "lock:realm-controller:my-realm",
            "path:realm-controller-lock:my-realm",
            owner,
        );
        let row = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        // `:` and `/` both collapse to `-`, so these two structurally
        // distinct generated ids encode to the same ResourceId.
        let mut colliding = valid_storage_row(
            "path:realm-controller-lock:my-realm",
            lock.owner_process.clone(),
        );
        colliding.id = ContractId::parse("path/realm-controller-lock/my-realm").unwrap();
        let sync = sync_of(&lock);
        let (_anchor, resource) = resolve(
            &scratch.0,
            "path-realm-controller-lock-my-realm",
            "state.lock",
        );
        let metadata = host_metadata(&scratch.0, 0o600);

        let mut set = LockSet::new();
        let err = set
            .acquire_from_generated(
                &sync,
                &lock,
                &row,
                &[colliding],
                &resource,
                realm_controller("my-realm"),
                OwnershipEpoch::new(1).unwrap(),
                metadata,
                &NeverCancelled,
            )
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidSchema);
    }
}
