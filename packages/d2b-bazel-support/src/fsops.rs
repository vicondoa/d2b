use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt, io,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

use sha2::{Digest as Sha2Digest, Sha256};

#[cfg(unix)]
use std::os::{fd::OwnedFd, unix::ffi::OsStrExt};

#[cfg(unix)]
use rustix::{
    fs::{self as rustix_fs, FileType as RustixFileType, Mode, OFlags, ResolveFlags, StatExt},
    io as rustix_io,
};

#[cfg(unix)]
pub use rustix::fs::OFlags as OpenFlags;

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OpenFlags;

/// The strict path policy is used for repository and cleanup paths. Provider
/// opens intentionally use the weaker policy so a declared runfiles leaf can
/// be a symlink.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResolvePolicy {
    #[default]
    Strict,
    Provider,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OpenRoute {
    #[default]
    OpenAt2,
    ComponentWalk,
}

/// The resolve bits selected by an open operation, kept separate from the
/// platform bitflags so injected tests can inspect the contract portably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveMask(u8);

impl ResolveMask {
    pub const NONE: Self = Self(0);
    pub const NO_MAGICLINKS: Self = Self(1);
    pub const NO_SYMLINKS: Self = Self(2);
    pub const BENEATH: Self = Self(4);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    const fn for_policy(policy: ResolvePolicy) -> Self {
        match policy {
            ResolvePolicy::Strict => {
                Self(Self::NO_MAGICLINKS.0 | Self::NO_SYMLINKS.0 | Self::BENEATH.0)
            }
            ResolvePolicy::Provider => Self::NO_MAGICLINKS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanoseconds: i64,
}

impl Timestamp {
    pub const fn new(seconds: i64, nanoseconds: i64) -> Self {
        Self {
            seconds,
            nanoseconds,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Regular,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FileMetadata {
    device: u64,
    inode: u64,
    size: u64,
    mode: u32,
    kind: FileKind,
    modified: Timestamp,
    changed: Timestamp,
}

impl FileMetadata {
    fn new(
        device: u64,
        inode: u64,
        size: u64,
        mode: u32,
        kind: FileKind,
        modified: Timestamp,
        changed: Timestamp,
    ) -> Self {
        Self {
            device,
            inode,
            size,
            mode,
            kind,
            modified,
            changed,
        }
    }

    pub const fn size(self) -> u64 {
        self.size
    }

    pub const fn mode(self) -> u32 {
        self.mode
    }

    pub const fn kind(self) -> FileKind {
        self.kind
    }

    pub const fn modified(self) -> Timestamp {
        self.modified
    }

    pub const fn changed(self) -> Timestamp {
        self.changed
    }

    pub const fn is_executable(self) -> bool {
        self.mode & 0o111 != 0
    }

    fn identity_matches(self, other: Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.size == other.size
            && self.modified == other.modified
            && self.changed == other.changed
    }
}

impl fmt::Debug for FileMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FileMetadata(..)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: OsString,
    pub kind: FileKind,
}

impl fmt::Debug for DirectoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectoryEntry(..)")
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn sha256(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl AsRef<[u8]> for Digest {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Digest(..)")
    }
}

enum HandleInner {
    #[cfg(unix)]
    Host(Arc<OwnedFd>),
    Memory(Arc<MemoryHandle>),
}

/// An owned directory or file descriptor. It intentionally has no path.
pub struct FsHandle(HandleInner);

impl fmt::Debug for FsHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FsHandle(..)")
    }
}

impl Clone for FsHandle {
    fn clone(&self) -> Self {
        match &self.0 {
            #[cfg(unix)]
            HandleInner::Host(fd) => Self(HandleInner::Host(Arc::clone(fd))),
            HandleInner::Memory(handle) => Self(HandleInner::Memory(Arc::clone(handle))),
        }
    }
}

impl FsHandle {
    pub fn is_directory(&self) -> bool {
        match &self.0 {
            #[cfg(unix)]
            HandleInner::Host(fd) => rustix_fs::fstat(fd.as_ref())
                .map(|stat| rustix_file_kind(stat.st_mode) == FileKind::Directory)
                .unwrap_or(false),
            HandleInner::Memory(handle) => {
                handle.fs.lock().ok().and_then(|state| {
                    state
                        .nodes
                        .get(&handle.inode)
                        .map(|node| node.metadata.kind)
                }) == Some(FileKind::Directory)
            }
        }
    }
}

/// A descriptor returned by the provider open. Its inner descriptor and path
/// remain private to this filesystem authority.
pub struct ProviderHandle {
    descriptor: FsHandle,
}

impl fmt::Debug for ProviderHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderHandle(..)")
    }
}

