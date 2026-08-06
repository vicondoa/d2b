//! Injectable boundaries for Provider artifact layout validation and launch.
//!
//! This module deliberately contains no Linux syscall implementation. The
//! resource compiler and runtime launcher will provide adapters that implement
//! these traits in their owning crates. Keeping the handles split by authority
//! makes it impossible for a launcher implementation to read an executable
//! through the interface it receives.

use std::{
    convert::Infallible,
    ffi::{OsStr, OsString},
    fmt,
};

/// The raw SHA-256 bytes returned by a readable artifact handle.
///
/// The contracts crate owns the canonical `sha256:<hex>` wire spelling. This
/// low-level seam stays independent of that crate so the dependency direction
/// remains `d2b-contracts -> d2b-core`.
pub type ArtifactDigest = [u8; 32];

/// A layout-relative path beneath an already anchored artifact directory.
///
/// Validation of the path and the kernel resolution flags belong to the
/// adapter. The trait boundary accepts an owned value so a future adapter can
/// retain no caller-owned path buffer.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LayoutPath(String);

impl LayoutPath {
    /// Construct a layout-relative path for an injected adapter.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Borrow the path for an adapter's validation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LayoutPath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for LayoutPath {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

impl AsRef<OsStr> for LayoutPath {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(&self.0)
    }
}

impl fmt::Debug for LayoutPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LayoutPath(<redacted>)")
    }
}

/// A layout-relative directory name beneath an anchored artifact directory.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LayoutDir(String);

impl LayoutDir {
    /// Construct a layout-relative directory name for an injected adapter.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Borrow the directory name for an adapter's validation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LayoutDir {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for LayoutDir {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

impl AsRef<OsStr> for LayoutDir {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(&self.0)
    }
}

impl fmt::Debug for LayoutDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LayoutDir(<redacted>)")
    }
}

/// Opaque argument vector passed to an already selected Provider executable.
///
/// The seam does not interpret or log argument contents. The eventual runtime
/// adapter owns the argv encoding and forwards only the fixed launch contract.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Argv(Vec<OsString>);

impl Argv {
    /// Construct an argument vector for an injected launcher.
    pub fn new(arguments: impl IntoIterator<Item = OsString>) -> Self {
        Self(arguments.into_iter().collect())
    }

    /// Borrow the arguments without exposing them through `Debug`.
    pub fn as_slice(&self) -> &[OsString] {
        &self.0
    }
}

impl fmt::Debug for Argv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Argv(<redacted>)")
    }
}

/// Opaque environment vector passed to an already selected Provider
/// executable.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Envp(Vec<OsString>);

impl Envp {
    /// Construct an environment vector for an injected launcher.
    pub fn new(entries: impl IntoIterator<Item = OsString>) -> Self {
        Self(entries.into_iter().collect())
    }

    /// Borrow the environment entries without exposing them through `Debug`.
    pub fn as_slice(&self) -> &[OsString] {
        &self.0
    }
}

impl fmt::Debug for Envp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Envp(<redacted>)")
    }
}

/// Failure while resolving or validating a Provider artifact layout.
///
/// These variants intentionally carry no path, errno text, or file contents.
/// Callers map them to the stable Provider artifact error taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    /// Resolution escaped the anchored artifact directory (`EXDEV`).
    NotBeneath,
    /// A symlink or magic link was refused (`ELOOP`).
    SymlinkRefused,
    /// The opened descriptor was not a regular file.
    NotRegular,
    /// The regular file did not have an execute bit.
    NotExecutable,
    /// The bounded prefix was not an admissible ELF image.
    NotElf,
    /// Opening a special node returned `ENXIO`.
    NoDevice,
    /// The required layout entry was absent (`ENOENT` at open).
    Absent,
}

impl LayoutError {
    /// Return the stable internal reason token.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotBeneath => "not-beneath",
            Self::SymlinkRefused => "symlink-refused",
            Self::NotRegular => "not-regular",
            Self::NotExecutable => "not-executable",
            Self::NotElf => "not-elf",
            Self::NoDevice => "no-device",
            Self::Absent => "absent",
        }
    }
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for LayoutError {}

/// Failure returned by an already-opened executable launch attempt.
///
/// `InterpreterUnresolvable` is deliberately distinct from
/// [`LayoutError::Absent`]: the former is an `ENOENT` from `execveat` after a
/// successful open, while the latter is an `ENOENT` during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// The kernel rejected the image format (`ENOEXEC`).
    FormatRejected,
    /// The kernel refused execution (`EACCES`).
    PermissionDenied,
    /// The image's interpreter could not be resolved (`ENOENT` from exec).
    InterpreterUnresolvable,
}

impl LaunchError {
    /// Return the stable internal reason token.
    pub const fn code(self) -> &'static str {
        match self {
            Self::FormatRejected => "format-rejected",
            Self::PermissionDenied => "permission-denied",
            Self::InterpreterUnresolvable => "interpreter-unresolvable",
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for LaunchError {}

/// A directory already anchored to one Provider artifact output.
///
/// Implementations must use the two distinct open modes required by ADR 0050:
/// readable handles use `O_RDONLY | O_NONBLOCK | O_CLOEXEC`, while executable
/// handles use `O_PATH | O_CLOEXEC`. No implementation is provided here.
pub trait AnchoredDir: Send + Sync {
    /// Readable descriptor authority used by the resource compiler.
    type Readable: ReadableFile;
    /// Executable descriptor authority used by the runtime launcher.
    type Executable: ExecutableFile;

    /// Resolve one layout-relative path to a readable regular file.
    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError>;

    /// Resolve one layout-relative path to an executable reference.
    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError>;

    /// Enumerate one closed layout-relative directory.
    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError>;
}

/// A readable, descriptor-verified regular file.
pub trait ReadableFile: Send {
    /// Return the descriptor's byte length established by `fstat`.
    fn len(&self) -> u64;

    /// Whether the descriptor contains no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read a bounded prefix from the already-open descriptor.
    fn read_prefix(&mut self, out: &mut [u8]) -> Result<usize, LayoutError>;

    /// Consume the descriptor and hash its bytes.
    fn read_to_digest(self) -> Result<ArtifactDigest, LayoutError>;
}

/// An executable-only descriptor reference.
///
/// This trait intentionally exposes no read method. A launcher cannot obtain
/// readable authority through the handle it receives.
pub trait ExecutableFile: Send {}

/// Launch a program from an already-opened executable reference.
pub trait ProcessLauncher: Send + Sync {
    /// The launcher accepts only executable authority, never a readable handle.
    type Executable: ExecutableFile;

    /// Consume the reference and attempt `execveat(AT_EMPTY_PATH)`.
    fn exec_from(
        &self,
        file: Self::Executable,
        argv: &Argv,
        envp: &Envp,
    ) -> Result<Infallible, LaunchError>;
}
