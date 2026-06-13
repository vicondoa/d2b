//! The `spec` file: the SpecCodec-encoded command guestd hands to the runner.
//!
//! guestd encodes (writer); the runner decodes (reader). Length-prefixed
//! binary, no serde, validated on both sides via the crate's validation
//! primitives so a corrupt or oversized spec is rejected deterministically.

use crate::codec::{DecodeError, Reader, Writer};
use crate::{validate_argv, validate_cwd, validate_env, RunnerEnv, ValidationError};

/// Spec format magic ("NLES" = NixLing Exec Spec) + version.
const SPEC_MAGIC: u32 = 0x4e4c_4553;
const SPEC_VERSION: u32 = 1;

/// The validated, sanitized command the runner supervises.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecSpec {
    /// Absolute argv[0] plus arguments (argv[0] is an abs path; no PATH lookup).
    pub argv: Vec<String>,
    /// Validated absolute working directory (None => `/`).
    pub cwd: Option<String>,
    /// Sanitized environment (no inherited host env).
    pub env: Vec<RunnerEnv>,
    /// Per-stream retained-log byte cap.
    pub stdout_log_cap: u64,
    pub stderr_log_cap: u64,
    /// Optional runtime ceiling in seconds; 0 means unlimited (no timer).
    pub max_runtime_sec: u64,
}

// Never echo argv/env/cwd through Debug.
impl std::fmt::Debug for ExecSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecSpec")
            .field("argc", &self.argv.len())
            .field("has_cwd", &self.cwd.is_some())
            .field("env_count", &self.env.len())
            .field("stdout_log_cap", &self.stdout_log_cap)
            .field("stderr_log_cap", &self.stderr_log_cap)
            .field("max_runtime_sec", &self.max_runtime_sec)
            .finish()
    }
}

/// Typed spec failure. Carries no caller bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecError {
    Decode(DecodeError),
    Validation(ValidationError),
    BadMagic,
    BadVersion,
    LogCapZero,
}

impl From<DecodeError> for SpecError {
    fn from(error: DecodeError) -> Self {
        SpecError::Decode(error)
    }
}

impl From<ValidationError> for SpecError {
    fn from(error: ValidationError) -> Self {
        SpecError::Validation(error)
    }
}

impl ExecSpec {
    fn validate(&self) -> Result<(), SpecError> {
        validate_argv(&self.argv)?;
        validate_cwd(self.cwd.as_deref())?;
        validate_env(&self.env)?;
        if self.stdout_log_cap == 0 || self.stderr_log_cap == 0 {
            return Err(SpecError::LogCapZero);
        }
        Ok(())
    }
}

/// Stateless encoder/decoder for [`ExecSpec`].
pub struct SpecCodec;

impl SpecCodec {
    /// Encode a validated spec. Returns the validation error if the spec is
    /// malformed (guestd validates before writing, so this should not fail in
    /// practice, but the encoder is fail-closed).
    pub fn encode(spec: &ExecSpec) -> Result<Vec<u8>, SpecError> {
        spec.validate()?;
        let mut w = Writer::new();
        w.put_u32(SPEC_MAGIC);
        w.put_u32(SPEC_VERSION);
        w.put_u32(spec.argv.len() as u32);
        for arg in &spec.argv {
            w.put_str(arg);
        }
        match &spec.cwd {
            Some(cwd) => {
                w.put_bool(true);
                w.put_str(cwd);
            }
            None => w.put_bool(false),
        }
        w.put_u32(spec.env.len() as u32);
        for entry in &spec.env {
            w.put_str(&entry.key);
            w.put_str(&entry.value);
        }
        w.put_u64(spec.stdout_log_cap);
        w.put_u64(spec.stderr_log_cap);
        w.put_u64(spec.max_runtime_sec);
        Ok(w.into_vec())
    }