impl ProviderHandle {
    fn descriptor(&self) -> &FsHandle {
        &self.descriptor
    }
}

/// A verified provider result retained for the authority-owned admission lane.
#[allow(dead_code)]
pub struct VerifiedProvider {
    handle: ProviderHandle,
    metadata: FileMetadata,
    digest: Digest,
}

pub enum VerificationError {
    Io,
    NotRegular,
    NotExecutable,
    Stale,
    DigestMismatch,
    MetadataChanged,
    ShortRead,
}

impl fmt::Debug for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io => formatter.write_str("provider descriptor operation failed"),
            Self::NotRegular => formatter.write_str("provider is not a regular file"),
            Self::NotExecutable => formatter.write_str("provider is not executable"),
            Self::Stale => formatter.write_str("provider is older than its newest input"),
            Self::DigestMismatch => formatter.write_str("provider digest does not match"),
            Self::MetadataChanged => {
                formatter.write_str("provider metadata changed during digest read")
            }
            Self::ShortRead => formatter.write_str("provider digest read was short"),
        }
    }
}

impl VerificationError {
    const fn code(&self) -> &'static str {
        match self {
            Self::Io => "D2B-BZLEXEC-PROVIDER-IO",
            Self::NotRegular => "D2B-BZLEXEC-PROVIDER-KIND",
            Self::NotExecutable => "D2B-BZLEXEC-PROVIDER-MODE",
            Self::Stale => "D2B-BZLEXEC-PROVIDER-STALE",
            Self::DigestMismatch => "D2B-BZLEXEC-PROVIDER-DIGEST",
            Self::MetadataChanged => "D2B-BZLEXEC-PROVIDER-METADATA",
            Self::ShortRead => "D2B-BZLEXEC-PROVIDER-READ",
        }
    }
}

impl std::error::Error for VerificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<io::Error> for VerificationError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

