use d2b_bazel_support::runfiles::{RunfilesMode, RunfilesView};

/// The runfiles mode captured for one locator operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeSelection {
    mode: RunfilesMode,
}

impl ModeSelection {
    /// Read the runfiles mode once and retain that decision.
    pub fn select<R>(runfiles: &R) -> Self
    where
        R: RunfilesView + ?Sized,
    {
        Self {
            mode: runfiles.mode(),
        }
    }

    /// Return the mode captured by [`Self::select`].
    pub const fn mode(self) -> RunfilesMode {
        self.mode
    }

    /// Run exactly the arm selected by [`Self::select`].
    ///
    /// The selected arm's result is returned directly. In particular, an
    /// error from the Bazel arm is not converted into an attempt to run Cargo.
    pub fn resolve<T, E, Bazel, Cargo>(self, bazel: Bazel, cargo: Cargo) -> Result<T, E>
    where
        Bazel: FnOnce() -> Result<T, E>,
        Cargo: FnOnce() -> Result<T, E>,
    {
        match self.mode {
            RunfilesMode::Bazel => bazel(),
            RunfilesMode::Cargo => cargo(),
        }
    }
}
