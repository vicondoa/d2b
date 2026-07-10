use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const MAX_CONFIGURED_ARGC: usize = 128;
pub const MAX_CONFIGURED_ARG_BYTES: usize = 16 * 1024;
pub const MAX_CONFIGURED_ARG_LEN: usize = 4096;

/// Serialized configured argv whose debug representation is always redacted.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct ConfiguredArgv(Vec<String>);

impl ConfiguredArgv {
    pub fn new(argv: Vec<String>) -> Result<Self, String> {
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

fn validate_argv(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("configured argv must not be empty".to_owned());
    }
    if argv.len() > MAX_CONFIGURED_ARGC {
        return Err(format!(
            "configured argv exceeds {MAX_CONFIGURED_ARGC} arguments"
        ));
    }
    let mut bytes = 0usize;
    for arg in argv {
        if arg.contains('\0') {
            return Err("configured argv must not contain NUL".to_owned());
        }
        if arg.len() > MAX_CONFIGURED_ARG_LEN {
            return Err(format!(
                "configured argv argument exceeds {MAX_CONFIGURED_ARG_LEN} bytes"
            ));
        }
        bytes = bytes
            .checked_add(arg.len())
            .ok_or_else(|| "configured argv byte count overflow".to_owned())?;
    }
    if bytes > MAX_CONFIGURED_ARG_BYTES {
        return Err(format!(
            "configured argv exceeds {MAX_CONFIGURED_ARG_BYTES} bytes"
        ));
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
