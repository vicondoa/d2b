use std::{fmt, io};

use d2b_bazel_support::fsops::{
    FileSystem, ProviderHandle, VerificationError, VerifiedProvider as SupportVerifiedProvider,
};

/// An opaque, compiler-derived capability for one verified executable
/// descriptor.
///
/// The type has no public inherent API. It has no path, descriptor, formatting,
/// conversion, duplication, default, or serialization surface. The only
/// operation available to a caller is passing the value to
/// [`crate::execute_verified`].
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
    provider: ProviderHandle,
}

#[allow(dead_code)]
pub(crate) trait VerifiedExecutableMint {}

impl VerifiedExecutable {
    pub(crate) fn from_support(value: SupportVerifiedProvider) -> Self {
        let (provider, _metadata, _digest) = value.into_parts();
        Self { provider }
    }

    pub(crate) fn duplicate_for_mapping(&self) -> io::Result<std::os::fd::OwnedFd> {
        self.provider.duplicate_for_mapping()
    }
}

/// Verify a provider descriptor and mint the opaque execution capability.
pub fn verify_provider<F: FileSystem>(
    filesystem: &F,
    provider: ProviderHandle,
    newest_input: Option<&ProviderHandle>,
    expected_digest: impl AsRef<[u8]>,
) -> Result<VerifiedExecutable, ProviderError> {
    d2b_bazel_support::fsops::verify_provider(filesystem, provider, newest_input, expected_digest)
        .map(VerifiedExecutable::from_support)
        .map_err(ProviderError::Verification)
}

#[derive(Debug)]
pub enum ProviderError {
    Verification(VerificationError),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verification(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Verification(error) => Some(error),
        }
    }
}

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