/// The only filesystem operations used by provider and cleanup consumers.
pub trait FileSystem {
    fn open(&self, path: &Path, flags: OpenFlags, policy: ResolvePolicy) -> io::Result<FsHandle>;
    fn openat2(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OpenFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle>;
    fn open_component_walk(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OpenFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle>;
    fn open_provider(&self, anchor: &FsHandle, relative: &Path) -> io::Result<ProviderHandle>;
    fn fstat(&self, descriptor: &FsHandle) -> io::Result<FileMetadata>;
    fn pread(&self, descriptor: &FsHandle, bytes: &mut [u8], offset: u64) -> io::Result<usize>;
}

pub fn verify_provider<F: FileSystem>(
    filesystem: &F,
    handle: ProviderHandle,
    newest_input: Option<&ProviderHandle>,
    expected_digest: impl AsRef<[u8]>,
) -> Result<VerifiedProvider, VerificationError> {
    let before = filesystem.fstat(handle.descriptor())?;
    if before.kind() != FileKind::Regular {
        return Err(VerificationError::NotRegular);
    }
    if !before.is_executable() {
        return Err(VerificationError::NotExecutable);
    }
    if let Some(input) = newest_input {
        let input_metadata = filesystem.fstat(input.descriptor())?;
        if before.modified() < input_metadata.modified() {
            return Err(VerificationError::Stale);
        }
    }

    let size = usize::try_from(before.size()).map_err(|_| VerificationError::ShortRead)?;
    let mut bytes = vec![0_u8; size];
    let count = filesystem.pread(handle.descriptor(), &mut bytes, 0)?;
    if count != size {
        return Err(VerificationError::ShortRead);
    }
    let actual = Digest::sha256(&bytes);
    if expected_digest.as_ref() != actual.as_ref() {
        return Err(VerificationError::DigestMismatch);
    }
    let after = filesystem.fstat(handle.descriptor())?;
    if !before.identity_matches(after) {
        return Err(VerificationError::MetadataChanged);
    }
    Ok(VerifiedProvider {
        handle,
        metadata: before,
        digest: actual,
    })
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenRecord {
    pub route: OpenRoute,
    pub policy: ResolvePolicy,
    pub flags: OFlags,
    pub resolve_flags: ResolveMask,
    pub intermediate_no_follow: bool,
}

#[cfg(unix)]
#[derive(Clone, Copy)]
pub struct HostFileSystem {
    route: OpenRoute,
}

#[cfg(unix)]
impl fmt::Debug for HostFileSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HostFileSystem(..)")
    }
}

#[cfg(unix)]
impl Default for HostFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(unix)]
impl HostFileSystem {
    pub const fn new() -> Self {
        Self {
            route: OpenRoute::OpenAt2,
        }
    }

    pub const fn forced_component_walk() -> Self {
        Self {
            route: OpenRoute::ComponentWalk,
        }
    }
}

#[cfg(unix)]
impl FileSystem for HostFileSystem {
    fn open(&self, path: &Path, flags: OFlags, _policy: ResolvePolicy) -> io::Result<FsHandle> {
        rustix_fs::open(path, flags | OFlags::CLOEXEC, Mode::empty())
            .map(|fd| FsHandle(HandleInner::Host(Arc::new(fd))))
            .map_err(io::Error::from)
    }

    fn openat2(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        validate_relative(relative)?;
        let fd = host_fd(anchor)?;
        rustix_fs::openat2(
            fd,
            relative,
            flags | OFlags::CLOEXEC,
            Mode::empty(),
            resolve_flags(policy),
        )
        .map(|opened| FsHandle(HandleInner::Host(Arc::new(opened))))
        .map_err(io::Error::from)
    }

    fn open_component_walk(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        validate_relative(relative)?;
        let mut components = relative
            .components()
            .filter_map(normal_component)
            .peekable();
        let Some(mut component) = components.next() else {
            return Err(invalid_input("relative path is empty"));
        };
        let mut current = Arc::new(rustix_io::dup(host_fd(anchor)?).map_err(io::Error::from)?);
        loop {
            let leaf = components.peek().is_none();
            let mut component_flags = if leaf {
                flags | OFlags::CLOEXEC
            } else {
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
            };
            if !leaf || policy == ResolvePolicy::Strict {
                component_flags |= OFlags::NOFOLLOW;
            }
            let opened =
                rustix_fs::openat(current.as_ref(), component, component_flags, Mode::empty())
                    .map_err(io::Error::from)?;
            if leaf {
                return Ok(FsHandle(HandleInner::Host(Arc::new(opened))));
            }
            current = Arc::new(opened);
            component = components.next().expect("peeked component exists");
        }
    }

    fn open_provider(&self, anchor: &FsHandle, relative: &Path) -> io::Result<ProviderHandle> {
        let flags = OFlags::RDONLY | OFlags::CLOEXEC;
        let descriptor = match self.route {
            OpenRoute::OpenAt2 => {
                match self.openat2(anchor, relative, flags, ResolvePolicy::Provider) {
                    Err(error)
                        if error.raw_os_error() == Some(rustix_io::Errno::NOSYS.raw_os_error()) =>
                    {
                        self.open_component_walk(anchor, relative, flags, ResolvePolicy::Provider)?
                    }
                    result => result?,
                }
            }
            OpenRoute::ComponentWalk => {
                self.open_component_walk(anchor, relative, flags, ResolvePolicy::Provider)?
            }
        };
        Ok(ProviderHandle { descriptor })
    }

