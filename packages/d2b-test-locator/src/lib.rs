#![forbid(unsafe_code)]

use std::{
    error::Error,
    fmt, io,
    path::{Component, Path, PathBuf},
};

use d2b_bazel_exec::{ProviderError, VerifiedExecutable, provider::verify_provider};
use d2b_bazel_support::runfiles::RunfilesLookup;

pub use d2b_bazel_support::{
    fsops::{FileSystem, OpenFlags, ResolvePolicy},
    runfiles::{RunfilesMode, RunfilesView},
};

/// Errors from one selected provider arm. A Bazel miss never falls back to
/// Cargo.
#[derive(Debug)]
pub enum LocatorError {
    Io(io::Error),
    Verification(ProviderError),
    BazelRunfilesUnavailable,
    RunfilesEntryMissing { relative: PathBuf },
    InvalidRunfilesEntry,
    CargoArmSelectedInBazelMode,
    InvalidCargoExecutable,
}

impl fmt::Display for LocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("provider filesystem operation failed"),
            Self::Verification(error) => error.fmt(formatter),
            Self::BazelRunfilesUnavailable => {
                formatter.write_str("Bazel runfiles environment is unavailable")
            }
            Self::RunfilesEntryMissing { .. } => {
                formatter.write_str("declared runfiles entry is missing")
            }
            Self::InvalidRunfilesEntry => {
                formatter.write_str("declared runfiles entry must be relative")
            }
            Self::CargoArmSelectedInBazelMode => {
                formatter.write_str("Cargo provider arm is unavailable in Bazel mode")
            }
            Self::InvalidCargoExecutable => {
                formatter.write_str("Cargo binary provider has no valid anchor")
            }
        }
    }
}

impl Error for LocatorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Verification(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for LocatorError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ProviderError> for LocatorError {
    fn from(error: ProviderError) -> Self {
        Self::Verification(error)
    }
}

pub type LocatorResult = Result<VerifiedExecutable, LocatorError>;

pub fn bazel_binary<F, R>(
    filesystem: &F,
    runfiles: &R,
    relative: &Path,
    expected_digest: impl AsRef<[u8]>,
) -> LocatorResult
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
            return Err(LocatorError::RunfilesEntryMissing {
                relative: relative.to_owned(),
            });
        }
    };
    validate_relative(&location.relative)?;
    let anchor = filesystem.open(&location.anchor, directory_flags(), ResolvePolicy::Strict)?;
    let provider = filesystem.open_provider(&anchor, &location.relative)?;
    verify_provider(filesystem, provider, None, expected_digest).map_err(Into::into)
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

/// Expand a Cargo integration-test binary environment value at its call site.
#[macro_export]
macro_rules! cargo_binary {
    ($filesystem:expr, $runfiles:expr, $binary:literal, $expected_digest:expr $(,)?) => {{
        let __d2b_filesystem = $filesystem;
        let __d2b_runfiles = $runfiles;
        let __d2b_result: $crate::LocatorResult = (|| {
            if $crate::RunfilesView::mode(__d2b_runfiles) == $crate::RunfilesMode::Bazel {
                return Err($crate::LocatorError::CargoArmSelectedInBazelMode);
            }
            let __d2b_provider_path =
                ::std::path::Path::new(env!(concat!("CARGO_BIN_EXE_", $binary)));
            let Some(__d2b_anchor) = __d2b_provider_path.parent() else {
                return Err($crate::LocatorError::InvalidCargoExecutable);
            };
            let Some(__d2b_leaf) = __d2b_provider_path.file_name() else {
                return Err($crate::LocatorError::InvalidCargoExecutable);
            };
            #[cfg(unix)]
            {
                let __d2b_anchor_handle = $crate::FileSystem::open(
                    __d2b_filesystem,
                    __d2b_anchor,
                    $crate::OpenFlags::RDONLY
                        | $crate::OpenFlags::DIRECTORY
                        | $crate::OpenFlags::CLOEXEC,
                    $crate::ResolvePolicy::Strict,
                )?;
                let __d2b_provider = $crate::FileSystem::open_provider(
                    __d2b_filesystem,
                    &__d2b_anchor_handle,
                    ::std::path::Path::new(__d2b_leaf),
                )?;
                ::d2b_bazel_exec::verify_provider(
                    __d2b_filesystem,
                    __d2b_provider,
                    None,
                    $expected_digest,
                )
                .map_err($crate::LocatorError::Verification)
            }
            #[cfg(not(unix))]
            {
                let _ = (__d2b_filesystem, __d2b_leaf);
                Err($crate::LocatorError::BazelRunfilesUnavailable)
            }
        })();
        __d2b_result
    }};
}
