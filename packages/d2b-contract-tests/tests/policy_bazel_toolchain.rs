//! Fixture-independent policy for the Spec 003 Bazel toolchain boundary.
//!
//! This test deliberately reads repository inputs rather than rendered Nix
//! fixtures. The toolchain and immutable C supervisor exist before the Bazel
//! generator, so their identity, protocol, policy, and recovery ownership
//! must be checked without a fixture build.

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use serde_json::Value;
use std::process::Command;

const BAZEL_NIX: &str = "pkgs/bazel-8.6.0-seccomp/default.nix";
const PATCH: &str = "pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch";
const POLICY: &str = "pkgs/bazel-8.6.0-seccomp/seccomp-policy.json";
const SUPERVISOR_NIX: &str = "pkgs/d2b-bazel-exec-supervisor/default.nix";
const SUPERVISOR: &str = "tests/tools/d2b-bazel-exec-supervisor/supervisor.c";
const PLANT: &str = "tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c";
const TOOLCHAIN_GOLDEN: &str = "tests/golden/bazel-toolchain.json";
const SUPERVISOR_GOLDEN: &str = "tests/golden/bazel-exec-supervisor.json";
const RUNBOOK: &str = "docs/contributing/critical-subsystems.md";
const DERIVATION_SHA256_METHOD: &str = "raw-drv-file-sha256";

fn json(path: &str) -> Value {
    serde_json::from_str(&read_repo_file(path))
        .unwrap_or_else(|error| panic!("{path} must be valid JSON: {error}"))
}

fn string(value: &Value, path: &str) -> String {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{path} must be a string"))
        .to_owned()
}

fn array_strings(value: &Value, path: &str) -> Vec<String> {
    value
        .pointer(path)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{path} must be an array"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("{path} contains a non-string entry"))
                .to_owned()
        })
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn assert_no_zero_sha256_placeholder(value: &Value, label: &str) {
    let zero_sha256 = "0".repeat(64);
    assert!(
        !value.to_string().contains(&zero_sha256),
        "{label} contains a zero SHA-256 placeholder"
    );
}

fn sha256_repo_file(relative: &str) -> String {
    let output = Command::new("sha256sum")
        .arg(repo_root().join(relative))
        .output()
        .expect("sha256sum must be available for native identity checks");
    assert!(output.status.success(), "sha256sum must hash {relative}");
    String::from_utf8(output.stdout)
        .expect("sha256sum output must be UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum output must contain a digest")
        .to_owned()
}

#[test]
fn every_toolchain_input_is_present_and_anchored() {
    for path in [
        BAZEL_NIX,
        PATCH,
        POLICY,
        SUPERVISOR_NIX,
        SUPERVISOR,
        PLANT,
        TOOLCHAIN_GOLDEN,
        SUPERVISOR_GOLDEN,
        "flake.nix",
        RUNBOOK,
    ] {
        assert!(
            repo_path_exists(path),
            "missing governed toolchain input {path}"
        );
    }
}