    fn fstat(&self, descriptor: &FsHandle) -> io::Result<FileMetadata> {
        let stat = rustix_fs::fstat(host_fd(descriptor)?).map_err(io::Error::from)?;
        Ok(FileMetadata::new(
            stat.st_dev,
            stat.st_ino,
            stat.st_size.max(0) as u64,
            stat.st_mode,
            rustix_file_kind(stat.st_mode),
            Timestamp::new(stat.mtime(), stat.st_mtime_nsec as i64),
            Timestamp::new(stat.ctime(), stat.st_ctime_nsec as i64),
        ))
    }

    fn pread(&self, descriptor: &FsHandle, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        rustix_io::pread(host_fd(descriptor)?, bytes, offset).map_err(io::Error::from)
    }
}

#[cfg(unix)]
fn host_fd(handle: &FsHandle) -> io::Result<&OwnedFd> {
    match &handle.0 {
        HandleInner::Host(fd) => Ok(fd.as_ref()),
        HandleInner::Memory(_) => Err(invalid_input("host filesystem received a memory handle")),
    }
}

#[cfg(unix)]
fn resolve_flags(policy: ResolvePolicy) -> ResolveFlags {
    match policy {
        ResolvePolicy::Strict => {
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS
        }
        ResolvePolicy::Provider => ResolveFlags::NO_MAGICLINKS,
    }
}

#[cfg(unix)]
fn normal_component(component: Component<'_>) -> Option<&OsStr> {
    match component {
        Component::Normal(value) => Some(value),
        _ => None,
    }
}

fn validate_relative(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_input(
            "relative path must contain normal components only",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn rustix_file_kind(mode: u32) -> FileKind {
    match RustixFileType::from_raw_mode(mode) {
        RustixFileType::RegularFile => FileKind::Regular,
        RustixFileType::Directory => FileKind::Directory,
        RustixFileType::Symlink => FileKind::Symlink,
        _ => FileKind::Other,
    }
}

struct MemoryHandle {
    fs: Arc<Mutex<MemoryState>>,
    inode: u64,
}

struct MemoryNode {
    metadata: FileMetadata,
    bytes: Vec<u8>,
    target: Option<u64>,
}

struct MemoryNodeSpec {
    kind: FileKind,
    bytes: Vec<u8>,
    mode: u32,
    modified: Timestamp,
    changed: Timestamp,
    target: Option<u64>,
}

struct MemoryState {
    next_inode: u64,
    nodes: BTreeMap<u64, MemoryNode>,
    children: BTreeMap<(u64, Vec<u8>), u64>,
    route: OpenRoute,
    open_records: Vec<OpenRecord>,
    provider_open_count: usize,
    openat2_error: Option<i32>,
    short_read: Option<usize>,
    mutate_metadata_after_pread: bool,
}

/// A deterministic filesystem fake. It records the exact open policy and
/// models descriptor identity without touching a host path.
#[derive(Clone)]
pub struct InMemoryFileSystem {
    state: Arc<Mutex<MemoryState>>,
}

impl fmt::Debug for InMemoryFileSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InMemoryFileSystem(..)")
    }
}

pub type FakeFileSystem = InMemoryFileSystem;

impl Default for InMemoryFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryFileSystem {
    pub fn new() -> Self {
        Self::with_route(OpenRoute::OpenAt2)
    }

