//! Signed swtpm settings.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::{MAX_SWTPM_LOG_LEVEL, MIN_SWTPM_LOG_LEVEL};

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

/// Closed argv-generation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwtpmArgvError {
    /// The log level is outside the signed Provider schema range.
    LogLevelOutOfRange,
}

impl fmt::Display for SwtpmArgvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LogLevelOutOfRange => "swtpm-log-level-out-of-range",
        })
    }
}

impl std::error::Error for SwtpmArgvError {}
