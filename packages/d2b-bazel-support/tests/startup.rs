use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use d2b_bazel_support::startup::{
    KernelVersion, NativeSystem, ProbeResult, RuntimeStartupProbe, StartupCode, StartupProbe,
    StartupRequirements, validate_startup,
};

#[derive(Clone)]
struct RecordingProbe {
    calls: Arc<Mutex<usize>>,
    result: ProbeResult,
}

impl RecordingProbe {
    fn new(result: ProbeResult) -> Self {
        Self {
            calls: Arc::new(Mutex::new(0)),
            result,
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("probe counter")
    }
}

impl StartupProbe for RecordingProbe {
    fn run(&self) -> std::io::Result<()> {
        *self.calls.lock().expect("probe counter") += 1;
        self.result.run()
    }
}

struct PolicyDeniedProbe;

impl StartupProbe for PolicyDeniedProbe {
    fn run(&self) -> io::Result<()> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

fn requirements() -> StartupRequirements {
    StartupRequirements {
        system: NativeSystem::X86_64Linux,
        kernel: KernelVersion::new(6, 1),
        yama_scope: Some(1),
        sandbox_policy_ok: true,
    }
}

#[test]
fn startup_checks_are_ordered_and_probe_runs_only_after_admission() {
    let probe = RecordingProbe::new(ProbeResult::Pass);
    assert!(validate_startup(requirements(), &probe).is_ok());
    assert_eq!(probe.calls(), 1);

    let mut old_kernel = requirements();
    old_kernel.kernel = KernelVersion::new(3, 18);
    assert_eq!(
        validate_startup(old_kernel, &probe)
            .expect_err("old kernel must refuse")
            .code(),
        StartupCode::KernelTooOld
    );
    assert_eq!(probe.calls(), 1);

    let mut bad_yama = requirements();
    bad_yama.yama_scope = Some(2);
    assert_eq!(
        validate_startup(bad_yama, &probe)
            .expect_err("restricted Yama must refuse")
            .code(),
        StartupCode::YamaRefused
    );
    assert_eq!(probe.calls(), 1);
}

#[test]
fn unsupported_system_and_real_probe_failure_have_distinct_owners() {
    let probe = RecordingProbe::new(ProbeResult::Fail);
    let mut unsupported = requirements();
    unsupported.system = NativeSystem::Unsupported;
    assert_eq!(
        validate_startup(unsupported, &probe)
            .expect_err("unsupported system must refuse")
            .code(),
        StartupCode::UnsupportedSystem
    );
    assert_eq!(probe.calls(), 0);

    assert_eq!(
        validate_startup(requirements(), &probe)
            .expect_err("probe failure must refuse")
            .code(),
        StartupCode::ProbeFailed
    );
    assert_eq!(probe.calls(), 1);

    assert_eq!(
        validate_startup(requirements(), &PolicyDeniedProbe)
            .expect_err("required policy denial must be distinct")
            .code(),
        StartupCode::SandboxPolicyDrift
    );

    let probe = RecordingProbe::new(ProbeResult::Pass);
    let mut policy = requirements();
    policy.sandbox_policy_ok = false;
    assert_eq!(
        validate_startup(policy, &probe)
            .expect_err("sandbox policy drift must refuse")
            .code(),
        StartupCode::SandboxPolicyDrift
    );
    assert_eq!(probe.calls(), 1);
}

#[test]
fn supported_aarch64_and_the_linux_3_19_boundary_are_admitted() {
    let probe = RecordingProbe::new(ProbeResult::Pass);
    let requirements = StartupRequirements {
        system: NativeSystem::Aarch64Linux,
        kernel: KernelVersion::new(3, 19),
        yama_scope: Some(1),
        sandbox_policy_ok: true,
    };
    validate_startup(requirements, &probe).expect("AArch64 at Linux 3.19 is supported");
    assert_eq!(probe.calls(), 1);
}

#[test]
fn runtime_probe_is_a_distinct_non_injected_startup_seam() {
    let probe = RuntimeStartupProbe;
    let started = Instant::now();
    #[cfg(target_os = "linux")]
    if let Err(error) = probe.run() {
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::Other | io::ErrorKind::PermissionDenied | io::ErrorKind::TimedOut
            ),
            "runtime probe returned an unexpected error class"
        );
    }
    #[cfg(not(target_os = "linux"))]
    assert!(probe.run().is_err());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "runtime startup probe exceeded its bound"
    );
}