    pub fn with_route(route: OpenRoute) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            1,
            MemoryNode {
                metadata: FileMetadata::new(
                    1,
                    1,
                    0,
                    0o755,
                    FileKind::Directory,
                    Timestamp::new(0, 0),
                    Timestamp::new(0, 0),
                ),
                bytes: Vec::new(),
                target: None,
            },
        );
        Self {
            state: Arc::new(Mutex::new(MemoryState {
                next_inode: 2,
                nodes,
                children: BTreeMap::new(),
                route,
                open_records: Vec::new(),
                provider_open_count: 0,
                openat2_error: None,
                short_read: None,
                mutate_metadata_after_pread: false,
            })),
        }
    }

    pub fn root(&self) -> FsHandle {
        FsHandle(HandleInner::Memory(Arc::new(MemoryHandle {
            fs: Arc::clone(&self.state),
            inode: 1,
        })))
    }

    pub fn add_directory(&self, parent: &FsHandle, name: &OsStr) -> io::Result<FsHandle> {
        self.add_node_with_spec(
            parent,
            name,
            MemoryNodeSpec {
                kind: FileKind::Directory,
                bytes: Vec::new(),
                mode: 0o755,
                modified: Timestamp::new(0, 0),
                changed: Timestamp::new(0, 0),
                target: None,
            },
        )
    }

    pub fn add_file(
        &self,
        parent: &FsHandle,
        name: &OsStr,
        bytes: &[u8],
        mode: u32,
        modified: Timestamp,
        changed: Timestamp,
    ) -> io::Result<FsHandle> {
        self.add_node_with_spec(
            parent,
            name,
            MemoryNodeSpec {
                kind: FileKind::Regular,
                bytes: bytes.to_vec(),
                mode,
                modified,
                changed,
                target: None,
            },
        )
    }

    pub fn add_symlink(
        &self,
        parent: &FsHandle,
        name: &OsStr,
        target: &FsHandle,
    ) -> io::Result<FsHandle> {
        self.add_node_with_spec(
            parent,
            name,
            MemoryNodeSpec {
                kind: FileKind::Symlink,
                bytes: Vec::new(),
                mode: 0o777,
                modified: Timestamp::new(0, 0),
                changed: Timestamp::new(0, 0),
                target: Some(memory_inode(target)?),
            },
        )
    }

    pub fn set_route(&self, route: OpenRoute) {
        if let Ok(mut state) = self.state.lock() {
            state.route = route;
        }
    }

    pub fn set_openat2_error(&self, errno: Option<i32>) {
        if let Ok(mut state) = self.state.lock() {
            state.openat2_error = errno;
        }
    }

    pub fn set_short_read(&self, count: Option<usize>) {
        if let Ok(mut state) = self.state.lock() {
            state.short_read = count;
        }
    }

    pub fn set_metadata_change_after_pread(&self, enabled: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.mutate_metadata_after_pread = enabled;
        }
    }

    pub fn set_mode(&self, descriptor: &FsHandle, mode: u32) {
        if let Ok(mut state) = self.state.lock()
            && let Some(inode) = descriptor_memory_inode(descriptor)
            && let Some(node) = state.nodes.get_mut(&inode)
        {
            node.metadata.mode = mode;
        }
    }

    pub fn set_provider_mode(&self, provider: &ProviderHandle, mode: u32) {
        self.set_mode(&provider.descriptor, mode);
    }

    pub fn rebind(
        &self,
        parent: &FsHandle,
        name: &OsStr,
        replacement: &FsHandle,
    ) -> io::Result<()> {
        let parent_inode = memory_inode(parent)?;
        let replacement_inode = memory_inode(replacement)?;
        let mut state = self.state.lock().map_err(lock_error)?;
        let key = (parent_inode, os_bytes(name));
        if !state.children.contains_key(&key) {
            return Err(errno(ErrnoKind::Noent));
        }
        state.children.insert(key, replacement_inode);
        Ok(())
    }

    pub fn provider_open_count(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.provider_open_count)
            .unwrap_or_default()
    }

    pub fn open_records(&self) -> Vec<OpenRecord> {
        self.state
            .lock()
            .map(|state| state.open_records.clone())
            .unwrap_or_default()
    }

    fn add_node_with_spec(
        &self,
        parent: &FsHandle,
        name: &OsStr,
        spec: MemoryNodeSpec,
    ) -> io::Result<FsHandle> {
        let parent_inode = memory_inode(parent)?;
        let mut state = self.state.lock().map_err(lock_error)?;
        let key = (parent_inode, os_bytes(name));
        if state.children.contains_key(&key) {
            return Err(errno(ErrnoKind::Access));
        }
        let inode = state.next_inode;
        state.next_inode = state.next_inode.saturating_add(1);
        state.nodes.insert(
            inode,
            MemoryNode {
                metadata: FileMetadata::new(
                    1,
                    inode,
                    spec.bytes.len() as u64,
                    spec.mode,
                    spec.kind,
                    spec.modified,
                    spec.changed,
                ),
                bytes: spec.bytes,
                target: spec.target,
            },
        );
        state.children.insert(key, inode);
        Ok(FsHandle(HandleInner::Memory(Arc::new(MemoryHandle {
            fs: Arc::clone(&self.state),
            inode,
        }))))
    }

    fn open_memory(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OpenFlags,
        policy: ResolvePolicy,
        route: OpenRoute,
    ) -> io::Result<FsHandle> {
        validate_relative(relative)?;
        let mut state = self.state.lock().map_err(lock_error)?;
        #[cfg(unix)]
        state.open_records.push(OpenRecord {
            route,
            policy,
            flags,
            resolve_flags: ResolveMask::for_policy(policy),
            intermediate_no_follow: route == OpenRoute::ComponentWalk
                && relative.components().count() > 1,
        });
        if route == OpenRoute::OpenAt2
            && let Some(error) = state.openat2_error
        {
            return Err(io::Error::from_raw_os_error(error));
        }
        let inode = resolve_memory_path(&state, memory_inode(anchor)?, relative, policy)?;
        let node = state
            .nodes
            .get(&inode)
            .ok_or_else(|| errno(ErrnoKind::Noent))?;
        #[cfg(unix)]
        if flags.contains(OFlags::DIRECTORY) && node.metadata.kind != FileKind::Directory {
            return Err(errno(ErrnoKind::Notdir));
        }
        Ok(FsHandle(HandleInner::Memory(Arc::new(MemoryHandle {
            fs: Arc::clone(&self.state),
            inode,
        }))))
    }
}

