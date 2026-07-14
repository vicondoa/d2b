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
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, fcntl},
    libc,
};
use rustix::fs::{FileType, Mode, OFlags};

use crate::{AnchoredResource, Error, ErrorCode, MetadataExpectation, RelativePath, Result};

pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
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
    fd: BorrowedFd<'a>,
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
        self.fd
    }

    pub fn policy(&self) -> FdTransferPolicy {
        self.policy
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
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockGuard")
            .field("lock_id", &self.lock_id)
            .field("resource_id", &self.resource_id)
            .field("ownership_epoch", &self.ownership_epoch)
            .field("global_order", &self.global_order)
            .field("held", &self.held)
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

    pub fn authorize_transfer(&self, requested: FdTransferPolicy) -> Result<OfdTransfer<'_>> {
        if !self.held
            || requested == FdTransferPolicy::Never
            || requested != self.transfer
            || requested != FdTransferPolicy::ComponentSessionAttachment
        {
            return Err(Error::Code(ErrorCode::TransferDenied));
        }
        Ok(OfdTransfer {
            fd: self.fd.as_fd(),
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
        resource: &AnchoredResource<'_>,
        metadata: MetadataExpectation,
        ownership_epoch: OwnershipEpoch,
        cancellation: &impl Cancellation,
    ) -> Result<&LockGuard> {
        self.acquire_with_clock(
            spec,
            resource,
            metadata,
            ownership_epoch,
            cancellation,
            &SystemClock,
        )
    }

    pub fn acquire_with_clock<C: Clock>(
        &mut self,
        spec: &LockSpec,
        resource: &AnchoredResource<'_>,
        metadata: MetadataExpectation,
        ownership_epoch: OwnershipEpoch,
        cancellation: &impl Cancellation,
        clock: &C,
    ) -> Result<&LockGuard> {
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
            match set_ofd_lock(&fd, libc::F_WRLCK as i16) {
                Ok(()) => break,
                Err(Errno::EAGAIN | Errno::EACCES) => {
                    if spec.contention == ContentionPolicy::FailFast {
                        return Err(Error::Code(ErrorCode::LockContended));
                    }
                    if spec.cancellation == CancellationPolicy::Cancellable
                        && cancellation.is_cancelled()
                    {
                        return Err(Error::Code(ErrorCode::Cancelled));
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
        });
        Ok(self
            .guards
            .last()
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
}

fn validate_lock_spec(
    spec: &LockSpec,
    resource: &AnchoredResource<'_>,
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