#[test]
fn policy_is_closed_no_network_and_constant_ptrace() {
    let policy = json(POLICY);
    assert_eq!(policy["schemaVersion"], 1);
    assert_eq!(policy["policyId"], "d2b-bazel-action-seccomp-v1");
    assert_eq!(policy["capabilityAbi"], "d2b-bazel-seccomp-abi-v1");
    assert_eq!(policy["defaultAction"], "allow");
    assert_eq!(policy["denial"]["action"], "errno");
    assert_eq!(policy["denial"]["errno"], "EACCES");
    assert_eq!(policy["denial"]["value"], 13);
    assert_eq!(policy["load"]["after"], "sandbox-construction");
    assert_eq!(policy["load"]["before"], "action-command-exec");
    assert_eq!(policy["load"]["noNewPrivs"], true);
    assert_eq!(policy["load"]["noFallback"], true);
    assert_eq!(policy["x86X32SyscallBit"]["rejectedBeforeDispatch"], true);
    assert_eq!(policy["x86X32SyscallBit"]["denial"]["action"], "errno");
    assert_eq!(policy["x86X32SyscallBit"]["denial"]["errno"], "EACCES");
    assert_eq!(policy["x86X32SyscallBit"]["denial"]["value"], 13);
    assert_eq!(
        array_strings(&policy, "/load/covers"),
        strings(&[
            "compile-build-command",
            "test-setup-command",
            "test-command",
            "descendants"
        ])
    );
    assert_eq!(
        array_strings(&policy, "/deniedSyscalls"),
        strings(&[
            "socket",
            "socketpair",
            "connect",
            "bind",
            "listen",
            "accept",
            "accept4",
            "sendto",
            "sendmsg",
            "sendmmsg",
            "recvfrom",
            "recvmsg",
            "recvmmsg",
            "shutdown",
            "getsockname",
            "getpeername",
            "setsockopt",
            "getsockopt",
            "pidfd_getfd",
            "io_uring_setup",
            "io_uring_enter",
            "io_uring_register",
            "socketcall"
        ])
    );

    let requests = policy["ptrace"]["requests"]
        .as_array()
        .expect("ptrace requests must be an array");
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests
            .iter()
            .map(|request| request["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "PTRACE_TRACEME",
            "PTRACE_SETOPTIONS",
            "PTRACE_CONT",
            "PTRACE_DETACH"
        ]
    );
    assert_eq!(requests[0]["pid"], 0);
    assert_eq!(requests[0]["address"]["value"], 0);
    assert_eq!(requests[0]["data"]["value"], 0);
    assert_eq!(requests[1]["address"]["value"], 0);
    assert_eq!(requests[1]["data"]["value"], 16);
    assert_eq!(
        requests[1]["data"]["expression"],
        "(void *)(uintptr_t)PTRACE_O_TRACEEXEC"
    );
    for request in &requests[1..] {
        assert_eq!(request["pid"]["dynamic"], true);
        assert_eq!(request["pid"]["owner"], "supervisor-owned-fork-result");
        assert_eq!(request["address"]["value"], 0);
    }
    assert_eq!(requests[2]["data"]["value"], 0);
    assert_eq!(requests[3]["data"]["value"], 0);
    assert_eq!(policy["ptrace"]["futureChildPidMatching"], false);
    assert_eq!(
        array_strings(&policy, "/ptrace/dynamicIdentity"),
        strings(&[
            "owned-fork-result",
            "confirmed-process-group",
            "direct-parent-relation",
            "traced-initial-stop",
            "sole-wait-owner",
            "PTRACE_EVENT_EXEC"
        ])
    );
    assert!(
        array_strings(&policy, "/ptrace/forbiddenRequests")
            .iter()
            .any(|entry| entry == "PTRACE_ATTACH")
    );
}

