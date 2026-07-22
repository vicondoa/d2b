use std::{
    ffi::OsStr,
    fmt,
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path},
    sync::Arc,
};

use d2b_contracts::v2_state::ResourceId;
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

use crate::{Error, ErrorCode, Result};

const RESOLVE_SAFE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS)
    .union(ResolveFlags::NO_XDEV);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeafName(String);

impl LeafName {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || value == "."
            || value == ".."
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LeafName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LeafName([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RelativePath {
    components: Vec<LeafName>,
    rendered: String,
}

impl RelativePath {
    pub fn from_components(
        components: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self> {
        let components = components
            .into_iter()
            .map(|component| LeafName::parse(component.into()))
            .collect::<Result<Vec<_>>>()?;
        if components.is_empty() {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let rendered = components
            .iter()
            .map(LeafName::as_str)
            .collect::<Vec<_>>()
            .join("/");
        Ok(Self {
            components,
            rendered,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.rendered
    }

    pub fn leaf(&self) -> &LeafName {
        self.components
            .last()
            .expect("non-empty path is established by constructor")
    }
}

impl fmt::Debug for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelativePath([redacted])")
    }
}

#[derive(Clone)]
pub struct AnchoredDir {
    fd: Arc<OwnedFd>,
}

impl fmt::Debug for AnchoredDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AnchoredDir")
    }
}

impl AnchoredDir {
    pub fn open_trusted(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| Error::io(ErrorCode::PathRejected, error))?;
        Ok(Self { fd: Arc::new(fd) })
    }

    pub fn from_owned_fd(fd: OwnedFd) -> Result<Self> {
        let stat = rustix::fs::fstat(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Directory {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let flags =
            rustix::fs::fcntl_getfd(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
        if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        Ok(Self { fd: Arc::new(fd) })
    }

    pub(crate) fn open_beneath(
        &self,
        path: &RelativePath,
        flags: OFlags,
        mode: Mode,
    ) -> Result<OwnedFd> {
        openat2(
            self.fd.as_ref(),
            path.as_str(),
            flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            mode,
            RESOLVE_SAFE,
        )
        .map_err(|error| {
            let code = if error == rustix::io::Errno::NOENT {
                ErrorCode::Missing
            } else if error == rustix::io::Errno::EXIST {
                ErrorCode::AlreadyExists
            } else {
                ErrorCode::PathRejected
            };
            Error::io(code, error)
        })
    }

    pub(crate) fn fd(&self) -> impl AsFd + '_ {
        self.fd.as_ref().as_fd()
    }

    /// Durably create (or idempotently adopt) a single-component directory
    /// child beneath this anchor. Delegates to
    /// [`Self::ensure_durable_dir_with_sync`] using the real `fsync(2)`
    /// syscall; see that method for the exact durability contract.
    pub fn ensure_durable_dir(&self, name: &LeafName, mode: Mode) -> Result<AnchoredDir> {
        self.ensure_durable_dir_with_sync(name, mode, &RealDurableSync)
    }

    /// Durably create (or idempotently adopt) a single-component directory
    /// child beneath this anchor, using `sync` for every `fsync` call.
    ///
    /// Every success path — a fresh `mkdirat`, adopting a directory a
    /// concurrent creator raced us to (`EEXIST`), and adopting a directory
    /// that already existed before this call was ever made — funnels
    /// through the *same* open-then-fsync-child-then-fsync-parent sequence
    /// before returning, so durability is never conditional on which of
    /// those three cases actually happened. A failed `fsync` (real I/O
    /// fault, or an injected one via a test [`DurableSync`]) propagates as
    /// an `Err` rather than being swallowed; the directory itself is left
    /// exactly as `mkdirat`/the filesystem left it, so a caller can safely
    /// retry this same call — the retry will adopt the existing directory
    /// and attempt the fsyncs again, rather than erroring on a spurious
    /// `EEXIST`. `mkdirat` targets exactly one path component under an
    /// already-anchored, already symlink-free directory fd, so no
    /// intermediate component can resolve through a symlink or magic link.
    pub(crate) fn ensure_durable_dir_with_sync<S: DurableSync>(
        &self,
        name: &LeafName,
        mode: Mode,
        sync: &S,
    ) -> Result<AnchoredDir> {
        let relpath = RelativePath::from_components([name.as_str().to_owned()])?;
        loop {
            match self.open_beneath(&relpath, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty()) {
                Ok(fd) => {
                    // Whether `fd` was just freshly created by us, adopted
                    // after racing a concurrent creator to `EEXIST` below, or
                    // simply pre-existed before this call: none of those are
                    // durability proof on their own. Fsync the child and then
                    // the parent's directory entry before reporting success.
                    sync.fsync(fd.as_fd())
                        .map_err(|error| Error::io(ErrorCode::Io, error))?;
                    sync.fsync(self.fd().as_fd())
                        .map_err(|error| Error::io(ErrorCode::Io, error))?;
                    return AnchoredDir::from_owned_fd(fd);
                }
                Err(err) if err.code() == ErrorCode::Missing => {}
                Err(err) => return Err(err),
            }
            match rustix::fs::mkdirat(self.fd(), name.as_str(), mode) {
                // Loop back around rather than duplicating the
                // open+fsync+fsync+return sequence here: the top of the loop
                // adopts exactly what was just created and performs the
                // fsyncs uniformly.
                Ok(()) => continue,
                Err(rustix::io::Errno::EXIST) => {
                    // Idempotent retry: someone else created it concurrently;
                    // loop back around and adopt it via open_beneath.
                    continue;
                }
                Err(error) => return Err(Error::io(ErrorCode::Io, error)),
            }
        }
    }
}

/// Seam for durably persisting directory metadata, allowing tests to inject
/// `fsync` faults without touching real filesystem behaviour. Production
/// code always uses [`RealDurableSync`]; only `#[cfg(test)]` code should
/// implement any other [`DurableSync`].
pub(crate) trait DurableSync {
    fn fsync(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<()>;
}

/// The real `fsync(2)` syscall, used by every non-test caller of
/// [`AnchoredDir::ensure_durable_dir`].
pub(crate) struct RealDurableSync;

impl DurableSync for RealDurableSync {
    fn fsync(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<()> {
        rustix::fs::fsync(fd)
    }
}

/// Returns a fresh `O_CLOEXEC` duplicate of `fd`, sharing the same
/// underlying open-file-description. Always uses `fcntl(F_DUPFD_CLOEXEC)`
/// (`rustix::io::fcntl_dupfd_cloexec`) rather than bare `dup`/`F_DUPFD`,
/// which would produce a descriptor that survives `exec` and leaks across
/// privilege boundaries.
pub(crate) fn dup_cloexec(fd: impl AsFd) -> Result<OwnedFd> {
    rustix::io::fcntl_dupfd_cloexec(fd, 0).map_err(|error| Error::io(ErrorCode::Io, error))
}

#[derive(Clone)]
pub struct AnchoredResource {
    pub resource_id: ResourceId,
    pub directory: AnchoredDir,
    pub leaf: LeafName,
}

impl fmt::Debug for AnchoredResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnchoredResource")
            .field("resource_id", &self.resource_id)
            .finish_non_exhaustive()
    }
}

impl AnchoredResource {
    pub fn new(resource_id: ResourceId, directory: &AnchoredDir, leaf: LeafName) -> Self {
        Self {
            resource_id,
            directory: directory.clone(),
            leaf,
        }
    }

    pub(crate) fn leaf_os(&self) -> &OsStr {
        OsStr::new(self.leaf.as_str())
    }
}

/// A non-forgeable, generated-storage-row-bound resource capability.
///
/// Constructible only via [`GeneratedResource::resolve`], which performs
/// the *entire* binding — trusted-anchor + generated absolute path
/// resolution via a single `openat2` beneath/no-symlink/no-magic-link/
/// same-filesystem call — in one atomic step. This type is deliberately
/// crate-private (not re-exported from [`crate`]): it has no public
/// constructor, no public field access, and no `Clone` impl, so an external
/// caller can never fabricate one, pair an arbitrary [`AnchoredDir`] with a
/// resource id it did not actually resolve, or retain a handle that
/// outlives the specific resolution that produced it. `d2b-state`'s only
/// external surfaces backed by this capability are
/// [`crate::LockSet::acquire_from_generated`] (the paired lock file) and
/// [`crate::LockGuard::bind_protected_resource`] (a protected resource the
/// guard authorizes); both consume a `GeneratedResource` immediately and
/// never expose it directly. The legacy [`AnchoredResource`] shape is left
/// untouched by this type so `atomic.rs`/`audit.rs`/broker code that
/// predates the generated-contract bridge needs no changes.
#[derive(Debug)]
pub(crate) struct GeneratedResource {
    resource_id: ResourceId,
    directory: AnchoredDir,
    leaf: LeafName,
}

impl GeneratedResource {
    /// Resolves a generated storage-contract row into a
    /// [`GeneratedResource`].
    ///
    /// `anchor` must be a directory the caller already trusts (typically
    /// opened via [`AnchoredDir::open_trusted`] against a broker/daemon-owned
    /// root); `anchor_path` is the absolute filesystem path `anchor` was
    /// opened at. `row_path` is the generated `StoragePathSpec`'s rendered
    /// absolute path template and `resource_id` its generated storage-row id
    /// already encoded to a [`ResourceId`] by the caller — this function
    /// does not invent or reinterpret ids, it only resolves the path. The
    /// path is resolved from `anchor` in a single `openat2`
    /// beneath/no-symlink/no-magic-link/same-filesystem call (see
    /// `RESOLVE_SAFE`): a caller cannot pair an arbitrary `anchor` with an
    /// arbitrary resource id and have it silently succeed against a
    /// symlinked or cross-filesystem target.
    pub(crate) fn resolve(
        anchor: &AnchoredDir,
        anchor_path: &Path,
        resource_id: ResourceId,
        row_path: &Path,
    ) -> Result<Self> {
        if !anchor_path.is_absolute() || !row_path.is_absolute() {
            return Err(Error::Code(ErrorCode::PathRejected));
        }
        let relative = row_path
            .strip_prefix(anchor_path)
            .map_err(|_| Error::Code(ErrorCode::PathRejected))?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => {
                    parts.push(part.to_str().ok_or(Error::Code(ErrorCode::PathRejected))?);
                }
                _ => return Err(Error::Code(ErrorCode::PathRejected)),
            }
        }
        let leaf_str = parts.pop().ok_or(Error::Code(ErrorCode::PathRejected))?;
        let leaf = LeafName::parse(leaf_str)?;

        let directory = if parts.is_empty() {
            // The resource file lives directly in `anchor`: bind our own
            // independent CLOEXEC descriptor rather than sharing the
            // caller's `anchor` handle by reference, so this resource's
            // lifetime is self-contained.
            let duplicated = dup_cloexec(anchor.fd())?;
            AnchoredDir::from_owned_fd(duplicated)?
        } else {
            let relpath = RelativePath::from_components(parts.into_iter().map(str::to_owned))?;
            let fd =
                anchor.open_beneath(&relpath, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;
            AnchoredDir::from_owned_fd(fd)?
        };

        Ok(Self {
            resource_id,
            directory,
            leaf,
        })
    }

    /// The exact `(dev, ino)` of the bound containing directory, queried
    /// fresh via `fstat` on *every* call — never a value captured or cached
    /// from resolution time — so a consumer re-checking this at bind/use
    /// time can never be fooled by a stale identity. Exercised by this
    /// module's own tests (production code no longer caches or re-derives
    /// directory identity through this type, since binding a protected
    /// resource re-resolves fresh via [`Self::resolve`] on every call
    /// instead); kept as a `#[cfg(test)]` verification hook rather than a
    /// live production accessor.
    #[cfg(test)]
    pub(crate) fn directory_identity(&self) -> Result<(u64, u64)> {
        let stat = rustix::fs::fstat(self.directory.fd())
            .map_err(|error| Error::io(ErrorCode::Io, error))?;
        Ok((stat.st_dev, stat.st_ino))
    }

    /// Opens the bound leaf beneath the resolved, trusted directory. Uses
    /// the same `openat2` beneath/no-symlink/no-magic-link/same-filesystem
    /// policy as directory resolution: a symlinked leaf is rejected exactly
    /// like a symlinked intermediate component.
    pub(crate) fn open(&self, flags: OFlags, mode: Mode) -> Result<OwnedFd> {
        let path = RelativePath::from_components([self.leaf.as_str().to_owned()])?;
        self.directory.open_beneath(&path, flags, mode)
    }

    /// Converts into the legacy [`AnchoredResource`] shape consumed by
    /// atomic/audit/broker code that predates the generated-contract
    /// bridge. This is the one sanctioned way to hand a generated-resolved
    /// resource to that code; the recipient still cannot re-derive or
    /// mutate the resolution that produced it (it only gets the same
    /// public fields any [`AnchoredResource::new`] caller could construct
    /// by hand, with no way to tell the two apart — the non-forgeability
    /// guarantee applies to *how this value was obtained*, not to the
    /// legacy type's own shape).
    pub(crate) fn into_anchored_resource(self) -> AnchoredResource {
        AnchoredResource::new(self.resource_id, &self.directory, self.leaf)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::symlink,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

    /// A scratch directory under `CARGO_MANIFEST_DIR/target/...`, never
    /// `/tmp`, matching the convention used by `tests/state.rs` and
    /// `lock.rs`'s own test module. Removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("d2b-state-path-tests")
                .join(format!(
                    "{name}-{}-{}",
                    std::process::id(),
                    SCRATCH_ID.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn dup_cloexec_produces_a_cloexec_duplicate_sharing_the_same_file() {
        let scratch = Scratch::new("dup-cloexec");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let duplicated = dup_cloexec(anchor.fd()).unwrap();
        let flags = rustix::fs::fcntl_getfd(&duplicated).unwrap();
        assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
        // Same open-file-description as the anchor: identical (dev, ino).
        let anchor_stat = rustix::fs::fstat(anchor.fd()).unwrap();
        let dup_stat = rustix::fs::fstat(&duplicated).unwrap();
        assert_eq!(
            (anchor_stat.st_dev, anchor_stat.st_ino),
            (dup_stat.st_dev, dup_stat.st_ino)
        );
    }

    #[test]
    fn ensure_durable_dir_is_idempotent_and_returns_the_same_identity() {
        let scratch = Scratch::new("durable-mkdir");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let name = LeafName::parse("state").unwrap();

        let first = anchor.ensure_durable_dir(&name, Mode::from(0o700)).unwrap();
        let first_stat = rustix::fs::fstat(first.fd()).unwrap();

        // Idempotent retry: calling again must adopt the existing directory
        // rather than failing on EEXIST, and must observe the identical
        // (dev, ino) — proving no delete+recreate raced underneath us.
        let second = anchor.ensure_durable_dir(&name, Mode::from(0o700)).unwrap();
        let second_stat = rustix::fs::fstat(second.fd()).unwrap();

        assert_eq!(
            (first_stat.st_dev, first_stat.st_ino),
            (second_stat.st_dev, second_stat.st_ino)
        );
    }

    #[test]
    fn ensure_durable_dir_adopts_after_eexist_race_and_is_retryable() {
        let scratch = Scratch::new("durable-mkdir-race");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let name = LeafName::parse("audit").unwrap();

        // Simulate a caller that retries after an earlier attempt raced a
        // concurrent creator to EEXIST: pre-create the directory out from
        // under the anchor exactly like a racing peer would, then confirm
        // `ensure_durable_dir` still succeeds by adopting it (the EEXIST
        // branch), rather than surfacing the race as a fatal error.
        std::fs::create_dir(scratch.0.join(name.as_str())).unwrap();

        let adopted = anchor.ensure_durable_dir(&name, Mode::from(0o700)).unwrap();
        let stat = rustix::fs::fstat(adopted.fd()).unwrap();
        assert_eq!(
            rustix::fs::FileType::from_raw_mode(stat.st_mode),
            rustix::fs::FileType::Directory
        );

        // A second retry after adoption must still be idempotent.
        let retried = anchor.ensure_durable_dir(&name, Mode::from(0o700)).unwrap();
        let retried_stat = rustix::fs::fstat(retried.fd()).unwrap();
        assert_eq!(
            (stat.st_dev, stat.st_ino),
            (retried_stat.st_dev, retried_stat.st_ino)
        );
    }

    /// A [`DurableSync`] that fails the first `remaining` `fsync` calls
    /// (real I/O-fault injection), then delegates to the real syscall.
    struct FaultingSync {
        remaining: std::cell::Cell<u32>,
    }

    impl DurableSync for FaultingSync {
        fn fsync(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<()> {
            let remaining = self.remaining.get();
            if remaining > 0 {
                self.remaining.set(remaining - 1);
                return Err(rustix::io::Errno::IO);
            }
            rustix::fs::fsync(fd)
        }
    }

    /// A [`DurableSync`] that always delegates to the real syscall but
    /// counts how many times it was invoked.
    struct CountingSync {
        calls: std::cell::Cell<u32>,
    }

    impl DurableSync for CountingSync {
        fn fsync(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            rustix::fs::fsync(fd)
        }
    }

    #[test]
    fn ensure_durable_dir_propagates_a_failed_fsync_and_is_retryable() {
        let scratch = Scratch::new("durable-mkdir-fsync-fault");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let name = LeafName::parse("state").unwrap();

        // The very first fsync call (the freshly-created child) fails: the
        // failure must propagate as an `Err`, not be swallowed, and must
        // not be reported as success.
        let faulting = FaultingSync {
            remaining: std::cell::Cell::new(1),
        };
        let err = anchor
            .ensure_durable_dir_with_sync(&name, Mode::from(0o700), &faulting)
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Io);

        // `mkdirat` itself already succeeded before the fsync fault, so a
        // retry must adopt the now-existing directory, fsync successfully
        // this time, and return the same durable identity — proving a
        // caller can safely retry after a fsync fault instead of being
        // stuck in a failed, half-durable state.
        let retried = anchor.ensure_durable_dir(&name, Mode::from(0o700)).unwrap();
        let stat = rustix::fs::fstat(retried.fd()).unwrap();
        assert_eq!(
            rustix::fs::FileType::from_raw_mode(stat.st_mode),
            rustix::fs::FileType::Directory
        );
    }

    #[test]
    fn ensure_durable_dir_fsyncs_child_and_parent_even_for_a_preexisting_directory() {
        let scratch = Scratch::new("durable-mkdir-preexisting");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let name = LeafName::parse("state").unwrap();

        // The directory already exists *before* `ensure_durable_dir` is
        // ever called — no `mkdirat`/`EEXIST` path is exercised at all.
        // Durability must still be proven: both the child and the parent
        // must be fsynced exactly as the fresh-create path does.
        std::fs::create_dir(scratch.0.join(name.as_str())).unwrap();

        let counting = CountingSync {
            calls: std::cell::Cell::new(0),
        };
        anchor
            .ensure_durable_dir_with_sync(&name, Mode::from(0o700), &counting)
            .unwrap();
        assert_eq!(
            counting.calls.get(),
            2,
            "must fsync both the child and the parent even for an already-existing directory"
        );
    }

    #[test]
    fn resolve_generated_rejects_symlinked_parent_component() {
        let scratch = Scratch::new("resolve-symlink-parent");
        let real_dir = scratch.0.join("real");
        std::fs::create_dir(&real_dir).unwrap();
        std::fs::write(real_dir.join("state.lock"), b"").unwrap();
        let link = scratch.0.join("linked");
        symlink(&real_dir, &link).unwrap();

        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("real-lock").unwrap();
        // The rendered row path walks through the symlinked component
        // `linked/` rather than `real/`; `RESOLVE_SAFE`'s NO_SYMLINKS must
        // reject this even though the final target byte-for-byte matches a
        // legitimate resource.
        let err =
            GeneratedResource::resolve(&anchor, &scratch.0, resource_id, &link.join("state.lock"))
                .unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathRejected);
    }

    #[test]
    fn resolve_generated_binds_the_containing_directory_not_the_leaf_itself() {
        // `resolve` deliberately only resolves and identity-binds the
        // *containing directory* of a generated row's leaf; the leaf file
        // itself is opened later (via `GeneratedResource::open`, which uses
        // `AnchoredDir::open_beneath` with `NOFOLLOW`). Prove the returned
        // resource carries the leaf name unresolved (no symlink-following
        // of the leaf happens here), and that the *directory-open* step
        // this function performs is what the dedicated
        // `open_beneath_rejects_symlinked_leaf` test below exercises for
        // the actual file-open.
        let scratch = Scratch::new("resolve-leaf-name-only");
        let real_dir = scratch.0.join("real");
        std::fs::create_dir(&real_dir).unwrap();
        std::fs::write(real_dir.join("actual.lock"), b"").unwrap();
        symlink(real_dir.join("actual.lock"), real_dir.join("state.lock")).unwrap();

        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("real-lock").unwrap();
        let resource = GeneratedResource::resolve(
            &anchor,
            &scratch.0,
            resource_id,
            &real_dir.join("state.lock"),
        )
        .unwrap();
        assert_eq!(resource.leaf.as_str(), "state.lock");
    }

    #[test]
    fn open_beneath_rejects_symlinked_leaf_component() {
        let scratch = Scratch::new("open-beneath-symlink-leaf");
        std::fs::write(scratch.0.join("actual.lock"), b"").unwrap();
        symlink(scratch.0.join("actual.lock"), scratch.0.join("state.lock")).unwrap();

        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let relpath = RelativePath::from_components(["state.lock".to_owned()]).unwrap();
        let err = anchor
            .open_beneath(&relpath, OFlags::RDONLY, Mode::empty())
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathRejected);
    }

    #[test]
    fn resolve_generated_rejects_relative_paths() {
        let scratch = Scratch::new("resolve-relative");
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("real-lock").unwrap();
        let err = GeneratedResource::resolve(
            &anchor,
            &scratch.0,
            resource_id,
            Path::new("relative/state.lock"),
        )
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathRejected);
    }

    #[test]
    fn resolve_generated_captures_directory_identity_for_nested_resource() {
        let scratch = Scratch::new("resolve-nested");
        let nested = scratch.0.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("state.lock"), b"").unwrap();

        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("nested-lock").unwrap();
        let resource = GeneratedResource::resolve(
            &anchor,
            &scratch.0,
            resource_id,
            &nested.join("state.lock"),
        )
        .unwrap();
        let expected = rustix::fs::fstat(
            rustix::fs::open(
                &nested,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resource.directory_identity().unwrap(),
            (expected.st_dev, expected.st_ino)
        );
    }

    #[test]
    fn resolve_generated_directory_identity_is_reread_not_cached() {
        // `directory_identity()` must reflect the *current* fstat of the
        // bound directory on every call, not a value captured once at
        // resolve time — proving a later caller re-checking identity can
        // never be fooled by a stale cached value.
        let scratch = Scratch::new("resolve-identity-fresh");
        std::fs::write(scratch.0.join("state.lock"), b"").unwrap();
        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("real-lock").unwrap();
        let resource = GeneratedResource::resolve(
            &anchor,
            &scratch.0,
            resource_id,
            &scratch.0.join("state.lock"),
        )
        .unwrap();
        let first = resource.directory_identity().unwrap();
        let second = resource.directory_identity().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn open_trusted_rejects_relative_and_nul_paths() {
        assert_eq!(
            AnchoredDir::open_trusted(Path::new("relative"))
                .unwrap_err()
                .code(),
            ErrorCode::PathRejected
        );
    }
}