#[cfg(unix)]
impl FileSystem for InMemoryFileSystem {
    fn open(&self, path: &Path, flags: OFlags, policy: ResolvePolicy) -> io::Result<FsHandle> {
        if path.is_absolute() {
            return Ok(self.root());
        }
        self.open_memory(&self.root(), path, flags, policy, OpenRoute::OpenAt2)
    }

    fn openat2(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        self.open_memory(anchor, relative, flags, policy, OpenRoute::OpenAt2)
    }

    fn open_component_walk(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        self.open_memory(anchor, relative, flags, policy, OpenRoute::ComponentWalk)
    }

    fn open_provider(&self, anchor: &FsHandle, relative: &Path) -> io::Result<ProviderHandle> {
        if let Ok(mut state) = self.state.lock() {
            state.provider_open_count = state.provider_open_count.saturating_add(1);
        }
        let flags = OFlags::RDONLY | OFlags::CLOEXEC;
        let descriptor = match self
            .state
            .lock()
            .map(|state| state.route)
            .unwrap_or_default()
        {
            OpenRoute::OpenAt2 => {
                match self.openat2(anchor, relative, flags, ResolvePolicy::Provider) {
                    Err(error)
                        if error.raw_os_error() == Some(rustix_io::Errno::NOSYS.raw_os_error()) =>
                    {
                        self.open_component_walk(anchor, relative, flags, ResolvePolicy::Provider)?
                    }
                    result => result?,
                }
            }
            OpenRoute::ComponentWalk => {
                self.open_component_walk(anchor, relative, flags, ResolvePolicy::Provider)?
            }
        };
        Ok(ProviderHandle { descriptor })
    }

