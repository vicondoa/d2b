#![forbid(unsafe_code)]

use d2b_bazel_support::runfiles::RunfilesMode;
use serde::{Deserialize, Serialize};

mod exec_handle;

pub use exec_handle::execute;

/// The neutral runner root. Later runner modules attach to this stable seam.
pub const RUNNER_CRATE_ROLE: &str = "bazel-rust-runner";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunnerMode {
    Cargo,
    Bazel,
}

impl From<RunfilesMode> for RunnerMode {
    fn from(mode: RunfilesMode) -> Self {
        match mode {
            RunfilesMode::Bazel => Self::Bazel,
            RunfilesMode::Cargo => Self::Cargo,
        }
    }
}

pub const fn mode_name(mode: RunnerMode) -> &'static str {
    match mode {
        RunnerMode::Cargo => "cargo",
        RunnerMode::Bazel => "bazel",
    }
}
