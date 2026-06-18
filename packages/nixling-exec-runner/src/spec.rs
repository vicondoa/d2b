//! The `spec` file: the SpecCodec-encoded command guestd hands to the runner.
//!
//! guestd encodes (writer); the runner decodes (reader). Length-prefixed
//! binary, no serde, validated on both sides via the crate's validation
//! primitives so a corrupt or oversized spec is rejected deterministically.

use crate::codec::{DecodeError, Reader, Writer};
use crate::{
    MAX_ARG_LEN, MAX_ARGV, MAX_ENV, RunnerEnv, ValidationError, contains_nul, validate_argv,
    validate_cwd, validate_env,
};

/// Spec format magic ("NLES" = NixLing Exec Spec) + version.
const SPEC_MAGIC: u32 = 0x4e4c_4553;
const SPEC_VERSION: u32 = 2;
const MAX_SYSTEMD_RUN_ARGS: usize = (MAX_ENV * 2) + MAX_ARGV + 32;

/// The validated, sanitized command the runner supervises.
#[derive(Clone, PartialEq, Eq)]
pub struct ExecSpec {
    /// Guest command argv. argv[0] may be absolute, bare, or relative; the
    /// workload user's login shell performs PATH lookup for bare commands.
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
    /// Workload user name guestd resolved as non-root.
    pub exec_user: String,
    /// UID guestd resolved for `exec_user`; the runner re-resolves and compares
    /// immediately before spawn.
    pub exec_uid: u32,
    /// Absolute path to `systemd-run`.
    pub systemd_run_path: String,
    /// Absolute path to the workload user's login shell wrapper.
    pub login_shell_path: String,
    /// Deterministic slot-derived workload unit name.
    pub workload_unit_name: String,
    /// `systemd-run` argv (excluding the binary path) built by guestd.
    pub systemd_run_args: Vec<String>,
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
            .field("has_exec_user", &(!self.exec_user.is_empty()))
            .field("has_systemd_run_path", &(!self.systemd_run_path.is_empty()))
            .field("has_login_shell_path", &(!self.login_shell_path.is_empty()))
            .field("systemd_run_argc", &self.systemd_run_args.len())
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
    BadExecUser,
    BadExecUid,
    BadPath,
    BadWorkloadUnitName,
    BadSystemdRunArgs,
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
        validate_exec_user(&self.exec_user)?;
        if self.exec_uid == 0 {
            return Err(SpecError::BadExecUid);
        }
        validate_abs_path(&self.systemd_run_path)?;
        validate_abs_path(&self.login_shell_path)?;
        validate_workload_unit_name(&self.workload_unit_name)?;
        validate_systemd_run_args(&self.systemd_run_args)?;
        Ok(())
    }
}

fn validate_exec_user(user: &str) -> Result<(), SpecError> {
    if user.is_empty() || user.len() > MAX_ARG_LEN || contains_nul(user) {
        return Err(SpecError::BadExecUser);
    }
    Ok(())
}

fn validate_abs_path(path: &str) -> Result<(), SpecError> {
    if path.is_empty() || path.len() > MAX_ARG_LEN || contains_nul(path) || !path.starts_with('/') {
        return Err(SpecError::BadPath);
    }
    Ok(())
}

fn validate_workload_unit_name(unit: &str) -> Result<(), SpecError> {
    if unit.is_empty()
        || unit.len() > MAX_ARG_LEN
        || contains_nul(unit)
        || unit.contains('/')
        || !unit.starts_with("nixling-exec-")
        || !unit.ends_with("-w.service")
        || !unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
    {
        return Err(SpecError::BadWorkloadUnitName);
    }
    Ok(())
}

fn validate_systemd_run_args(args: &[String]) -> Result<(), SpecError> {
    if args.is_empty() || args.len() > MAX_SYSTEMD_RUN_ARGS {
        return Err(SpecError::BadSystemdRunArgs);
    }
    for arg in args {
        if arg.is_empty() || arg.len() > MAX_ARG_LEN || contains_nul(arg) {
            return Err(SpecError::BadSystemdRunArgs);
        }
    }
    Ok(())
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
        w.put_str(&spec.exec_user);
        w.put_u32(spec.exec_uid);
        w.put_str(&spec.systemd_run_path);
        w.put_str(&spec.login_shell_path);
        w.put_str(&spec.workload_unit_name);
        w.put_u32(spec.systemd_run_args.len() as u32);
        for arg in &spec.systemd_run_args {
            w.put_str(arg);
        }
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
        let exec_user = r.get_str()?;
        let exec_uid = r.get_u32()?;
        let systemd_run_path = r.get_str()?;
        let login_shell_path = r.get_str()?;
        let workload_unit_name = r.get_str()?;
        let systemd_argc = r.get_u32()? as usize;
        if systemd_argc > MAX_SYSTEMD_RUN_ARGS {
            return Err(SpecError::BadSystemdRunArgs);
        }
        let mut systemd_run_args = Vec::with_capacity(systemd_argc);
        for _ in 0..systemd_argc {
            systemd_run_args.push(r.get_str()?);
        }
        r.finish()?;

        let spec = ExecSpec {
            argv,
            cwd,
            env,
            stdout_log_cap,
            stderr_log_cap,
            max_runtime_sec,
            exec_user,
            exec_uid,
            systemd_run_path,
            login_shell_path,
            workload_unit_name,
            systemd_run_args,
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
            exec_user: "alice".to_owned(),
            exec_uid: 1000,
            systemd_run_path: "/run/current-system/sw/bin/systemd-run".to_owned(),
            login_shell_path: "/run/current-system/sw/bin/bash".to_owned(),
            workload_unit_name: "nixling-exec-07-w.service".to_owned(),
            systemd_run_args: vec![
                "--uid=alice".to_owned(),
                "--unit=nixling-exec-07-w.service".to_owned(),
                "--property=PAMName=login".to_owned(),
                "--".to_owned(),
                "/run/current-system/sw/bin/bash".to_owned(),
                "-l".to_owned(),
                "-c".to_owned(),
                r#"exec "$@""#.to_owned(),
                "nl-exec".to_owned(),
                "/bin/sleep".to_owned(),
                "infinity".to_owned(),
            ],
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
        // argv validation here only checks shape (nul/len/leading dash);
        // abs-path is deliberately NOT required because detached commands run
        // through the workload user's login shell and may be bare/relative.
        assert!(SpecCodec::encode(&spec).is_ok());

        let mut spec = sample();
        spec.argv = vec!["-bad".to_owned()];
        assert_eq!(
            SpecCodec::encode(&spec),
            Err(SpecError::Validation(ValidationError::ArgLeadingDash))
        );

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
        assert!(!rendered.contains("alice"));
        assert!(!rendered.contains("nl-exec"));
    }
}
