#![forbid(unsafe_code)]

use std::{
    error::Error,
    fmt, io,
    path::{Component, Path},
};

use d2b_bazel_exec::{ProviderError, VerifiedExecutable};
use d2b_bazel_support::runfiles::RunfilesLookup;

mod mode;

pub use d2b_bazel_support::{
    fsops::{FileSystem, OpenFlags, ResolvePolicy},
    runfiles::{RunfilesMode, RunfilesView},
};
pub use mode::ModeSelection;

/// Errors from one selected provider arm. A Bazel miss never falls back to
/// Cargo. All renderings are fixed and contain no supplied path or I/O text.
pub enum LocatorError {
    Io,
    Verification,
    BazelRunfilesUnavailable,
    RunfilesEntryMissing,
    InvalidRunfilesEntry,
    CargoArmSelectedInBazelMode,
    InvalidCargoExecutable,
    ExecutionAuthorityUnavailable,
}

impl LocatorError {
    const fn code(&self) -> &'static str {
        match self {
            Self::Io => "D2B-BZLEXEC-LOCATOR-IO",
            Self::Verification => "D2B-BZLEXEC-LOCATOR-VERIFICATION",
            Self::BazelRunfilesUnavailable => "D2B-BZLEXEC-LOCATOR-BAZEL-ENV",
            Self::RunfilesEntryMissing => "D2B-BZLEXEC-LOCATOR-RUNFILES",
            Self::InvalidRunfilesEntry => "D2B-BZLEXEC-LOCATOR-RELATIVE",
            Self::CargoArmSelectedInBazelMode => "D2B-BZLEXEC-LOCATOR-CARGO-ARM",
            Self::InvalidCargoExecutable => "D2B-BZLEXEC-LOCATOR-CARGO-EXECUTABLE",
            Self::ExecutionAuthorityUnavailable => "D2B-BZLEXEC-PROVIDER-AUTHORITY",
        }
    }
}

impl fmt::Debug for LocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl fmt::Display for LocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for LocatorError {}

impl From<io::Error> for LocatorError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

impl From<ProviderError> for LocatorError {
    fn from(_: ProviderError) -> Self {
        Self::Verification
    }
}

pub type LocatorResult = Result<VerifiedExecutable, LocatorError>;

/// Resolve the declared Bazel runfiles entry and open it under the strict
/// directory anchor. The capability mint is intentionally not reachable from
/// this downstream crate until the immutable toolchain authority is wired.
pub fn bazel_binary<F, R>(filesystem: &F, runfiles: &R, relative: &Path) -> LocatorResult
where
    F: FileSystem,
    R: RunfilesView,
{
    validate_relative(relative)?;
    if runfiles.mode() != RunfilesMode::Bazel {
        return Err(LocatorError::BazelRunfilesUnavailable);
    }
    let location = match runfiles.lookup(relative) {
        RunfilesLookup::Present(location) => location,
        RunfilesLookup::Missing | RunfilesLookup::NotBazel => {
            return Err(LocatorError::RunfilesEntryMissing);
        }
    };
    validate_relative(&location.relative)?;
    let anchor = filesystem.open(&location.anchor, directory_flags(), ResolvePolicy::Strict)?;
    let _provider = filesystem.open_provider(&anchor, &location.relative)?;
    Err(LocatorError::ExecutionAuthorityUnavailable)
}

fn validate_relative(path: &Path) -> Result<(), LocatorError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LocatorError::InvalidRunfilesEntry);
    }
    Ok(())
}

#[cfg(unix)]
fn directory_flags() -> OpenFlags {
    OpenFlags::RDONLY | OpenFlags::DIRECTORY | OpenFlags::CLOEXEC
}

#[cfg(not(unix))]
fn directory_flags() -> OpenFlags {
    OpenFlags
}

/// Retain the call-site syntax for the later authority-owned Cargo lane
/// without expanding a path or environment value into a diagnostic.
#[macro_export]
macro_rules! cargo_binary {
    ($filesystem:expr, $runfiles:expr, $binary:literal, $expected_digest:expr $(,)?) => {{
        let __d2b_filesystem = $filesystem;
        let __d2b_runfiles = $runfiles;
        let __d2b_result: $crate::LocatorResult = (|| {
            let _ = (
                &__d2b_filesystem,
                &__d2b_runfiles,
                stringify!($binary),
                &$expected_digest,
            );
            if $crate::RunfilesView::mode(__d2b_runfiles) == $crate::RunfilesMode::Bazel {
                return Err($crate::LocatorError::CargoArmSelectedInBazelMode);
            }
            Err($crate::LocatorError::ExecutionAuthorityUnavailable)
        })();
        __d2b_result
    }};
}
