//! The d2b host-integration lane's guest launcher.
//!
//! The lane is Bazel-owned end to end: a build action produces each check's
//! guest image as a cacheable graph output, and this crate boots it, waits
//! for it to activate, and takes it down. What it deliberately does *not* do
//! is decide the invocation - memory, vCPU count, the drive layout, the
//! per-check device options - for a guest. Those are read off the re-homed
//! guest node's evaluated configuration and reach the launcher through the
//! image's manifest, so a check boots the guest it declared rather than a
//! uniform shape the lane made up.
//!
//! The crate holds no test aggregate: the repository's test census
//! force-registers any crate that has one into the main package suite, and
//! this crate is not a main-package test. The lane's own suite in
//! `bazel/checks/vm` names its clippy targets and its guest-boot targets
//! directly, so it is still linted and still run by the repository's gates.

pub mod checks;
pub mod error;
pub mod guest;
pub mod host;
pub mod legacy;
pub mod manifest;
pub mod monitor;

pub use error::{HarnessError, Result, UnsnapshottableDevice};
pub use guest::{ActiveGuest, GuestSpec, RestoreTarget, SnapshotPoint, boot, report, reserve_loopback_port};
pub use legacy::{LegacyCheck, LegacyError, LegacyGuest, LegacyOutcome};
pub use host::{Capability, HostFacts, require_this_host};
pub use manifest::{Assertions, CheckRecord, EphemeralDrive, Footprint, GuestManifest, PoolBudget};
pub use monitor::{BlockDevice, Cache, Monitor};