#[test]
fn policy_binds_fresh_pid_namespace_and_typed_quarantine() {
    let policy = json(POLICY);
    assert_eq!(policy["monitor"]["namespace"], "CLONE_NEWPID");
    assert_eq!(policy["monitor"]["pid1OwnsAbnormalTeardown"], true);
    assert_eq!(policy["monitor"]["pid1OutsideActionTree"], true);
    assert_eq!(policy["monitor"]["nonblockingReapProgress"], true);
    assert_eq!(policy["monitor"]["userspaceCeilingMs"], 10000);
    assert_eq!(policy["monitor"]["kernelCleanupBounded"], false);
    assert_eq!(policy["monitor"]["pendingState"], "pending-kernel-cleanup");
    assert_eq!(policy["monitor"]["quarantineBeforeConsumingReap"], true);
    assert_eq!(policy["monitor"]["successAfterQuarantine"], false);
    assert_eq!(policy["monitor"]["reuseBeforeConsumingReap"], false);
    assert_eq!(policy["monitor"]["replacementWaitOwner"], false);
    assert_eq!(policy["monitor"]["manualRelease"], false);
    assert_eq!(policy["monitor"]["reboot"], false);
    assert_eq!(
        policy["monitor"]["releaseRecord"],
        "D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE"
    );
    assert_eq!(
        policy["monitor"]["releaseCleanup"],
        "complete-after-quarantine"
    );
    assert_eq!(
        policy["monitor"]["releaseQuarantine"],
        "entered-and-released-after-consuming-reap"
    );
    assert_eq!(
        policy["monitor"]["runbook"],
        "docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine"
    );

    let toolchain = json(TOOLCHAIN_GOLDEN);
    let predicates = toolchain["preHelperPredicates"]
        .as_array()
        .expect("pre-helper predicate table must be an array");
    assert_eq!(predicates.len(), 5);
    assert_eq!(
        predicates
            .iter()
            .map(|predicate| predicate["code"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "D2B-BZLEXEC-NIX-PTRACE-SYSTEM",
            "D2B-BZLEXEC-TOOLCHAIN-PTRACE-KERNEL",
            "D2B-BZLEXEC-TOOLCHAIN-PTRACE-YAMA",
            "D2B-BZLEXEC-TOOLCHAIN-PTRACE-PROBE",
            "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY"
        ]
    );
    assert_eq!(predicates[0]["owner"], "nix-evaluation");
    assert_eq!(predicates[1]["owner"], "toolchain-startup");
    assert_eq!(predicates[2]["owner"], "toolchain-startup");
    assert_eq!(predicates[3]["owner"], "toolchain-startup");
    assert_eq!(predicates[4]["owner"], "patched-sandbox");
    for predicate in predicates {
        assert!(
            predicate["correction"]
                .as_str()
                .is_some_and(|correction| correction.contains("run make test-flake"))
        );
        assert!(predicate["locator"].as_str().is_some());
    }
}

#[test]
fn supervisor_has_exact_four_pointer_width_ptrace_calls() {
    let source = read_repo_file(SUPERVISOR);
    assert_eq!(
        source.matches("ptrace(").count(),
        4,
        "the supervisor must have exactly four ptrace call sites"
    );
    for call in [
        "ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)",
        "ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)",
        "ptrace(PTRACE_CONT, child, (void *)0, (void *)0)",
        "ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)",
    ] {
        assert!(source.contains(call), "missing exact ptrace call {call}");
    }
    for forbidden in [
        "PTRACE_ATTACH",
        "PTRACE_SEIZE",
        "PTRACE_GETREGS",
        "PTRACE_SYSCALL",
        "PTRACE_GETEVENTMSG",
        "CAP_SYS_PTRACE",
        "fexecve",
        "/proc/self/fd",
    ] {
        assert!(
            !source.contains(forbidden),
            "supervisor contains forbidden operation {forbidden}"
        );
    }
    assert!(source.contains("execveat"));
    assert!(source.contains("AT_EMPTY_PATH"));
    assert!(source.contains("pipe2(exec_pipe, O_CLOEXEC | O_NONBLOCK)"));
    assert_eq!(source.matches("fork()").count(), 1);
    assert!(source.contains("setpgid(0, 0)"));
    assert!(source.contains("setpgid(child, child)"));
    assert!(source.contains("\"D2BS\""));
    assert!(source.contains("\"D2BE\""));
    assert!(source.contains("D2B_EXEC_DEADLINE_MS 10000"));
    assert!(source.contains("D2B_STATUS_HEADER_SIZE 8"));
    assert!(source.contains("D2B_PRIVATE_EXECUTABLE_FD 9"));
    assert!(source.contains("D2B_STATUS_FD 8"));
    assert!(source.contains("D2B_READY"));
    assert!(source.contains("D2B_EXECUTED"));
    assert!(source.contains("D2B_EXITED"));
    assert!(source.contains("D2B_SIGNALED"));
    assert!(source.contains("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION"));
    assert!(source.contains("D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH"));
    assert!(source.contains("D2B-BZLEXEC-HELPER-PTRACE-EVENT"));
    assert!(source.contains("D2B-BZLEXEC-HELPER-PTRACE-DETACH"));
}

#[test]
fn patched_sandbox_keeps_policy_before_action_and_owns_all_sandbox_codes() {
    let patch = read_repo_file(PATCH);
    assert!(patch.contains("D2BPrepareActionPolicy();"));
    assert!(patch.contains("D2BInheritedDescriptorPreflight"));
    assert!(patch.contains("PR_SET_NO_NEW_PRIVS"));
    assert!(patch.contains("SECCOMP_MODE_FILTER"));
    assert!(patch.contains("SECCOMP_RET_ERRNO | EACCES"));
    assert!(patch.contains("D2B_BAZEL_SECCOMP_POLICY"));
    assert!(!patch.contains("/proc/self/exe"));
    assert!(patch.contains("PTRACE_TRACEME"));
    assert!(patch.contains("PTRACE_SETOPTIONS"));
    assert!(patch.contains("PTRACE_CONT"));
    assert!(patch.contains("PTRACE_DETACH"));
    assert!(patch.contains("future child pid is intentionally not compared"));
    assert!(patch.contains("CLONE_NEWPID"));
    assert!(patch.contains("kD2BUserspaceCeilingMs = 10000"));
    assert!(patch.contains("WNOHANG"));
    assert!(patch.contains("__X32_SYSCALL_BIT"));
    assert!(patch.contains("BPF_JMP | BPF_JSET | BPF_K"));
    assert!(
        patch.find("BPF_JMP | BPF_JSET | BPF_K").expect("x32 guard")
            < patch
                .find("std::vector<struct sock_filter> ptrace_rules")
                .expect("syscall dispatch")
    );
    for code in [
        "D2B-BZLEXEC-SANDBOX-NAMESPACE",
        "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY",
        "D2B-BZLEXEC-SANDBOX-MONITOR",
        "D2B-BZLEXEC-SANDBOX-KILL",
        "D2B-BZLEXEC-SANDBOX-REAP",
        "D2B-BZLEXEC-SANDBOX-CEILING",
        "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP",
        "D2B-BZLEXEC-SANDBOX-CLEANUP",
        "D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE",
    ] {
        assert!(
            patch.contains(code),
            "patched sandbox is missing recovery code {code}"
        );
    }
    assert!(patch.contains("no-success-no-reuse"));
    assert!(patch.contains("complete-after-quarantine"));
    assert!(patch.contains("entered-and-released-after-consuming-reap"));
    assert!(patch.contains(
        "docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine"
    ));
    for forbidden in [
        "retry-before-release",
        "replacement-waiter",
        "manual-release",
        "prohibited=reboot",
    ] {
        assert!(
            patch.contains(forbidden),
            "missing quarantine prohibition {forbidden}"
        );
    }
}

#[test]
fn x32_filter_mutation_is_rejected_before_dispatch() {
    let patch = read_repo_file(PATCH);
    let guard = "BPF_JMP | BPF_JSET | BPF_K";
    assert!(patch.contains(guard));
    let mutated = patch.replace(guard, "BPF_JMP | BPF_JEQ | BPF_K");
    assert!(
        !mutated.contains(guard),
        "the mutation must remove the pre-dispatch x32 guard"
    );
    assert!(
        mutated
            .find("__X32_SYSCALL_BIT")
            .expect("mutated x32 source")
            < mutated
                .find("std::vector<struct sock_filter> ptrace_rules")
                .expect("syscall dispatch")
    );
}

#[test]
fn strategy_lock_and_observation_inputs_are_closed() {
    let bazelrc = read_repo_file(".bazelrc");
    for setting in [
        "common --spawn_strategy=sandboxed",
        "common --strategy=Rustc=sandboxed",
        "common --strategy=RustcMetadata=sandboxed",
        "common --strategy=Clippy=sandboxed",
        "common --strategy=rustdoc=sandboxed",
        "common --strategy=rustfmt=sandboxed",
        "common --strategy=CargoBuildScript=sandboxed",
        "common --strategy=TestRunner=sandboxed",
    ] {
        assert!(
            bazelrc.contains(setting),
            "missing closed strategy {setting}"
        );
    }
    for forbidden in [
        "sandboxed,local",
        "sandboxed,standalone",
        "sandboxed,worker",
        "sandboxed,process",
    ] {
        assert!(
            !bazelrc.contains(forbidden),
            "strategy fallback {forbidden}"
        );
    }

    let rule = read_repo_file("bazel/rules/sandboxed_action.bzl");
    assert!(rule.contains("d2b_validate_effective_strategies"));
    assert!(rule.contains("D2B_STRATEGY_OVERRIDE_KEYS"));
    assert!(rule.contains("d2b.execution_strategy"));
    assert!(rule.contains("D2B_ACTION_NETWORK"));

    let package = read_repo_file(BAZEL_NIX);
    assert!(package.contains("D2B_BAZEL_STRATEGY_LOCK"));
    assert!(package.contains("D2B-BZLNET-STRATEGY"));
    assert!(package.contains("retry=make test-flake"));
    assert!(package.contains("strategyOverrides = false"));
}

#[test]
fn crash_plant_is_observable_and_has_beyond_ceiling_barrier() {
    let plant = read_repo_file(PLANT);
    for stage in [
        "before-ready",
        "after-ready",
        "after-executed",
        "during-grace",
        "direct-descendant",
        "double-fork-descendant",
        "beyond-ceiling",
    ] {
        assert!(plant.contains(stage), "plant stage is missing {stage}");
    }
    assert!(plant.contains("--liveness-path"));
    assert!(plant.contains("--barrier-path"));
    assert!(plant.contains("O_WRONLY | O_NONBLOCK | O_CLOEXEC"));
    assert!(plant.contains("O_RDONLY | O_CLOEXEC"));
    assert!(plant.contains("signal(SIGTERM, SIG_IGN)"));
    assert!(!plant.contains("open(\"/dev/null\""));
    assert!(!plant.contains("D2B_SANDBOX_PLANT_BARRIER_FD"));

    let mutated = plant.replace("O_WRONLY | O_NONBLOCK | O_CLOEXEC", "O_WRONLY");
    assert!(
        !mutated.contains("O_WRONLY | O_NONBLOCK | O_CLOEXEC"),
        "liveness descriptor mutation must be observable"
    );
}

#[test]
fn sandbox_diagnostics_are_closed_and_retryable() {
    let patch = read_repo_file(PATCH);
    for code in [
        "D2B-BZLEXEC-SANDBOX-NAMESPACE",
        "D2B-BZLEXEC-SANDBOX-PTRACE-POLICY",
        "D2B-BZLEXEC-SANDBOX-MONITOR",
        "D2B-BZLEXEC-SANDBOX-KILL",
        "D2B-BZLEXEC-SANDBOX-REAP",
        "D2B-BZLEXEC-SANDBOX-CEILING",
        "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP",
        "D2B-BZLEXEC-SANDBOX-CLEANUP",
    ] {
        assert!(patch.contains(code), "missing sandbox code {code}");
    }
    assert!(patch.contains("correction=Restore the pinned sandbox patch and policy"));
    assert!(patch.contains("retry=make test-flake"));
    assert!(patch.contains("result=failed;reuse=denied"));
    assert!(!patch.contains("pid="));
    assert!(!patch.contains("pgid="));
    assert!(!patch.contains("run_id"));
    assert!(!patch.contains("attempt_id"));
}

#[test]
fn package_and_flake_select_only_the_pinned_bazel_output() {
    let package = read_repo_file(BAZEL_NIX);
    let flake = read_repo_file("flake.nix");
    let supervisor_package = read_repo_file(SUPERVISOR_NIX);
    assert!(package.contains("version = \"8.6.0\""));
    assert!(package.contains("bazel-${version}-dist.zip"));
    assert!(package.contains("sha256-W22eB0IzHNZe3xaF8AZOkUTDCic3NXkypdqSDY61Su0="));
    assert!(package.contains("linux-sandbox-seccomp.patch"));
    assert!(package.contains("seccomp-policy.json"));
    assert!(package.contains("patches = (old.patches or [ ]) ++ [ sandboxPatch ]"));
    assert!(package.contains("D2B_BAZEL_SECCOMP_POLICY"));
    assert!(package.contains("wrapProgram \"$out/bin/bazel\""));
    assert!(package.contains("userspaceCeilingMs = 10000"));
    assert!(package.contains("futureChildPidMatching = false"));
    assert!(supervisor_package.contains("pkgsStatic"));
    assert!(supervisor_package.contains("-static"));
    assert!(supervisor_package.contains("d2b-bazel-exec-supervisor"));
    assert!(supervisor_package.contains("protocolVersion = 1"));
    assert!(supervisor_package.contains("linuxMinimum = \"3.19\""));
    assert!(supervisor_package.contains("capSysPtrace = false"));
    assert!(flake.contains("import ./pkgs/bazel-8.6.0-seccomp"));
    assert!(flake.contains("bazelSeccomp"));
    assert!(flake.contains("D2B-BZLEXEC-NIX-PTRACE-SYSTEM"));
    assert!(flake.contains("native-system"));
    assert!(flake.contains("run make test-flake"));
    assert!(!flake.contains("bazel_8"));
    assert!(!flake.contains("Bazelisk"));
}

#[test]
fn runbook_has_ordered_live_monitor_quarantine_steps() {
    let runbook = read_repo_file(RUNBOOK);
    let heading = "## Bazel pending kernel cleanup quarantine";
    let heading_position = runbook
        .find(heading)
        .expect("pending-kernel-cleanup runbook heading is required");
    let section = &runbook[heading_position..];
    for marker in [
        "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP",
        "original job and patched `linux-sandbox` monitor live",
        "Drain the affected CI worker or provider",
        "GitHub-hosted",
        "drain-without-terminate",
        "D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE",
        "cleanup=complete-after-quarantine",
        "quarantine=entered-and-released-after-consuming-reap",
        "do not retry",
        "Reboot, retry-before-release, replacement wait ownership, and manual release",
    ] {
        assert!(section.contains(marker), "runbook missing {marker}");
    }
    assert!(
        section.find("1. Keep").unwrap() < section.find("2. In").unwrap()
            && section.find("2. In").unwrap() < section.find("3. Drain").unwrap()
            && section.find("3. Drain").unwrap() < section.find("4. Wait").unwrap()
            && section.find("4. Wait").unwrap() < section.find("5. Confirm").unwrap()
            && section.find("5. Confirm").unwrap() < section.find("6. Only").unwrap()
    );
}

#[test]
fn golden_identity_records_are_redacted_and_native_scoped() {
    let toolchain = json(TOOLCHAIN_GOLDEN);
    let supervisor = json(SUPERVISOR_GOLDEN);
    assert_no_zero_sha256_placeholder(&toolchain, TOOLCHAIN_GOLDEN);
    assert_no_zero_sha256_placeholder(&supervisor, SUPERVISOR_GOLDEN);
    for value in [&toolchain, &supervisor] {
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            array_strings(value, "/supportedSystems"),
            strings(&["x86_64-linux", "aarch64-linux"])
        );
        let text = value.to_string();
        assert!(!text.contains("/nix/store/"));
        assert!(!text.contains("/home/"));
        assert!(!text.contains("pid="));
        assert!(!text.contains("run_id"));
    }
    assert_eq!(
        string(&toolchain, "/derivationSha256Method"),
        DERIVATION_SHA256_METHOD
    );
    assert_eq!(
        string(&supervisor, "/derivationSha256Method"),
        DERIVATION_SHA256_METHOD
    );
    for path in [
        "/source/sha256",
        "/patch/sha256",
        "/policy/sha256",
        "/output/narSha256",
        "/output/executableSha256",
        "/capabilityAbi/sha256",
    ] {
        assert_eq!(
            string(&toolchain, path).len(),
            64,
            "{path} must be hex digest"
        );
    }
    for system in ["x86_64-linux", "aarch64-linux"] {
        let row = &toolchain["nativeOutputs"][system];
        for path in ["/derivationSha256", "/narSha256", "/executableSha256"] {
            let digest = row
                .pointer(path)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{system}{path} must be a string"));
            assert_eq!(digest.len(), 64, "{system}{path} must be hex digest");
            assert_ne!(
                digest,
                "0".repeat(64),
                "{system}{path} must not be a zero placeholder"
            );
        }
    }
    assert_eq!(
        string(&toolchain, "/nativeOutputs/x86_64-linux/derivationSha256"),
        "5c00bb451a0851f096f4a396bc4efd0bed2deedaf1c37ac649ee3a988c03116d"
    );
    assert_eq!(
        string(&toolchain, "/nativeOutputs/aarch64-linux/derivationSha256"),
        "84a3d3df481794798afbdd9459073cb6e8c2ff845b028066bceb01687574b9e5"
    );
    assert_eq!(
        string(&toolchain, "/nativeOutputs/aarch64-linux/narSha256"),
        "8318412b0722765167051e15ff735819b8e9f0b2ab619e1653975c25db3bbb16"
    );
    assert_eq!(
        string(&toolchain, "/nativeOutputs/aarch64-linux/executableSha256"),
        "9898ce560dc199283b26c9f0efee8a217c53f45d1687a0c6b0c36cb9a2d7ee59"
    );
    assert_eq!(
        string(
            &toolchain,
            "/nativeOutputs/aarch64-linux/startupProbe/filterLoad"
        ),
        "observed"
    );
    for path in [
        "/source/sha256",
        "/expression/sha256",
        "/identity/protocolSha256",
        "/output/narSha256",
        "/output/executableSha256",
        "/output/staticElfSha256",
    ] {
        assert_eq!(
            string(&supervisor, path).len(),
            64,
            "{path} must be hex digest"
        );
    }
    for system in ["x86_64-linux", "aarch64-linux"] {
        let row = &supervisor["nativeOutputs"][system];
        for path in ["/derivationSha256", "/narSha256", "/executableSha256"] {
            let digest = row
                .pointer(path)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{system}{path} must be a string"));
            assert_eq!(digest.len(), 64, "{system}{path} must be hex digest");
            assert_ne!(
                digest,
                "0".repeat(64),
                "{system}{path} must not be a zero placeholder"
            );
        }
    }
    assert_eq!(supervisor["protocol"]["version"], 1);
    assert_eq!(supervisor["protocol"]["privateExecutableFd"], 9);
    assert_eq!(supervisor["protocol"]["statusFd"], 8);
    assert_eq!(supervisor["protocol"]["status"]["retainedBufferBytes"], 27);
    assert_eq!(
        supervisor["protocol"]["status"]["noStatusOverlongProbe"],
        true
    );
    assert_eq!(supervisor["linuxMinimum"], "3.19");
    assert_eq!(supervisor["yama"]["capSysPtrace"], false);
    assert_eq!(
        array_strings(&supervisor, "/sourceFiles"),
        strings(&[
            "tests/tools/d2b-bazel-exec-supervisor/supervisor.c",
            "tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c"
        ])
    );
}

#[test]
fn current_sandbox_and_supervisor_source_hashes_are_bound_to_golden_identity() {
    let toolchain = json(TOOLCHAIN_GOLDEN);
    let supervisor = json(SUPERVISOR_GOLDEN);
    assert_eq!(string(&toolchain, "/patch/sha256"), sha256_repo_file(PATCH));
    assert_eq!(
        string(&toolchain, "/policy/sha256"),
        sha256_repo_file(POLICY)
    );
    assert_eq!(
        string(&supervisor, "/source/sha256"),
        sha256_repo_file(SUPERVISOR)
    );
    assert_eq!(
        string(&supervisor, "/source/plantSha256"),
        sha256_repo_file(PLANT)
    );
    assert_eq!(
        string(&supervisor, "/expression/sha256"),
        sha256_repo_file(SUPERVISOR_NIX)
    );
    let flake = read_repo_file("flake.nix");
    assert!(flake.contains("bazelSourceIdentityGate"));
    assert!(flake.contains("currentBazelSourceHashes"));
    assert!(!flake.contains("phase-valid"));
}
