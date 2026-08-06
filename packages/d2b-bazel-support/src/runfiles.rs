use std::{
    collections::BTreeSet,
    env, io,
    path::{Component, Path, PathBuf},
};

/// The provider mode selected by the process environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunfilesMode {
    Bazel,
    Cargo,
}

/// A runfiles anchor and the declared path below that anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunfilesLocation {
    pub anchor: PathBuf,
    pub relative: PathBuf,
}

/// The result of one runfiles lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunfilesLookup {
    Present(RunfilesLocation),
    Missing,
    NotBazel,
}

/// The injected boundary used by tests and by the locator.
pub trait RunfilesView {
    fn mode(&self) -> RunfilesMode;
    fn lookup(&self, relative: &Path) -> RunfilesLookup;
}

/// Read-only host runfiles lookup.
#[derive(Clone, Debug, Default)]
pub struct HostRunfilesView {
    root: Option<PathBuf>,
    manifest_entries: Option<BTreeSet<PathBuf>>,
}

impl HostRunfilesView {
    pub fn from_env() -> Self {
        let root = env::var_os("TEST_SRCDIR")
            .or_else(|| env::var_os("RUNFILES_DIR"))
            .map(PathBuf::from);
        let manifest_entries = env::var_os("RUNFILES_MANIFEST_FILE")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|contents| {
                contents
                    .lines()
                    .filter_map(|line| line.split_once(' ').map(|(entry, _)| PathBuf::from(entry)))
                    .collect()
            });
        Self {
            root,
            manifest_entries,
        }
    }

    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            manifest_entries: None,
        }
    }
}

impl RunfilesView for HostRunfilesView {
    fn mode(&self) -> RunfilesMode {
        if self.root.is_some() {
            RunfilesMode::Bazel
        } else {
            RunfilesMode::Cargo
        }
    }

    fn lookup(&self, relative: &Path) -> RunfilesLookup {
        let Some(root) = &self.root else {
            return RunfilesLookup::NotBazel;
        };
        if !valid_relative(relative) {
            return RunfilesLookup::Missing;
        }
        if let Some(entries) = &self.manifest_entries {
            if !entries.contains(relative) {
                return RunfilesLookup::Missing;
            }
        } else if !root.join(relative).exists() {
            return RunfilesLookup::Missing;
        }
        RunfilesLookup::Present(RunfilesLocation {
            anchor: root.clone(),
            relative: relative.to_owned(),
        })
    }
}

/// An explicit runfiles fake. It does not inspect a host path.
#[derive(Clone, Debug)]
pub struct InMemoryRunfilesView {
    state: InMemoryRunfilesState,
}

#[derive(Clone, Debug)]
enum InMemoryRunfilesState {
    Present {
        anchor: PathBuf,
        entries: BTreeSet<PathBuf>,
    },
    Missing,
    Cargo,
}

impl InMemoryRunfilesView {
    pub fn present(anchor: impl Into<PathBuf>, entries: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            state: InMemoryRunfilesState::Present {
                anchor: anchor.into(),
                entries: entries.into_iter().collect(),
            },
        }
    }

    pub fn missing() -> Self {
        Self {
            state: InMemoryRunfilesState::Missing,
        }
    }

    pub fn cargo() -> Self {
        Self {
            state: InMemoryRunfilesState::Cargo,
        }
    }

    /// Compatibility spelling for callers that describe the non-Bazel arm.
    pub fn no_bazel() -> Self {
        Self::cargo()
    }
}

impl RunfilesView for InMemoryRunfilesView {
    fn mode(&self) -> RunfilesMode {
        match self.state {
            InMemoryRunfilesState::Present { .. } | InMemoryRunfilesState::Missing => {
                RunfilesMode::Bazel
            }
            InMemoryRunfilesState::Cargo => RunfilesMode::Cargo,
        }
    }

    fn lookup(&self, relative: &Path) -> RunfilesLookup {
        match &self.state {
            InMemoryRunfilesState::Present { anchor, entries } => {
                if valid_relative(relative) && entries.contains(relative) {
                    RunfilesLookup::Present(RunfilesLocation {
                        anchor: anchor.clone(),
                        relative: relative.to_owned(),
                    })
                } else {
                    RunfilesLookup::Missing
                }
            }
            InMemoryRunfilesState::Missing => RunfilesLookup::Missing,
            InMemoryRunfilesState::Cargo => RunfilesLookup::NotBazel,
        }
    }
}

pub fn valid_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub fn require_bazel_location(
    view: &impl RunfilesView,
    relative: &Path,
) -> io::Result<RunfilesLocation> {
    match view.lookup(relative) {
        RunfilesLookup::Present(location) => Ok(location),
        RunfilesLookup::Missing => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "declared runfiles entry is missing",
        )),
        RunfilesLookup::NotBazel => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Bazel runfiles environment is unavailable",
        )),
    }
}
