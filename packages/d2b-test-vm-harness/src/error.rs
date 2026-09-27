//! Failure vocabulary for the lane's guest launcher.
//!
//! Every failure a caller can act on names the thing that was missing or the
//! thing that broke: a host that cannot run the lane, a guest whose attached
//! devices cannot be snapshotted, a guest that never activated. A launcher
//! that reports "error" leaves a contributor guessing which of the three it
//! was.

use std::{fmt, io, path::PathBuf, time::Duration};

use crate::host::Capability;

/// A writable block device the guest attached that cannot carry an internal
/// snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsnapshottableDevice {
    /// The device's id as QEMU names it on the monitor.
    pub device: String,
    /// The backing file QEMU resolved for it.
    pub file: String,
    /// The image format that device reports.
    pub format: String,
}

impl fmt::Display for UnsnapshottableDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} (file {}, format {})",
            self.device, self.file, self.format
        )
    }
}

/// Everything that can stop the lane before, during, or after a boot.
#[derive(Debug)]
pub enum HarnessError {
    /// The guest image carries no readable manifest, or one the launcher
    /// cannot use.
    Manifest { path: PathBuf, detail: String },
    /// The host cannot run the lane. Every missing capability is named at
    /// once so a contributor fixes them in one pass rather than one boot at
    /// a time.
    HostUnsupported { missing: Vec<Capability> },
    /// A guest attached a writable device with no internal-snapshot support.
    /// The lane refuses to run without restore rather than silently
    /// degrading.
    NotSnapshottable { devices: Vec<UnsnapshottableDevice> },
    /// The guest never reached its activation contract inside the bound the
    /// caller gave it. The console tail travels with the failure, because a
    /// bounded wait with no diagnostics is the hang the bound replaced.
    NotActivated {
        bound: Duration,
        marker: String,
        console_tail: String,
    },
    /// The guest reported activation for a different guest shape than the one
    /// the launcher was asked to boot.
    WrongShape {
        expected: String,
        reported: String,
    },
    /// The monitor refused a command.
    Monitor { command: String, detail: String },
    /// Filesystem or process work that failed underneath the launcher.
    Io { action: String, source: io::Error },
    /// The emulator could not be started at all.
    Spawn { detail: String },
    /// The launcher's own configuration is unusable.
    Configuration(String),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest { path, detail } => {
                write!(formatter, "guest manifest {} is unusable: {detail}", path.display())
            }
            Self::HostUnsupported { missing } => {
                write!(formatter, "this host cannot run the host-integration lane:")?;
                for capability in missing {
                    write!(formatter, "\n  - {capability}")?;
                }
                write!(
                    formatter,
                    "\nthe lane requires hardware virtualization; it does not fall back to emulation"
                )
            }
            Self::NotSnapshottable { devices } => {
                write!(
                    formatter,
                    "the guest attached writable devices that do not support internal snapshots, so it cannot be restored between checks:"
                )?;
                for device in devices {
                    write!(formatter, "\n  - {device}")?;
                }
                write!(
                    formatter,
                    "\nmaterialize each of them in qcow2, or attach them with an ephemeral overlay, and the lane will run it"
                )
            }
            Self::NotActivated {
                bound,
                marker,
                console_tail,
            } => {
                write!(
                    formatter,
                    "the guest did not reach activation within {}s: no {marker} marker on its console",
                    bound.as_secs()
                )?;
                if !console_tail.is_empty() {
                    write!(formatter, "\n--- guest console tail ---\n{console_tail}")?;
                }
                Ok(())
            }
            Self::WrongShape {
                expected,
                reported,
            } => {
                write!(
                    formatter,
                    "the guest reported activation for the {reported} shape, but the lane asked it to boot the {expected} shape"
                )
            }
            Self::Monitor { command, detail } => {
                write!(formatter, "the emulator monitor refused {command}: {detail}")
            }
            Self::Io { action, source } => write!(formatter, "{action}: {source}"),
            Self::Spawn { detail } => write!(formatter, "the emulator could not be started: {detail}"),
            Self::Configuration(detail) => write!(formatter, "lane configuration is unusable: {detail}"),
        }
    }
}

impl std::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl HarnessError {
    /// Wrap filesystem or process work in the action that attempted it, so
    /// the message says what the launcher was doing rather than only what the
    /// operating system said.
    pub fn io(action: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            action: action.into(),
            source,
        }
    }
}

/// The launcher's result type.
pub type Result<T, E = HarnessError> = std::result::Result<T, E>;
