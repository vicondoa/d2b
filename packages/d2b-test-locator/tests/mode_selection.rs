#[path = "../src/mode.rs"]
mod mode;

use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use d2b_bazel_support::{
    fsops::{
        Digest, FileMetadata, FileSystem, FsHandle, InMemoryFileSystem, OpenFlags, ProviderHandle,
        ResolvePolicy,
    },
    runfiles::{InMemoryRunfilesView, RunfilesLookup, RunfilesMode, RunfilesView},
};
use d2b_test_locator::{LocatorError, bazel_binary};

macro_rules! env {
    ($($tokens:tt)*) => {
        "/caller/anchor/provider"
    };
}

#[derive(Debug)]
struct CountingRunfiles {
    selected: Cell<RunfilesMode>,
    mode_reads: Cell<usize>,
}

impl CountingRunfiles {
    fn new(mode: RunfilesMode) -> Self {
        Self {
            selected: Cell::new(mode),
            mode_reads: Cell::new(0),
        }
    }

    fn set_mode(&self, mode: RunfilesMode) {
        self.selected.set(mode);
    }

    fn mode_reads(&self) -> usize {
        self.mode_reads.get()
    }
}

impl RunfilesView for CountingRunfiles {
    fn mode(&self) -> RunfilesMode {
        self.mode_reads.set(self.mode_reads.get() + 1);
        self.selected.get()
    }

    fn lookup(&self, _relative: &Path) -> RunfilesLookup {
        RunfilesLookup::NotBazel
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArmError {
    BazelMiss,
}

#[test]
fn mode_is_read_once_and_the_selection_is_stable() {
    let runfiles = CountingRunfiles::new(RunfilesMode::Bazel);
    let selection = mode::ModeSelection::select(&runfiles);

    runfiles.set_mode(RunfilesMode::Cargo);

    assert_eq!(selection.mode(), RunfilesMode::Bazel);
    assert_eq!(runfiles.mode_reads(), 1);
    assert_eq!(
        selection.resolve(|| Ok::<_, ()>("bazel"), || Ok::<_, ()>("cargo")),
        Ok("bazel")
    );
    assert_eq!(runfiles.mode_reads(), 1);
}

#[test]
fn bazel_mode_miss_is_refused_without_a_cargo_fallback() {
    let runfiles = CountingRunfiles::new(RunfilesMode::Bazel);
    let selection = mode::ModeSelection::select(&runfiles);
    let cargo_called = Cell::new(false);

    let result = selection.resolve(
        || Err(ArmError::BazelMiss),
        || {
            cargo_called.set(true);
            Ok("stale-cargo-provider")
        },
    );

    assert_eq!(result, Err(ArmError::BazelMiss));
    assert!(!cargo_called.get());
}

#[test]
fn cargo_binary_macro_expands_the_environment_at_the_call_site() {
    let (filesystem, digest) = fixture();
    let runfiles = InMemoryRunfilesView::cargo();

    let executable =
        d2b_test_locator::cargo_binary!(&filesystem, &runfiles, "caller_binary", digest)
            .expect("the caller-provided Cargo path should locate the provider");

    assert_eq!(
        filesystem
            .provider_paths
            .lock()
            .expect("provider paths")
            .as_slice(),
        [PathBuf::from("provider")]
    );
    drop(executable);
}

#[test]
fn selected_mode_dispatches_one_arm_without_chaining() {
    let runfiles = CountingRunfiles::new(RunfilesMode::Cargo);
    let selection = mode::ModeSelection::select(&runfiles);
    let bazel_called = Cell::new(false);
    let cargo_called = Cell::new(false);

    let result = selection.resolve(
        || {
            bazel_called.set(true);
            Err(ArmError::BazelMiss)
        },
        || {
            cargo_called.set(true);
            Ok("cargo-provider")
        },
    );

    assert_eq!(result, Ok("cargo-provider"));
    assert!(!bazel_called.get());
    assert!(cargo_called.get());
}

#[test]
fn bazel_binary_reports_a_missing_declared_entry_instead_of_using_cargo() {
    let (filesystem, digest) = fixture();
    let runfiles = InMemoryRunfilesView::present("/runfiles/root", []);

    let error = match bazel_binary(&filesystem, &runfiles, Path::new("provider"), digest) {
        Ok(_) => panic!("a missing Bazel entry must refuse"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        LocatorError::RunfilesEntryMissing { ref relative } if relative == Path::new("provider")
    ));
    assert_eq!(filesystem.inner.provider_open_count(), 0);
}

struct AnchoredFileSystem {
    inner: InMemoryFileSystem,
    provider_paths: Mutex<Vec<PathBuf>>,
}

impl FileSystem for AnchoredFileSystem {
    fn open(
        &self,
        _path: &Path,
        _flags: OpenFlags,
        _policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        Ok(self.inner.root())
    }

    fn openat2(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OpenFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        self.inner.openat2(anchor, relative, flags, policy)
    }

    fn open_component_walk(
        &self,
        anchor: &FsHandle,
        relative: &Path,
        flags: OpenFlags,
        policy: ResolvePolicy,
    ) -> io::Result<FsHandle> {
        self.inner
            .open_component_walk(anchor, relative, flags, policy)
    }

    fn open_provider(&self, anchor: &FsHandle, relative: &Path) -> io::Result<ProviderHandle> {
        self.provider_paths
            .lock()
            .expect("provider paths")
            .push(relative.to_owned());
        self.inner.open_provider(anchor, relative)
    }

    fn fstat(&self, descriptor: &FsHandle) -> io::Result<FileMetadata> {
        self.inner.fstat(descriptor)
    }

    fn pread(&self, descriptor: &FsHandle, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        self.inner.pread(descriptor, bytes, offset)
    }
}

fn fixture() -> (AnchoredFileSystem, Digest) {
    let inner = InMemoryFileSystem::new();
    let root = inner.root();
    inner
        .add_file(
            &root,
            std::ffi::OsStr::new("provider"),
            b"verified-provider",
            0o755,
            d2b_bazel_support::fsops::Timestamp::new(10, 0),
            d2b_bazel_support::fsops::Timestamp::new(10, 0),
        )
        .expect("provider fixture");
    (
        AnchoredFileSystem {
            inner,
            provider_paths: Mutex::new(Vec::new()),
        },
        Digest::sha256(b"verified-provider"),
    )
}
