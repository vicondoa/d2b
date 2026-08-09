use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const IMMUTABLE_SUPERVISOR_PATH: Option<&str> = option_env!("D2B_BAZEL_EXEC_SUPERVISOR");
const STARTUP_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
static PENDING_PROBE_CHILDREN: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
static PROBE_REAPER: OnceLock<()> = OnceLock::new();

/// Native systems admitted by the immutable execution toolchain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSystem {
    X86_64Linux,
    Aarch64Linux,
    Unsupported,
}

/// The minimum kernel version required by the ptrace startup probe.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KernelVersion {
    pub major: u32,
    pub minor: u32,
}

impl KernelVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }
}

/// Inputs checked before an execution helper may be started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupRequirements {
    pub system: NativeSystem,
    pub kernel: KernelVersion,
    pub yama_scope: Option<u8>,
    pub sandbox_policy_ok: bool,
}

/// The startup stage owns each refusal separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupCode {
    UnsupportedSystem,
    KernelTooOld,
    YamaRefused,
    ProbeFailed,
    SandboxPolicyDrift,
}

impl StartupCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedSystem => "D2B-BZLEXEC-NIX-PTRACE-SYSTEM",
            Self::KernelTooOld => "D2B-BZLEXEC-TOOLCHAIN-PTRACE-KERNEL",
            Self::YamaRefused => "D2B-BZLEXEC-TOOLCHAIN-PTRACE-YAMA",
            Self::ProbeFailed => "D2B-BZLEXEC-TOOLCHAIN-PTRACE-PROBE",
            Self::SandboxPolicyDrift => "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY",
        }
    }
}

#[derive(Debug)]
pub struct StartupError {
    code: StartupCode,
}

impl StartupError {
    pub const fn code(&self) -> StartupCode {
        self.code
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl std::error::Error for StartupError {}

/// The probe is kept behind a narrow boundary so the admission order can be
/// tested without making the production execution owner injectable.
pub trait StartupProbe {
    fn run(&self) -> io::Result<()>;
}

/// The runtime ptrace and sandbox-policy admission check.
///
/// Kernel and Yama values are supplied separately in [`StartupRequirements`].
/// The immutable supervisor runs a bounded parent-child ptrace round trip and
/// then requires the pinned sandbox filter to return its fixed denial for a
/// forbidden request. It deliberately returns no probe output or path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeStartupProbe;

fn immutable_supervisor_path() -> io::Result<PathBuf> {
    let value = IMMUTABLE_SUPERVISOR_PATH
        .ok_or_else(|| io::Error::other("immutable startup probe is unavailable"))?;
    let path = Path::new(value);
    if !path.is_absolute()
        || !value.starts_with("/nix/store/")
        || path
            .file_name()
            .is_none_or(|name| name != "d2b-bazel-exec-supervisor")
    {
        return Err(io::Error::other("immutable startup probe is unavailable"));
    }
    Ok(path.to_path_buf())
}

fn reap_probe_children() {
    let mut children = PENDING_PROBE_CHILDREN
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    children.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
}

fn retain_probe_child(child: Child) {
    {
        let mut children = PENDING_PROBE_CHILDREN
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        children.push(child);
    }
    PROBE_REAPER.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("d2b-bazel-startup-reaper".to_owned())
            .spawn(|| {
                loop {
                    reap_probe_children();
                    thread::sleep(Duration::from_millis(10));
                }
            });
    });
}

impl StartupProbe for RuntimeStartupProbe {
    fn run(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let mut child = Command::new(immutable_supervisor_path()?)
                .arg("--d2b-startup-probe")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| io::Error::other("startup probe spawn failed"))?;
            let deadline = Instant::now() + STARTUP_PROBE_TIMEOUT;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => return Ok(()),
                    Ok(Some(status)) if status.code() == Some(13) => {
                        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                    }
                    Ok(Some(_)) => {
                        return Err(io::Error::other("ptrace startup probe refused"));
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        retain_probe_child(child);
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    Err(_) => {
                        retain_probe_child(child);
                        return Err(io::Error::other("ptrace startup probe wait failed"));
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::other("ptrace startup probe is Linux-only"))
        }
    }
}

pub fn validate_startup(
    requirements: StartupRequirements,
    probe: &impl StartupProbe,
) -> Result<(), StartupError> {
    match requirements.system {
        NativeSystem::X86_64Linux | NativeSystem::Aarch64Linux => {}
        NativeSystem::Unsupported => {
            return Err(StartupError {
                code: StartupCode::UnsupportedSystem,
            });
        }
    }
    if requirements.kernel < KernelVersion::new(3, 19) {
        return Err(StartupError {
            code: StartupCode::KernelTooOld,
        });
    }
    if requirements.yama_scope.is_some_and(|scope| scope > 1) {
        return Err(StartupError {
            code: StartupCode::YamaRefused,
        });
    }
    probe.run().map_err(|error| StartupError {
        code: if error.kind() == io::ErrorKind::PermissionDenied {
            StartupCode::SandboxPolicyDrift
        } else {
            StartupCode::ProbeFailed
        },
    })?;
    if !requirements.sandbox_policy_ok {
        return Err(StartupError {
            code: StartupCode::SandboxPolicyDrift,
        });
    }
    Ok(())
}

/// A deterministic probe fake for unit and integration tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeResult {
    Pass,
    Fail,
}

impl StartupProbe for ProbeResult {
    fn run(&self) -> io::Result<()> {
        match self {
            Self::Pass => Ok(()),
            Self::Fail => Err(io::Error::other("probe refused")),
        }
    }
}
