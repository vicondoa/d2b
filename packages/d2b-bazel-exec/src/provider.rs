use std::{fmt, io};

#[cfg(unix)]
use std::os::fd::OwnedFd;

/// An opaque, compiler-derived capability for one admitted executable
/// descriptor.
///
/// The type has no public inherent API. It has no path, descriptor, formatting,
/// conversion, duplication, default, or serialization surface. The only
/// operation available to a caller is passing the value to the sole consuming
/// execution API. Production admission is owned by the later immutable
/// toolchain lane; this crate deliberately has no unchecked path-plus-digest
/// constructor.
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// let _ = VerifiedExecutable::new();
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// fn cannot_extract(executable: VerifiedExecutable) {
///     let _ = executable.as_raw_fd();
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// fn cannot_format(executable: VerifiedExecutable) {
///     let _ = format!("{executable:?}");
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// fn cannot_display(executable: VerifiedExecutable) {
///     let _ = format!("{executable}");
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
/// use std::borrow::Borrow;
/// use std::os::fd::{AsFd, AsRawFd, OwnedFd};
///
/// fn cannot_access(executable: VerifiedExecutable) {
///     let _: &OwnedFd = executable.borrow();
///     let _ = executable.as_fd();
///     let _ = executable.as_raw_fd();
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// fn cannot_duplicate(executable: VerifiedExecutable) {
///     let _ = executable.clone();
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// let _: VerifiedExecutable = Default::default();
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// fn requires_serialization<T: serde::Serialize>(_: &T) {}
/// fn cannot_serialize(executable: VerifiedExecutable) {
///     requires_serialization(&executable);
/// }
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
/// use std::path::PathBuf;
///
/// let _ = VerifiedExecutable::from(PathBuf::from("provider"));
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
///
/// impl d2b_bazel_exec::provider::VerifiedExecutableMint for VerifiedExecutable {}
/// ```
///
/// ```compile_fail
/// use d2b_bazel_exec::VerifiedExecutable;
/// use std::ops::Deref;
///
/// fn no_deref(executable: VerifiedExecutable) {
///     let _: &std::path::Path = executable.deref();
/// }
/// ```
pub struct VerifiedExecutable {
    #[cfg(unix)]
    provider: OwnedFd,
    #[cfg(not(unix))]
    provider: (),
}

#[allow(dead_code)]
pub(crate) trait VerifiedExecutableMint {}

impl VerifiedExecutable {
    #[cfg(unix)]
    pub(crate) fn duplicate_for_mapping(&self) -> io::Result<OwnedFd> {
        rustix::io::fcntl_dupfd_cloexec(&self.provider, 3).map_err(io::Error::from)
    }
}

#[cfg(test)]
pub(crate) fn verified_executable_for_test(file: std::fs::File) -> VerifiedExecutable {
    #[cfg(unix)]
    {
        VerifiedExecutable {
            provider: file.into(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        VerifiedExecutable { provider: () }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderError {
    AuthorityUnavailable,
    UnsupportedPlatform,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorityUnavailable => formatter.write_str("D2B-BZLEXEC-PROVIDER-AUTHORITY"),
            Self::UnsupportedPlatform => formatter.write_str("D2B-BZLEXEC-NIX-PTRACE-SYSTEM"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Closed classification for the `execveat` failure surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecErrno {
    Enosys,
    Eacces,
    Enoexec,
    Enoent,
    Etxtbsy,
    Other,
}

pub fn classify_exec_error(error: &io::Error) -> ExecErrno {
    match error.raw_os_error() {
        Some(value) if value == rustix::io::Errno::NOSYS.raw_os_error() => ExecErrno::Enosys,
        Some(value) if value == rustix::io::Errno::ACCESS.raw_os_error() => ExecErrno::Eacces,
        Some(value) if value == rustix::io::Errno::NOEXEC.raw_os_error() => ExecErrno::Enoexec,
        Some(value) if value == rustix::io::Errno::NOENT.raw_os_error() => ExecErrno::Enoent,
        Some(value) if value == rustix::io::Errno::TXTBSY.raw_os_error() => ExecErrno::Etxtbsy,
        _ => ExecErrno::Other,
    }
}