    fn fstat(&self, descriptor: &FsHandle) -> io::Result<FileMetadata> {
        let inode = memory_inode(descriptor)?;
        self.state
            .lock()
            .map_err(lock_error)?
            .nodes
            .get(&inode)
            .map(|node| node.metadata)
            .ok_or_else(|| errno(ErrnoKind::Noent))
    }

    fn pread(&self, descriptor: &FsHandle, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        let inode = memory_inode(descriptor)?;
        let mut state = self.state.lock().map_err(lock_error)?;
        let short_read = state.short_read;
        let mutate = state.mutate_metadata_after_pread;
        let node = state
            .nodes
            .get_mut(&inode)
            .ok_or_else(|| errno(ErrnoKind::Noent))?;
        if node.metadata.kind != FileKind::Regular {
            return Err(errno(ErrnoKind::Isdir));
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let available = node.bytes.len().saturating_sub(start);
        let count = available
            .min(bytes.len())
            .min(short_read.unwrap_or(usize::MAX));
        if count != 0 {
            bytes[..count].copy_from_slice(&node.bytes[start..start + count]);
        }
        if mutate {
            node.metadata.modified.seconds = node.metadata.modified.seconds.saturating_add(1);
            node.metadata.changed.seconds = node.metadata.changed.seconds.saturating_add(1);
        }
        Ok(count)
    }
}

fn resolve_memory_path(
    state: &MemoryState,
    anchor: u64,
    relative: &Path,
    policy: ResolvePolicy,
) -> io::Result<u64> {
    let components = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.is_empty() {
        return Err(invalid_input("relative path is empty"));
    }
    let mut current = anchor;
    for (index, component) in components.iter().enumerate() {
        let child = state
            .children
            .get(&(current, os_bytes(component)))
            .copied()
            .ok_or_else(|| errno(ErrnoKind::Noent))?;
        let node = state
            .nodes
            .get(&child)
            .ok_or_else(|| errno(ErrnoKind::Noent))?;
        let leaf = index + 1 == components.len();
        if node.metadata.kind == FileKind::Symlink {
            if !leaf || policy == ResolvePolicy::Strict {
                return Err(errno(ErrnoKind::Loop));
            }
            return node.target.ok_or_else(|| errno(ErrnoKind::Noent));
        }
        if !leaf && node.metadata.kind != FileKind::Directory {
            return Err(errno(ErrnoKind::Notdir));
        }
        current = child;
    }
    Ok(current)
}

fn memory_inode(handle: &FsHandle) -> io::Result<u64> {
    descriptor_memory_inode(handle)
        .ok_or_else(|| invalid_input("memory filesystem received a host handle"))
}

fn descriptor_memory_inode(handle: &FsHandle) -> Option<u64> {
    match &handle.0 {
        HandleInner::Memory(value) => Some(value.inode),
        #[cfg(unix)]
        HandleInner::Host(_) => None,
    }
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[derive(Clone, Copy)]
enum ErrnoKind {
    Access,
    Isdir,
    Loop,
    Noent,
    Notdir,
}

fn errno(kind: ErrnoKind) -> io::Error {
    #[cfg(unix)]
    let value = match kind {
        ErrnoKind::Access => rustix_io::Errno::ACCESS,
        ErrnoKind::Isdir => rustix_io::Errno::ISDIR,
        ErrnoKind::Loop => rustix_io::Errno::LOOP,
        ErrnoKind::Noent => rustix_io::Errno::NOENT,
        ErrnoKind::Notdir => rustix_io::Errno::NOTDIR,
    };
    #[cfg(unix)]
    return io::Error::from_raw_os_error(value.raw_os_error());
    #[cfg(not(unix))]
    {
        let _ = kind;
        io::Error::other("filesystem operation is unavailable")
    }
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("filesystem fake lock poisoned")
}
