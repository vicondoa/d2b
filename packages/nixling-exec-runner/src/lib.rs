#![doc = "Guest-side detached exec runner validation primitives."]

pub const MAX_ARGV: usize = 128;
pub const MAX_ARG_LEN: usize = 4096;
pub const MAX_CWD_LEN: usize = 4096;
pub const MAX_ENV: usize = 256;
pub const MAX_ENV_KEY_LEN: usize = 128;
pub const MAX_ENV_VALUE_LEN: usize = 8192;

#[derive(Clone, PartialEq, Eq)]
pub struct RunnerCommand {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<RunnerEnv>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RunnerEnv {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    EmptyArgv,
    TooManyArgs,
    ArgEmpty,
    ArgTooLong,
    ArgContainsNul,
    CwdEmpty,
    CwdRelative,
    CwdTooLong,
    CwdContainsNul,
    TooManyEnv,
    EnvKeyEmpty,
    EnvKeyTooLong,
    EnvKeyContainsNul,
    EnvKeyContainsEquals,
    EnvValueTooLong,
    EnvValueContainsNul,
}

impl RunnerCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_argv(&self.argv)?;
        validate_cwd(self.cwd.as_deref())?;
        validate_env(&self.env)
    }
}

pub fn validate_argv(argv: &[String]) -> Result<(), ValidationError> {
    if argv.is_empty() {
        return Err(ValidationError::EmptyArgv);
    }
    if argv.len() > MAX_ARGV {
        return Err(ValidationError::TooManyArgs);
    }
    for arg in argv {
        if arg.is_empty() {
            return Err(ValidationError::ArgEmpty);
        }
        if arg.len() > MAX_ARG_LEN {
            return Err(ValidationError::ArgTooLong);
        }
        if contains_nul(arg) {
            return Err(ValidationError::ArgContainsNul);
        }
    }
    Ok(())
}

pub fn validate_cwd(cwd: Option<&str>) -> Result<(), ValidationError> {
    let Some(cwd) = cwd else {
        return Ok(());
    };
    if cwd.is_empty() {
        return Err(ValidationError::CwdEmpty);
    }
    if contains_nul(cwd) {
        return Err(ValidationError::CwdContainsNul);
    }
    if !cwd.starts_with('/') {
        return Err(ValidationError::CwdRelative);
    }
    if cwd.len() > MAX_CWD_LEN {
        return Err(ValidationError::CwdTooLong);
    }
    Ok(())
}

pub fn validate_env(env: &[RunnerEnv]) -> Result<(), ValidationError> {
    if env.len() > MAX_ENV {
        return Err(ValidationError::TooManyEnv);
    }
    for entry in env {
        if entry.key.is_empty() {
            return Err(ValidationError::EnvKeyEmpty);
        }
        if entry.key.len() > MAX_ENV_KEY_LEN {
            return Err(ValidationError::EnvKeyTooLong);
        }
        if contains_nul(&entry.key) {
            return Err(ValidationError::EnvKeyContainsNul);
        }
        if entry.key.contains('=') {
            return Err(ValidationError::EnvKeyContainsEquals);
        }
        if entry.value.len() > MAX_ENV_VALUE_LEN {
            return Err(ValidationError::EnvValueTooLong);
        }
        if contains_nul(&entry.value) {
            return Err(ValidationError::EnvValueContainsNul);
        }
    }
    Ok(())
}

fn contains_nul(value: &str) -> bool {
    value.as_bytes().contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_command() -> RunnerCommand {
        RunnerCommand {
            argv: vec!["/bin/true".to_owned()],
            cwd: Some("/home/alice".to_owned()),
            env: vec![RunnerEnv {
                key: "PATH".to_owned(),
                value: "/run/current-system/sw/bin".to_owned(),
            }],
        }
    }

    #[test]
    fn accepts_bounded_command() {
        assert!(valid_command().validate().is_ok());
        assert!(validate_argv(&["x".repeat(MAX_ARG_LEN)]).is_ok());
        assert!(validate_argv(&vec!["x".to_owned(); MAX_ARGV]).is_ok());
        assert!(validate_cwd(Some(&format!("/{}", "x".repeat(MAX_CWD_LEN - 1)))).is_ok());
        assert!(validate_env(&[RunnerEnv {
            key: "K".repeat(MAX_ENV_KEY_LEN),
            value: "V".repeat(MAX_ENV_VALUE_LEN),
        }])
        .is_ok());
        assert!(validate_env(&vec![
            RunnerEnv {
                key: "K".to_owned(),
                value: "V".to_owned(),
            };
            MAX_ENV
        ])
        .is_ok());
    }

    #[test]
    fn rejects_empty_or_too_many_args() {
        assert_eq!(
            RunnerCommand {
                argv: Vec::new(),
                cwd: None,
                env: Vec::new(),
            }
            .validate(),
            Err(ValidationError::EmptyArgv)
        );
        assert_eq!(
            validate_argv(&vec!["x".to_owned(); MAX_ARGV + 1]),
            Err(ValidationError::TooManyArgs)
        );
    }

    #[test]
    fn rejects_unbounded_or_nul_values_without_echoing_payloads() {
        assert_eq!(
            validate_argv(&["x".repeat(MAX_ARG_LEN + 1)]),
            Err(ValidationError::ArgTooLong)
        );
        assert_eq!(
            validate_argv(&["".to_owned()]),
            Err(ValidationError::ArgEmpty)
        );
        assert_eq!(
            validate_argv(&["bad\0arg".to_owned()]),
            Err(ValidationError::ArgContainsNul)
        );
        assert_eq!(
            validate_cwd(Some("a\0b")),
            Err(ValidationError::CwdContainsNul)
        );
        assert_eq!(validate_cwd(Some("")), Err(ValidationError::CwdEmpty));
        assert_eq!(
            validate_cwd(Some("relative")),
            Err(ValidationError::CwdRelative)
        );
        assert_eq!(
            validate_cwd(Some(&format!("/{}", "x".repeat(MAX_CWD_LEN)))),
            Err(ValidationError::CwdTooLong)
        );
        assert_eq!(
            validate_env(&vec![
                RunnerEnv {
                    key: "K".to_owned(),
                    value: "V".to_owned(),
                };
                MAX_ENV + 1
            ]),
            Err(ValidationError::TooManyEnv)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "".to_owned(),
                value: "value".to_owned(),
            }]),
            Err(ValidationError::EnvKeyEmpty)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "K".repeat(MAX_ENV_KEY_LEN + 1),
                value: "value".to_owned(),
            }]),
            Err(ValidationError::EnvKeyTooLong)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "BAD\0KEY".to_owned(),
                value: "value".to_owned(),
            }]),
            Err(ValidationError::EnvKeyContainsNul)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "BAD=KEY".to_owned(),
                value: "value".to_owned(),
            }]),
            Err(ValidationError::EnvKeyContainsEquals)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "TOKEN".to_owned(),
                value: "x".repeat(MAX_ENV_VALUE_LEN + 1),
            }]),
            Err(ValidationError::EnvValueTooLong)
        );
        assert_eq!(
            validate_env(&[RunnerEnv {
                key: "TOKEN".to_owned(),
                value: "bad\0value".to_owned(),
            }]),
            Err(ValidationError::EnvValueContainsNul)
        );
    }
}
