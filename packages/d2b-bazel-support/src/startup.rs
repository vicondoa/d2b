use std::{fmt, io};

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

/// The process-local part of the ptrace admission check.
///
/// Kernel and Yama values are supplied separately in [`StartupRequirements`].
/// This probe verifies that the running Linux process exposes the proc status
/// needed by the ptrace handoff and is not already being traced by another
/// process.  It deliberately emits no proc path or kernel text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeStartupProbe;

impl StartupProbe for RuntimeStartupProbe {
    fn run(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let status = std::fs::read_to_string("/proc/self/status")?;
            let tracer = status
                .lines()
                .find_map(|line| line.strip_prefix("TracerPid:"))
                .ok_or_else(|| io::Error::other("ptrace status is unavailable"))?;
            if tracer.trim() != "0" {
                return Err(io::Error::other("process is already traced"));
            }
            Ok(())
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
    probe.run().map_err(|_| StartupError {
        code: StartupCode::ProbeFailed,
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