    /// Decode and re-validate a spec from on-disk bytes.
    pub fn decode(bytes: &[u8]) -> Result<ExecSpec, SpecError> {
        let mut r = Reader::new(bytes);
        if r.get_u32()? != SPEC_MAGIC {
            return Err(SpecError::BadMagic);
        }
        if r.get_u32()? != SPEC_VERSION {
            return Err(SpecError::BadVersion);
        }
        let argc = r.get_u32()? as usize;
        if argc > crate::MAX_ARGV {
            return Err(SpecError::Validation(ValidationError::TooManyArgs));
        }
        let mut argv = Vec::with_capacity(argc);
        for _ in 0..argc {
            argv.push(r.get_str()?);
        }
        let cwd = if r.get_bool()? {
            Some(r.get_str()?)
        } else {
            None
        };
        let env_count = r.get_u32()? as usize;
        if env_count > crate::MAX_ENV {
            return Err(SpecError::Validation(ValidationError::TooManyEnv));
        }
        let mut env = Vec::with_capacity(env_count);
        for _ in 0..env_count {
            let key = r.get_str()?;
            let value = r.get_str()?;
            env.push(RunnerEnv { key, value });
        }
        let stdout_log_cap = r.get_u64()?;
        let stderr_log_cap = r.get_u64()?;
        let max_runtime_sec = r.get_u64()?;
        r.finish()?;

        let spec = ExecSpec {
            argv,
            cwd,
            env,
            stdout_log_cap,
            stderr_log_cap,
            max_runtime_sec,
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ExecSpec {
        ExecSpec {
            argv: vec!["/bin/sleep".to_owned(), "infinity".to_owned()],
            cwd: Some("/var/empty".to_owned()),
            env: vec![RunnerEnv {
                key: "PATH".to_owned(),
                value: "/run/current-system/sw/bin".to_owned(),
            }],
            stdout_log_cap: 4 * 1024 * 1024,
            stderr_log_cap: 4 * 1024 * 1024,
            max_runtime_sec: 0,
        }
    }

    #[test]
    fn round_trips() {
        let spec = sample();
        let bytes = SpecCodec::encode(&spec).unwrap();
        let decoded = SpecCodec::decode(&bytes).unwrap();
        assert!(spec == decoded);
    }

    #[test]
    fn round_trips_without_cwd_and_with_ceiling() {
        let mut spec = sample();
        spec.cwd = None;
        spec.max_runtime_sec = 600;
        let bytes = SpecCodec::encode(&spec).unwrap();
        let decoded = SpecCodec::decode(&bytes).unwrap();
        assert!(spec == decoded);
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut bytes = SpecCodec::encode(&sample()).unwrap();
        bytes[0] ^= 0xff;
        assert_eq!(SpecCodec::decode(&bytes), Err(SpecError::BadMagic));

        let mut bytes = SpecCodec::encode(&sample()).unwrap();
        // Flip the version word (bytes 4..8).
        bytes[4] ^= 0xff;
        assert_eq!(SpecCodec::decode(&bytes), Err(SpecError::BadVersion));
    }

    #[test]
    fn rejects_zero_log_cap_and_invalid_command() {
        let mut spec = sample();
        spec.stdout_log_cap = 0;
        assert_eq!(SpecCodec::encode(&spec), Err(SpecError::LogCapZero));

        let mut spec = sample();
        spec.argv = vec!["relative".to_owned()];
        // argv validation here only checks shape (nul/len); abs-path is a
        // guestd policy gate, so a relative argv[0] still encodes but the
        // runner refuses to spawn it (see the bin's spawn path).
        assert!(SpecCodec::encode(&spec).is_ok());

        let mut spec = sample();
        spec.argv = vec!["bad\0arg".to_owned()];
        assert_eq!(
            SpecCodec::encode(&spec),
            Err(SpecError::Validation(ValidationError::ArgContainsNul))
        );
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let mut bytes = SpecCodec::encode(&sample()).unwrap();
        bytes.push(0);
        assert_eq!(
            SpecCodec::decode(&bytes),
            Err(SpecError::Decode(DecodeError::TrailingBytes))
        );
    }

    #[test]
    fn debug_never_echoes_payload() {
        let spec = sample();
        let rendered = format!("{spec:?}");
        assert!(!rendered.contains("/bin/sleep"));
        assert!(!rendered.contains("PATH"));
        assert!(!rendered.contains("/run/current-system"));
    }
}
