use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const MAX_CONFIGURED_ARGC: usize = 128;
pub const MAX_CONFIGURED_ARG_BYTES: usize = 16 * 1024;
pub const MAX_CONFIGURED_ARG_LEN: usize = 4096;

/// Failure classes for [`ConfiguredArgv`] construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfiguredArgvError {
    Empty,
    TooManyArgs { max: usize },
    NulByte,
    ArgTooLong { max: usize },
    ByteCountOverflow,
    TooManyBytes { max: usize },
}

impl core::fmt::Display for ConfiguredArgvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConfiguredArgvError::Empty => f.write_str("configured argv must not be empty"),
            ConfiguredArgvError::TooManyArgs { max } => {
                write!(f, "configured argv exceeds {max} arguments")
            }
            ConfiguredArgvError::NulByte => {
                f.write_str("configured argv must not contain NUL")
            }
            ConfiguredArgvError::ArgTooLong { max } => {
                write!(f, "configured argv argument exceeds {max} bytes")
            }
            ConfiguredArgvError::ByteCountOverflow => {
                f.write_str("configured argv byte count overflow")
            }
            ConfiguredArgvError::TooManyBytes { max } => {
                write!(f, "configured argv exceeds {max} bytes")
            }
        }
    }
}

impl std::error::Error for ConfiguredArgvError {}

/// Serialized configured argv whose debug representation is always redacted.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ConfiguredArgv(Vec<String>);

impl ConfiguredArgv {
    pub fn new(argv: Vec<String>) -> Result<Self, ConfiguredArgvError> {
        validate_argv(&argv)?;
        Ok(Self(argv))
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn into_inner(self) -> Vec<String> {
        self.0
    }
}

impl fmt::Debug for ConfiguredArgv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguredArgv")
            .field("argc", &self.0.len())
            .field("argv", &"<redacted>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for ConfiguredArgv {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let argv = Vec::<String>::deserialize(deserializer)?;
        Self::new(argv).map_err(serde::de::Error::custom)
    }
}

fn validate_argv(argv: &[String]) -> Result<(), ConfiguredArgvError> {
    if argv.is_empty() {
        return Err(ConfiguredArgvError::Empty);
    }
    if argv.len() > MAX_CONFIGURED_ARGC {
        return Err(ConfiguredArgvError::TooManyArgs {
            max: MAX_CONFIGURED_ARGC,
        });
    }
    let mut bytes = 0usize;
    for arg in argv {
        if arg.contains('\0') {
            return Err(ConfiguredArgvError::NulByte);
        }
        if arg.len() > MAX_CONFIGURED_ARG_LEN {
            return Err(ConfiguredArgvError::ArgTooLong {
                max: MAX_CONFIGURED_ARG_LEN,
            });
        }
        bytes = bytes
            .checked_add(arg.len())
            .ok_or(ConfiguredArgvError::ByteCountOverflow)?;
    }
    if bytes > MAX_CONFIGURED_ARG_BYTES {
        return Err(ConfiguredArgvError::TooManyBytes {
            max: MAX_CONFIGURED_ARG_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_argv_is_bounded_and_debug_redacted() {
        let canary = "private-canary-argv";
        let argv = ConfiguredArgv::new(vec!["firefox".to_owned(), canary.to_owned()]).unwrap();
        let debug = format!("{argv:?}");
        assert!(!debug.contains(canary));
        assert!(!debug.contains("firefox"));
        assert!(debug.contains("argc"));
        assert!(ConfiguredArgv::new(Vec::new()).is_err());
        assert!(ConfiguredArgv::new(vec!["x\0y".to_owned()]).is_err());
        assert!(ConfiguredArgv::new(vec!["x".repeat(MAX_CONFIGURED_ARG_LEN + 1)]).is_err());
    }
}
