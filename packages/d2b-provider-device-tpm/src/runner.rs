//! Opaque swtpm runner declarations.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::state::{StateDirIntent, StateDirectoryToken};
use crate::{MAX_SWTPM_LOG_LEVEL, MIN_SWTPM_LOG_LEVEL};

/// Signed component binary selected by the Provider descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BinaryKind {
    /// Long-lived swtpm socket binary.
    Swtpm,
    /// One-shot swtpm control binary.
    SwtpmIoctl,
}

/// Opaque signed binary reference. It cannot carry a store path.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedBinaryRef {
    kind: BinaryKind,
    descriptor_token: [u8; 32],
}

impl SignedBinaryRef {
    /// Construct a signed reference at the Core adapter boundary.
    pub const fn from_core(kind: BinaryKind, descriptor_token: [u8; 32]) -> Self {
        Self {
            kind,
            descriptor_token,
        }
    }

    /// Return the selected binary kind.
    pub const fn kind(&self) -> BinaryKind {
        self.kind
    }
}

impl fmt::Debug for SignedBinaryRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignedBinaryRef(<redacted>)")
    }
}

/// Device-tpm desired settings. There is no path, artifact, or flush-toggle
/// field. The pre-start flush is mandatory for every activation cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwtpmSettings {
    /// swtpm `--log level` in the closed 1..=20 range.
    #[serde(default = "default_log_level")]
    pub log_level: u8,
}

impl Default for SwtpmSettings {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
        }
    }
}

impl SwtpmSettings {
    /// Validate settings received from the signed Provider schema.
    pub const fn validate(self) -> Result<Self, SwtpmArgvError> {
        if self.log_level < MIN_SWTPM_LOG_LEVEL || self.log_level > MAX_SWTPM_LOG_LEVEL {
            Err(SwtpmArgvError::LogLevelOutOfRange)
        } else {
            Ok(self)
        }
    }
}

fn default_log_level() -> u8 {
    20
}

/// Opaque ticket for the pre-start flush.
#[derive(Clone, PartialEq, Eq)]
pub struct FlushLaunchTicket([u8; 16]);

impl FlushLaunchTicket {
    /// Construct a ticket at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for FlushLaunchTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FlushLaunchTicket(<redacted>)")
    }
}

/// Opaque ticket for the long-lived swtpm process.
#[derive(Clone, PartialEq, Eq)]
pub struct SwtpmStartLaunchTicket([u8; 16]);

impl SwtpmStartLaunchTicket {
    /// Construct a ticket at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SwtpmStartLaunchTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SwtpmStartLaunchTicket(<redacted>)")
    }
}

/// A deterministic semantic argv snapshot. The placeholders are resolved by
/// Core from the opaque tickets, never by Provider code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwtpmArgv {
    args: Vec<&'static str>,
}

impl SwtpmArgv {
    /// Build the fixed argv shape used by the signed swtpm component.
    pub fn for_settings(settings: SwtpmSettings) -> Result<Self, SwtpmArgvError> {
        settings.validate()?;
        let mut args = vec![
            "swtpm",
            "socket",
            "--tpm2",
            "--tpmstate",
            "<state-dir>",
            "--ctrl",
            "<ctrl-socket>",
            "--server",
            "<server-socket>",
        ];
        args.extend(["--flags", "startup-clear"]);
        args.extend([
            "--log",
            "file=<state-dir>/swtpm.log,level=<log-level>",
            "--pid",
            "<state-dir>/swtpm.pid",
        ]);
        args.push("--daemon=false");
        Ok(Self { args })
    }

    /// Return the placeholder argv snapshot for golden tests.
    pub fn args(&self) -> &[&'static str] {
        &self.args
    }

    /// Return the pre-start flush semantic argv snapshot.
    pub const fn flush_args() -> [&'static str; 4] {
        ["swtpm_ioctl", "-i", "--unix", "<ctrl-socket>"]
    }
}

/// Closed argv-generation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwtpmArgvError {
    /// The log level is outside the signed Provider schema range.
    LogLevelOutOfRange,
    /// A ticket was not issued for the expected binary.
    WrongBinaryKind,
    /// The state intent and ticket were not paired.
    TicketIntentMismatch,
}

impl fmt::Display for SwtpmArgvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LogLevelOutOfRange => "swtpm-log-level-out-of-range",
            Self::WrongBinaryKind => "swtpm-binary-kind-mismatch",
            Self::TicketIntentMismatch => "swtpm-ticket-intent-mismatch",
        })
    }
}

impl std::error::Error for SwtpmArgvError {}

/// Validate that a Core-issued long-lived launch ticket is bound to the
/// requested opaque state intent.
pub fn validate_start_ticket(
    intent: &StateDirIntent,
    state_token: &StateDirectoryToken,
    binary: &SignedBinaryRef,
) -> Result<(), SwtpmArgvError> {
    if binary.kind() != BinaryKind::Swtpm {
        return Err(SwtpmArgvError::WrongBinaryKind);
    }
    if intent.directory().as_bytes() != state_token.as_bytes() {
        return Err(SwtpmArgvError::TicketIntentMismatch);
    }
    Ok(())
}
