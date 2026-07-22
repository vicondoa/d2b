use std::{
    ffi::OsStr,
    fmt,
    os::{
        fd::{AsFd, OwnedFd},
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
    /// child beneath this anchor.
    ///
    /// On first creation this `fsync`s the newly-created child directory's
    /// own fd (flushing its metadata) and then `fsync`s `self` (this
    /// directory) so the new directory *entry* itself survives a crash
    /// before this call returns success. A concurrent creator racing to the
    /// same name is treated as success (idempotent retry: the child is
    /// re-opened and adopted rather than erroring), which is exactly the
    /// property first-write paths (gateway/restart) need. `mkdirat` targets
    /// exactly one path component under an already-anchored, already
    /// symlink-free directory fd, so no intermediate component can resolve
    /// through a symlink or magic link.
    pub fn ensure_durable_dir(&self, name: &LeafName, mode: Mode) -> Result<AnchoredDir> {
        let relpath = RelativePath::from_components([name.as_str().to_owned()])?;
        loop {
            match self.open_beneath(&relpath, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty()) {
                Ok(fd) => return AnchoredDir::from_owned_fd(fd),
                Err(err) if err.code() == ErrorCode::Missing => {}
                Err(err) => return Err(err),
            }
            match rustix::fs::mkdirat(self.fd(), name.as_str(), mode) {
                Ok(()) => {
                    let child = self.open_beneath(
                        &relpath,
                        OFlags::RDONLY | OFlags::DIRECTORY,
                        Mode::empty(),
                    )?;
                    rustix::fs::fsync(&child).map_err(|error| Error::io(ErrorCode::Io, error))?;
                    self.fsync_self()?;
                    return AnchoredDir::from_owned_fd(child);
                }
                Err(rustix::io::Errno::EXIST) => {
                    // Idempotent retry: someone else created it concurrently;
                    // loop back around and adopt it via open_beneath.
                    continue;
                }
                Err(error) => return Err(Error::io(ErrorCode::Io, error)),
            }
        }
    }

    pub(crate) fn fsync_self(&self) -> Result<()> {
        rustix::fs::fsync(self.fd()).map_err(|error| Error::io(ErrorCode::Io, error))
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
    /// The `(dev, ino)` of `directory` at the moment this resource was
    /// bound, captured only by [`AnchoredResource::resolve_generated`].
    /// `None` for resources built via the generic [`AnchoredResource::new`]
    /// constructor, which has no trusted-anchor/generated-row binding to
    /// verify against. Used to detect a directory replaced out from under a
    /// previously-bound resource (see [`crate::LockGuard::verify_binding`]).
    directory_identity: Option<(u64, u64)>,
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
            directory_identity: None,
        }
    }

    /// Resolve a generated storage-contract row into a non-forgeable
    /// resource capability.
    ///
    /// `anchor` must be a directory the caller already trusts (typically
    /// opened via [`AnchoredDir::open_trusted`] against a broker/daemon-owned
    /// root); `anchor_path` is the absolute filesystem path `anchor` was
    /// opened at. `row_path` is the generated `StoragePathSpec`'s rendered
    /// absolute path template and `row_id` its generated storage-row id
    /// (already encoded to a [`ResourceId`] by the caller — this function
    /// does not invent or reinterpret ids). The path is resolved from
    /// `anchor` in a single `openat2` beneath/no-symlink/no-magic-link/
    /// same-filesystem call (see `RESOLVE_SAFE`): a caller cannot pair an
    /// arbitrary `anchor` with an arbitrary resource id and have it silently
    /// succeed against a symlinked or cross-filesystem target. The exact
    /// containing directory's `(dev, ino)` is captured at resolution time
    /// so a later consumer (e.g. a held [`crate::LockGuard`]) can detect the
    /// directory being replaced between resolution and use.
    pub fn resolve_generated(
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

        let (directory, identity) = if parts.is_empty() {
            // The lock/resource file lives directly in `anchor`: bind our
            // own independent CLOEXEC descriptor rather than sharing the
            // caller's `anchor` handle by reference, so this resource's
            // lifetime and identity capture are self-contained.
            let duplicated = dup_cloexec(anchor.fd())?;
            let dir = AnchoredDir::from_owned_fd(duplicated)?;
            let stat =
                rustix::fs::fstat(dir.fd()).map_err(|error| Error::io(ErrorCode::Io, error))?;
            (dir, (stat.st_dev, stat.st_ino))
        } else {
            let relpath = RelativePath::from_components(parts.into_iter().map(str::to_owned))?;
            let fd =
                anchor.open_beneath(&relpath, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;
            let stat = rustix::fs::fstat(&fd).map_err(|error| Error::io(ErrorCode::Io, error))?;
            let identity = (stat.st_dev, stat.st_ino);
            (AnchoredDir::from_owned_fd(fd)?, identity)
        };

        Ok(Self {
            resource_id,
            directory,
            leaf,
            directory_identity: Some(identity),
        })
    }

    /// The exact `(dev, ino)` of [`Self::directory`] captured at
    /// [`Self::resolve_generated`] time, or `None` for resources built via
    /// the generic [`Self::new`] constructor.
    pub fn directory_identity(&self) -> Option<(u64, u64)> {
        self.directory_identity
    }

    pub(crate) fn leaf_os(&self) -> &OsStr {
        OsStr::new(self.leaf.as_str())
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
    fn ensure_durable_dir_fsyncs_child_and_parent_and_is_retryable_after_fault_injection() {
        let scratch = Scratch::new("durable-mkdir-fault");
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
        let err = AnchoredResource::resolve_generated(
            &anchor,
            &scratch.0,
            resource_id,
            &link.join("state.lock"),
        )
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathRejected);
    }

    #[test]
    fn resolve_generated_binds_the_containing_directory_not_the_leaf_itself() {
        // `resolve_generated` deliberately only resolves and identity-binds
        // the *containing directory* of a generated row's leaf; the leaf
        // file itself is opened later (e.g. via `AnchoredDir::open_beneath`
        // with `NOFOLLOW`) by the actual lock-acquire/open path. Prove the
        // returned resource carries the leaf name unresolved (no
        // symlink-following of the leaf happens here), and that the
        // *directory-open* step this function performs is what the
        // dedicated `open_beneath_rejects_symlinked_leaf` test below
        // exercises for the actual file-open.
        let scratch = Scratch::new("resolve-leaf-name-only");
        let real_dir = scratch.0.join("real");
        std::fs::create_dir(&real_dir).unwrap();
        std::fs::write(real_dir.join("actual.lock"), b"").unwrap();
        symlink(real_dir.join("actual.lock"), real_dir.join("state.lock")).unwrap();

        let anchor = AnchoredDir::open_trusted(&scratch.0).unwrap();
        let resource_id = ResourceId::parse("real-lock").unwrap();
        let resource = AnchoredResource::resolve_generated(
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
        let err = AnchoredResource::resolve_generated(
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
        let resource = AnchoredResource::resolve_generated(
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
            resource.directory_identity(),
            Some((expected.st_dev, expected.st_ino))
        );
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
