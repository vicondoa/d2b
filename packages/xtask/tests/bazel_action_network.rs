#[path = "../src/hermeticity.rs"]
mod hermeticity;

#[test]
fn complete_inventory_covers_configured_aquery_and_strategy_sets() {
    let inventory = hermeticity::complete_action_network_inventory();
    hermeticity::validate_action_network_inventory(&inventory).expect("complete inventory");
    assert_eq!(inventory.action_network, "none");
    assert_eq!(inventory.sandbox_provider, "pkgs/bazel-8.6.0-seccomp");
    assert_eq!(inventory.capability_abi, "d2b-bazel-seccomp-abi-v1");
    assert!(inventory.repository_fetches_outside_actions);
    assert!(inventory.fallback_strategies.is_empty());
    assert!(
        inventory
            .strategy_inventory
            .values()
            .all(|strategy| strategy == "sandboxed" || strategy == "linux-sandbox")
    );
}

#[test]
fn non_sandboxed_strategy_and_missing_pre_action_plants_refuse() {
    let mut inventory = hermeticity::complete_action_network_inventory();
    inventory
        .strategy_inventory
        .insert("stable:Rustc".to_owned(), "process".to_owned());
    let error = hermeticity::validate_action_network_inventory(&inventory)
        .expect_err("process strategy must refuse");
    assert!(error.to_string().contains("non-sandbox"));

    let mut inventory = hermeticity::complete_action_network_inventory();
    inventory
        .socket_plants
        .retain(|plant| plant != "action-network-io-uring");
    assert!(matches!(
        hermeticity::validate_action_network_inventory(&inventory),
        Err(hermeticity::ActionNetworkError::MissingPlant(_))
    ));
}

#[test]
fn rendered_action_network_inventory_is_deterministic_json() {
    let first = hermeticity::action_network_json().expect("first inventory");
    let second = hermeticity::action_network_json().expect("second inventory");
    assert_eq!(first, second);
    assert!(first.ends_with('\n'));
    assert!(first.contains("\"action_network\": \"none\""));
    assert!(first.contains("\"strategy_inventory\""));
}

#[test]
fn observed_configured_aquery_and_effective_strategy_sets_reconcile() {
    let configured = serde_json::json!({
        "actionKinds": hermeticity::GOVERNED_ACTION_KINDS,
        "cqueryTargets": hermeticity::RULES_RUST_EVIDENCE_LABELS,
        "coverageTargets": hermeticity::CONFIGURED_COVERAGE_TARGETS,
    })
    .to_string();
    let aquery = serde_json::json!({
        "actions": hermeticity::GOVERNED_ACTION_KINDS
            .iter()
            .map(|kind| serde_json::json!({"kind": kind}))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let strategies = serde_json::json!({
        "strategies": hermeticity::GOVERNED_ACTION_KINDS
            .iter()
            .map(|kind| (*kind, hermeticity::SANDBOX_STRATEGY))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "fallbackStrategies": [],
        "actionEnvironment": ["BASH_ENV", "PATH"],
        "testEnvironment": [],
    })
    .to_string();
    let observation =
        hermeticity::ActionNetworkObservation::from_json(&configured, &aquery, &strategies)
            .expect("real observation shape");
    let inventory = hermeticity::complete_action_network_inventory_from_observation(&observation);
    hermeticity::validate_observed_action_network(&inventory, &observation)
        .expect("observations reconcile");
}

#[test]
fn observed_strategy_mutation_refuses_local_fallback() {
    let configured = serde_json::json!({
        "actionKinds": hermeticity::GOVERNED_ACTION_KINDS,
        "cqueryTargets": hermeticity::RULES_RUST_EVIDENCE_LABELS,
        "coverageTargets": hermeticity::CONFIGURED_COVERAGE_TARGETS,
    })
    .to_string();
    let aquery = serde_json::json!({
        "actions": hermeticity::GOVERNED_ACTION_KINDS,
    })
    .to_string();
    let strategies = serde_json::json!({
        "strategies": {
            "stable:Rustc": "local",
            "stable:RustcMetadata": "sandboxed",
        },
    })
    .to_string();
    let observation =
        hermeticity::ActionNetworkObservation::from_json(&configured, &aquery, &strategies)
            .expect("mutated observation parses");
    let inventory = hermeticity::complete_action_network_inventory_from_observation(&observation);
    assert!(matches!(
        hermeticity::validate_observed_action_network(&inventory, &observation),
        Err(hermeticity::ActionNetworkError::WrongStrategy { .. })
    ));
}

#[test]
fn pinned_toolchain_record_requires_both_native_outputs_and_patch_identity() {
    let record = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .join("tests/golden/bazel-toolchain.json"),
    )
    .expect("toolchain record");
    hermeticity::validate_pinned_toolchain_record(&record).expect("pinned record");
    let mut wrong = record;
    wrong = wrong.replace("\"sandboxed\"", "\"process\"");
    assert!(hermeticity::validate_pinned_toolchain_record(&wrong).is_err());
}

#[test]
fn inherited_descriptor_diagnostics_are_typed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let patch =
        std::fs::read_to_string(root.join("pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch"))
            .expect("sandbox patch");
    for (code, outcome) in [
        (
            "kD2BPreflightFailure",
            "outcome=descriptor-census-failed action=refused",
        ),
        (
            "kD2BInheritedSocketFailure",
            "outcome=inherited-socket action=refused",
        ),
        (
            "kD2BInheritedRingFailure",
            "outcome=inherited-io-uring action=refused",
        ),
    ] {
        assert!(patch.contains(code), "missing descriptor code {code}");
        assert!(
            patch.contains(outcome),
            "missing descriptor outcome {outcome}"
        );
    }
    assert!(!patch.contains("D2BPrintSandboxDiagnostic(kD2BInheritedSocketFailure)"));
    assert!(!patch.contains("D2BPrintSandboxDiagnostic(kD2BInheritedRingFailure)"));
    let policy: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("pkgs/bazel-8.6.0-seccomp/seccomp-policy.json"))
            .expect("seccomp policy"),
    )
    .expect("valid seccomp policy");
    assert_eq!(
        policy["inheritedDescriptorPreflight"]["failureCodes"],
        serde_json::json!({
            "descriptorCensus": "D2B-BZLNET-PREFLIGHT",
            "inheritedSocket": "D2B-BZLNET-INHERITED-SOCKET",
            "inheritedRing": "D2B-BZLNET-INHERITED-RING",
        })
    );
}

#[test]
fn bazel_wrapper_anchors_rc_and_environment_policy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let wrapper = std::fs::read_to_string(root.join("pkgs/bazel-8.6.0-seccomp/default.nix"))
        .expect("Bazel wrapper");
    let bazelrc = std::fs::read_to_string(root.join(".bazelrc")).expect("Bazel rc");
    for setting in [
        "--nosystem_rc",
        "--nohome_rc",
        "--noworkspace_rc",
        "--bazelrc=",
        "--unset BAZELRC",
        "--unset BAZEL_OPTS",
        "--unset BAZEL_WRAPPER",
        "nativeExecutable",
        "--incompatible_strict_action_env",
    ] {
        assert!(
            wrapper.contains(setting) || bazelrc.contains(setting),
            "missing {setting}"
        );
    }
    for forbidden in [
        "--system_rc",
        "--home_rc",
        "--workspace_rc",
        "--ignore_all_rc_files",
        "--action_env",
        "--test_env",
        "--remote_executor",
        "--spawn_strategy",
        "--invocation_policy",
        "--flagfile",
        "--noenable_bzlmod",
        "--enable_workspace",
    ] {
        assert!(
            wrapper.contains(forbidden),
            "wrapper does not reject {forbidden}"
        );
    }
    assert!(!bazelrc.contains("--action_env="));
    assert!(!bazelrc.contains("--test_env="));
    assert!(wrapper.contains("--unset BAZEL_INTERNAL_INVOCATION_POLICY"));
}

#[test]
fn evidence_uses_real_rules_rust_rules_and_persists_names_only() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let build =
        std::fs::read_to_string(root.join("bazel/evidence/BUILD.bazel")).expect("evidence BUILD");
    let environment = std::fs::read_to_string(root.join("bazel/evidence/environment_probe.c"))
        .expect("environment probe");
    for rule in [
        "cargo_build_script(",
        "rust_binary(",
        "rust_clippy(",
        "rust_doc(",
        "rust_doc_test(",
        "rust_library(",
        "rust_test(",
        "rust_unpretty(",
        "rustfmt_test(",
    ] {
        assert!(
            build.contains(rule),
            "missing genuine rules_rust rule {rule}"
        );
    }
    assert!(!build.contains("d2b_action_probe"));
    assert!(
        !root
            .join("bazel/evidence/action-network-observation.json")
            .exists()
    );
    assert!(environment.contains("environment-names-only-v1"));
    assert!(!environment.contains("fputs(*entry"));
}

#[test]
fn cargo_bazel_selects_checksum_pinned_native_architectures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let module = std::fs::read_to_string(root.join("MODULE.bazel")).expect("MODULE.bazel");
    let repository_rule =
        std::fs::read_to_string(root.join("bazel/cargo/cargo_bazel.bzl")).expect("cargo rule");
    for value in [
        "cargo-bazel-x86_64-unknown-linux-gnu",
        "cargo-bazel-aarch64-unknown-linux-gnu",
        "dcf36eda09624c7cdd700103959a44a10b49a90d2efe1dd41c226a8cf4706846",
        "736b045185326560c677256435685cef9fd734ce4d5e7d41ac7f0e27dd9c2a69",
    ] {
        assert!(
            module.contains(value),
            "missing native cargo-bazel pin {value}"
        );
    }
    for value in [
        "repository_ctx.os.arch",
        "aarch64_url",
        "aarch64_sha256",
        "x86_64_url",
        "x86_64_sha256",
        "unsupported native cargo-bazel architecture",
    ] {
        assert!(
            repository_rule.contains(value),
            "cargo-bazel rule is missing {value}"
        );
    }
}

#[cfg(unix)]
mod native_patched_bazel {
    use super::hermeticity;
    use std::{
        collections::{BTreeMap, BTreeSet},
        env,
        fs::{self, File, OpenOptions},
        io::{self, Read, Write},
        os::unix::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        process::{Command, Output, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const SECRET_NAME: &str = "D2B_PLANTED_ACTION_SECRET";

    use nix::{
        sys::{
            signal::{Signal, kill},
            stat::Mode,
        },
        unistd::{Pid, mkfifo},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask repository root")
            .to_path_buf()
    }

    fn bazel_executable() -> PathBuf {
        if let Some(path) = env::var_os("D2B_BAZEL_NATIVE_BIN") {
            return PathBuf::from(path);
        }
        if let Some(path) = env::var_os("BAZEL") {
            return PathBuf::from(path);
        }
        let path = env::var_os("PATH").expect("native Bazel PATH");
        env::split_paths(&path)
            .map(|directory| directory.join("bazel"))
            .find(|candidate| candidate.is_file())
            .expect("D2B_BAZEL_NATIVE_BIN or pinned Bazel on PATH")
    }

    fn scratch_root(root: &Path) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("native plant clock")
            .as_nanos();
        let scratch = root.join(".scratch").join(format!(
            "bazel-native-plants-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&scratch).expect("native plant scratch");
        scratch
    }

    fn configure_environment(command: &mut Command, secret: &str) {
        command.env_clear();
        for name in ["PATH", "JAVA_HOME", "TMPDIR"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env(SECRET_NAME, secret);
    }

    fn channel_root(scratch: &Path, channel: &str) -> PathBuf {
        scratch.join(channel)
    }

    fn bazel_arguments(
        scratch: &Path,
        channel: &str,
        command: &str,
        execution_log: Option<&Path>,
    ) -> Vec<String> {
        let channel_root = channel_root(scratch, channel);
        let mut arguments = vec![
            "--batch".to_owned(),
            format!("--output_user_root={}", channel_root.join("user").display()),
            format!("--output_base={}", channel_root.join("base").display()),
            command.to_owned(),
            "--enable_bzlmod".to_owned(),
            "--noenable_workspace".to_owned(),
            format!("--@rules_rust//rust/toolchain/channel={channel}"),
            "--@rules_rust//rust/settings:pipelined_compilation=true".to_owned(),
            format!("--sandbox_writable_path={}", scratch.display()),
        ];
        if channel == "nightly" {
            arguments.push(
                "--@rules_rust//rust/settings:experimental_compile_rustdoc_tests=true".to_owned(),
            );
        }
        if let Some(execution_log) = execution_log {
            arguments.push(format!(
                "--execution_log_json_file={}",
                execution_log.display()
            ));
        }
        arguments
    }

    struct NativeBazel<'a> {
        executable: &'a Path,
        root: &'a Path,
        scratch: &'a Path,
        secret: &'a str,
    }

    fn run_bazel(
        bazel: &NativeBazel<'_>,
        channel: &str,
        command_name: &str,
        labels: &[&str],
        execution_log: Option<&Path>,
        defines: &[(&str, &str)],
    ) -> Output {
        let mut command = Command::new(bazel.executable);
        command.current_dir(bazel.root);
        configure_environment(&mut command, bazel.secret);
        let mut arguments = bazel_arguments(bazel.scratch, channel, command_name, execution_log);
        for (name, value) in defines {
            arguments.push(format!("--define={name}={value}"));
        }
        command.args(arguments);
        command.args(labels);
        command.output().expect("run pinned Bazel action")
    }

    fn run_query(
        bazel: &NativeBazel<'_>,
        channel: &str,
        kind: &str,
        output: &str,
        expression: &str,
    ) -> Output {
        let mut arguments = bazel_arguments(bazel.scratch, channel, kind, None);
        arguments.push(format!("--output={output}"));
        arguments.push(expression.to_owned());
        let mut command = Command::new(bazel.executable);
        command.current_dir(bazel.root);
        configure_environment(&mut command, bazel.secret);
        command.args(arguments);
        command.output().expect("run pinned Bazel query")
    }

    fn run_bzlmod_repository_query(
        bazel: &Path,
        root: &Path,
        scratch: &Path,
        expression: &str,
        secret: &str,
    ) -> Output {
        let stable_root = channel_root(scratch, "stable");
        let mut command = Command::new(bazel);
        command.current_dir(root);
        configure_environment(&mut command, secret);
        command.args([
            "--batch".to_owned(),
            format!("--output_user_root={}", stable_root.join("user").display()),
            format!("--output_base={}", stable_root.join("base").display()),
            "query".to_owned(),
            "--enable_bzlmod".to_owned(),
            "--noenable_workspace".to_owned(),
            "--output=label".to_owned(),
            expression.to_owned(),
        ]);
        command.output().expect("run Bzlmod repository query")
    }

    fn assert_secret_absent(secret: &str, label: &str, bytes: &[u8]) {
        assert!(
            !String::from_utf8_lossy(bytes).contains(secret),
            "{label} persisted the planted secret"
        );
    }

    fn configured_labels(output: &Output) -> BTreeSet<String> {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|label| label.starts_with("//bazel/evidence:"))
            .map(str::to_owned)
            .collect()
    }

    fn normalized_aquery_actions(channel: &str, output: &Output) -> BTreeSet<String> {
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("aquery JSON");
        value
            .get("actions")
            .and_then(serde_json::Value::as_array)
            .expect("aquery actions")
            .iter()
            .filter_map(|action| action.get("mnemonic").and_then(serde_json::Value::as_str))
            .filter_map(|mnemonic| hermeticity::normalize_rules_rust_mnemonic(channel, mnemonic))
            .collect()
    }

    fn normalized_execution_actions(
        channel: &str,
        paths: &[&Path],
        secret: &str,
    ) -> BTreeMap<String, String> {
        let mut actions = BTreeMap::new();
        for path in paths {
            let contents = fs::read_to_string(path).expect("execution log");
            assert!(!contents.contains(secret), "execution log persisted secret");
            for entry in
                serde_json::Deserializer::from_str(&contents).into_iter::<serde_json::Value>()
            {
                let entry = entry.expect("execution-log JSON record");
                let Some(mnemonic) = entry.get("mnemonic").and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                let Some(action) = hermeticity::normalize_rules_rust_mnemonic(channel, mnemonic)
                else {
                    continue;
                };
                let runner = entry
                    .get("runner")
                    .and_then(serde_json::Value::as_str)
                    .expect("governed execution runner");
                assert_eq!(runner, "linux-sandbox", "{action} escaped linux-sandbox");
                actions.insert(action, runner.to_owned());
            }
        }
        actions
    }

    fn patched_linux_sandbox(scratch: &Path, bazel: &Path) -> PathBuf {
        let store_root = bazel
            .parent()
            .and_then(Path::parent)
            .expect("pinned Bazel store root");
        let installed_policy = store_root.join("share/d2b/bazel/seccomp-policy.json");
        assert!(
            installed_policy.is_file(),
            "pinned Bazel policy is installed beside the executable"
        );
        let install_root = scratch.join("stable/user/install");
        fs::read_dir(&install_root)
            .expect("Bazel install root")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("linux-sandbox"))
            .find(|candidate| candidate.is_file())
            .expect("patched Bazel linux-sandbox")
    }

    fn create_fifo(path: &Path) -> (File, File) {
        mkfifo(path, Mode::from_bits_truncate(0o600)).expect("create native plant FIFO");
        let reader = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
            .open(path)
            .expect("open native plant FIFO reader");
        let writer = OpenOptions::new()
            .write(true)
            .custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
            .open(path)
            .expect("open native plant FIFO writer");
        (reader, writer)
    }

    fn wait_for_marker(reader: &mut File, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut byte = [0_u8; 1];
        loop {
            match reader.read(&mut byte) {
                Ok(1) => return,
                Ok(0) => {}
                Ok(_) => unreachable!("one-byte native plant read"),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("native plant liveness read failed: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "native plant did not publish liveness marker"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_close(reader: &mut File, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut byte = [0_u8; 1];
        loop {
            match reader.read(&mut byte) {
                Ok(0) => return,
                Ok(_) => panic!("native plant descendant retained the liveness FIFO"),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("native plant close read failed: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "native plant liveness FIFO stayed open after sandbox exit"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn run_crash_stage(
        bazel: &Path,
        root: &Path,
        scratch: &Path,
        stage: &str,
        index: usize,
        secret: &str,
    ) -> Output {
        let stage_root = scratch.join(format!("stage-{index}-{stage}"));
        fs::create_dir_all(&stage_root).expect("native stage root");
        let liveness_path = stage_root.join("liveness.fifo");
        let barrier_path = stage_root.join("barrier.fifo");
        let (mut liveness_reader, liveness_writer) = create_fifo(&liveness_path);
        drop(liveness_writer);
        let (_barrier_reader, mut barrier_writer) = create_fifo(&barrier_path);
        let liveness = liveness_path.to_string_lossy().into_owned();
        let barrier = barrier_path.to_string_lossy().into_owned();
        let defines = [
            ("D2B_STAGE", stage),
            ("D2B_LIVENESS_PATH", liveness.as_str()),
            ("D2B_BARRIER_PATH", barrier.as_str()),
        ];
        let mut command = Command::new(bazel);
        command.current_dir(root);
        configure_environment(&mut command, secret);
        command.args(bazel_arguments(scratch, "stable", "build", None));
        for (name, value) in defines {
            command.arg(format!("--define={name}={value}"));
        }
        command.arg("//bazel/evidence:crash-plant-action");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = command.spawn().expect("spawn native crash plant Bazel");
        wait_for_marker(&mut liveness_reader, Duration::from_secs(10));
        if stage == "during-grace" {
            barrier_writer
                .write_all(b"release")
                .expect("release native grace barrier");
        }
        if stage == "beyond-ceiling" {
            kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM)
                .expect("signal native beyond-ceiling Bazel");
        }
        let output = child
            .wait_with_output()
            .expect("wait for native crash plant Bazel");
        wait_for_close(&mut liveness_reader, Duration::from_secs(15));
        output
    }

    fn run_beyond_ceiling_direct(
        linux_sandbox: &Path,
        crash_plant: &Path,
        policy: &Path,
        scratch: &Path,
        secret: &str,
    ) -> Output {
        let stage_root = scratch.join("stage-direct-beyond-ceiling");
        fs::create_dir_all(&stage_root).expect("direct beyond-ceiling root");
        let liveness_path = stage_root.join("liveness.fifo");
        let barrier_path = stage_root.join("barrier.fifo");
        let (mut liveness_reader, liveness_writer) = create_fifo(&liveness_path);
        drop(liveness_writer);
        let (_barrier_reader, mut barrier_writer) = create_fifo(&barrier_path);

        let mut command = Command::new(linux_sandbox);
        command.current_dir(scratch);
        configure_environment(&mut command, secret);
        command.env("D2B_BAZEL_SECCOMP_POLICY", policy);
        command.env("D2B_BAZEL_STRATEGY_LOCK", "d2b-bazel-sandbox-v1");
        command.args([
            "-W",
            scratch.to_str().expect("UTF-8 native sandbox root"),
            "-w",
            scratch.to_str().expect("UTF-8 native sandbox root"),
            "-M",
            "/nix/store",
            "-m",
            "/nix/store",
            "--",
        ]);
        command.arg(crash_plant);
        command.args([
            "--stage",
            "beyond-ceiling",
            "--liveness-path",
            liveness_path.to_str().expect("UTF-8 liveness path"),
            "--barrier-path",
            barrier_path.to_str().expect("UTF-8 barrier path"),
        ]);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .expect("spawn direct beyond-ceiling sandbox");
        wait_for_marker(&mut liveness_reader, Duration::from_secs(10));
        kill(Pid::from_raw(child.id() as i32), Signal::SIGUSR1)
            .expect("mark direct sandbox teardown abnormal");
        thread::sleep(Duration::from_millis(10_500));
        assert!(
            child
                .try_wait()
                .expect("observe original monitor")
                .is_none(),
            "original monitor exited before consuming-reap release"
        );
        barrier_writer
            .write_all(b"release")
            .expect("release direct beyond-ceiling barrier");
        let output = child
            .wait_with_output()
            .expect("wait for original direct sandbox monitor");
        wait_for_close(&mut liveness_reader, Duration::from_secs(15));
        output
    }

    #[test]
    #[ignore = "requires a native patched Bazel and Linux namespace support"]
    fn native_bazel_runs_network_descriptor_and_cleanup_plants() {
        let root = repository_root();
        let scratch = scratch_root(&root);
        let bazel = bazel_executable();
        let expected_system =
            env::var("D2B_BAZEL_NATIVE_SYSTEM").expect("native CI system contract");
        let actual_system = match env::consts::ARCH {
            "x86_64" => "x86_64-linux",
            "aarch64" => "aarch64-linux",
            architecture => panic!("unsupported native Bazel architecture {architecture}"),
        };
        assert_eq!(
            expected_system, actual_system,
            "native Bazel plants cannot run through a foreign system"
        );
        let secret = format!(
            "d2b-native-secret-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("secret clock")
                .as_nanos()
        );
        let native_bazel = NativeBazel {
            executable: &bazel,
            root: &root,
            scratch: &scratch,
            secret: &secret,
        };

        for bypass in [
            "--invocation_policy=/dev/null",
            "--flagfile=/dev/null",
            "--noenable_bzlmod",
            "--enable_workspace",
        ] {
            let mut command = Command::new(&bazel);
            command.current_dir(&root);
            configure_environment(&mut command, &secret);
            command.args(["--batch", bypass, "help"]);
            let output = command.output().expect("run wrapper bypass refusal");
            assert_eq!(output.status.code(), Some(64), "{bypass} was not refused");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("D2B-BZLNET-STRATEGY"),
                "{bypass} omitted typed refusal"
            );
            assert_secret_absent(&secret, "wrapper refusal stdout", &output.stdout);
            assert_secret_absent(&secret, "wrapper refusal stderr", &output.stderr);
        }
        let mut internal_policy = Command::new(&bazel);
        internal_policy.current_dir(&root);
        configure_environment(&mut internal_policy, &secret);
        internal_policy.env("BAZEL_INTERNAL_INVOCATION_POLICY", &secret);
        internal_policy.args(["--batch", "help"]);
        let internal_output = internal_policy
            .output()
            .expect("run internal invocation-policy neutralization");
        assert!(
            internal_output.status.success(),
            "internal invocation-policy environment was not neutralized"
        );
        assert_secret_absent(
            &secret,
            "internal invocation-policy stdout",
            &internal_output.stdout,
        );
        assert_secret_absent(
            &secret,
            "internal invocation-policy stderr",
            &internal_output.stderr,
        );

        let network = run_bazel(
            &native_bazel,
            "stable",
            "build",
            &["//bazel/evidence:network-plant-action"],
            None,
            &[],
        );
        assert!(
            network.status.success(),
            "network denial plant failed: {}",
            String::from_utf8_lossy(&network.stderr)
        );
        assert_secret_absent(&secret, "network stdout", &network.stdout);
        assert_secret_absent(&secret, "network stderr", &network.stderr);

        for (repository, expected) in [
            (
                "@cargo_bazel_pinned//:cargo-bazel",
                "@cargo_bazel_pinned//:cargo-bazel",
            ),
            ("@product//:all", "@product//"),
        ] {
            let query = run_bzlmod_repository_query(&bazel, &root, &scratch, repository, &secret);
            assert!(
                query.status.success(),
                "Bzlmod query for {repository} failed: {}",
                String::from_utf8_lossy(&query.stderr)
            );
            assert!(
                String::from_utf8_lossy(&query.stdout).contains(expected),
                "Bzlmod query for {repository} did not exercise its repository"
            );
            assert_secret_absent(&secret, "Bzlmod query stdout", &query.stdout);
            assert_secret_absent(&secret, "Bzlmod query stderr", &query.stderr);
        }

        let labels_expression =
            format!("set({})", hermeticity::RULES_RUST_EVIDENCE_LABELS.join(" "));
        let stable_aquery_expression = concat!(
            "deps(set(",
            "//bazel/evidence:rules-rust-evidence-build ",
            "//bazel/evidence:evidence-test ",
            "//bazel/evidence:evidence-doctest ",
            "//bazel/evidence:evidence-rustfmt))"
        );
        let nightly_aquery_expression = concat!(
            "deps(set(",
            "//bazel/evidence:rules-rust-evidence-nightly-build ",
            "//bazel/evidence:evidence-test ",
            "//bazel/evidence:evidence-doctest ",
            "//bazel/evidence:evidence-rustfmt))"
        );
        let mut observed_actions = BTreeSet::new();
        let mut observed_execution = BTreeMap::new();
        let mut observed_labels = BTreeSet::new();

        for (channel, build_target, aquery_expression) in [
            (
                "stable",
                "//bazel/evidence:rules-rust-evidence-build",
                stable_aquery_expression,
            ),
            (
                "nightly",
                "//bazel/evidence:rules-rust-evidence-nightly-build",
                nightly_aquery_expression,
            ),
        ] {
            let cquery = run_query(
                &native_bazel,
                channel,
                "cquery",
                "label",
                &labels_expression,
            );
            assert!(
                cquery.status.success(),
                "{channel} cquery failed: {}",
                String::from_utf8_lossy(&cquery.stderr)
            );
            assert_secret_absent(&secret, "cquery stdout", &cquery.stdout);
            assert_secret_absent(&secret, "cquery stderr", &cquery.stderr);
            let labels = configured_labels(&cquery);
            let expected_labels = hermeticity::RULES_RUST_EVIDENCE_LABELS
                .iter()
                .map(|label| (*label).to_owned())
                .collect::<BTreeSet<_>>();
            assert_eq!(labels, expected_labels, "{channel} cquery target drift");
            observed_labels.extend(labels);

            let aquery = run_query(
                &native_bazel,
                channel,
                "aquery",
                "jsonproto",
                aquery_expression,
            );
            assert!(
                aquery.status.success(),
                "{channel} aquery failed: {}",
                String::from_utf8_lossy(&aquery.stderr)
            );
            assert_secret_absent(&secret, "aquery stdout", &aquery.stdout);
            assert_secret_absent(&secret, "aquery stderr", &aquery.stderr);
            observed_actions.extend(normalized_aquery_actions(channel, &aquery));

            let metadata_log = scratch.join(format!("{channel}-metadata-execution.json"));
            let metadata = run_bazel(
                &native_bazel,
                channel,
                "build",
                &[
                    "--output_groups=build_metadata",
                    "//bazel/evidence:evidence-library",
                ],
                Some(&metadata_log),
                &[],
            );
            assert!(
                metadata.status.success(),
                "{channel} RustcMetadata build failed: {}",
                String::from_utf8_lossy(&metadata.stderr)
            );
            assert_secret_absent(&secret, "metadata stdout", &metadata.stdout);
            assert_secret_absent(&secret, "metadata stderr", &metadata.stderr);

            let rustdoc_zip_log = scratch.join(format!("{channel}-rustdoc-zip-execution.json"));
            let rustdoc_zip = run_bazel(
                &native_bazel,
                channel,
                "build",
                &[
                    "--output_groups=rustdoc_zip",
                    "//bazel/evidence:evidence-doc",
                ],
                Some(&rustdoc_zip_log),
                &[],
            );
            assert!(
                rustdoc_zip.status.success(),
                "{channel} RustdocZip build failed: {}",
                String::from_utf8_lossy(&rustdoc_zip.stderr)
            );
            assert_secret_absent(&secret, "rustdoc zip stdout", &rustdoc_zip.stdout);
            assert_secret_absent(&secret, "rustdoc zip stderr", &rustdoc_zip.stderr);

            let build_log = scratch.join(format!("{channel}-build-execution.json"));
            let build = run_bazel(
                &native_bazel,
                channel,
                "build",
                &[build_target],
                Some(&build_log),
                &[],
            );
            assert!(
                build.status.success(),
                "{channel} rules_rust build failed: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            assert_secret_absent(&secret, "rules_rust build stdout", &build.stdout);
            assert_secret_absent(&secret, "rules_rust build stderr", &build.stderr);

            let test_log = scratch.join(format!("{channel}-test-execution.json"));
            let tests = run_bazel(
                &native_bazel,
                channel,
                "test",
                &[
                    "//bazel/evidence:evidence-test",
                    "//bazel/evidence:evidence-doctest",
                    "//bazel/evidence:evidence-rustfmt",
                ],
                Some(&test_log),
                &[],
            );
            assert!(
                tests.status.success(),
                "{channel} rules_rust tests failed: {}",
                String::from_utf8_lossy(&tests.stderr)
            );
            assert_secret_absent(&secret, "rules_rust test stdout", &tests.stdout);
            assert_secret_absent(&secret, "rules_rust test stderr", &tests.stderr);
            observed_execution.extend(normalized_execution_actions(
                channel,
                &[
                    metadata_log.as_path(),
                    rustdoc_zip_log.as_path(),
                    build_log.as_path(),
                    test_log.as_path(),
                ],
                &secret,
            ));
        }

        let expected_actions = hermeticity::GOVERNED_ACTION_KINDS
            .iter()
            .map(|kind| (*kind).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(observed_actions, expected_actions, "aquery action drift");
        assert_eq!(
            observed_execution.keys().cloned().collect::<BTreeSet<_>>(),
            expected_actions,
            "execution action drift"
        );
        let observation = hermeticity::ActionNetworkObservation {
            configured_targets: observed_actions.clone(),
            coverage_targets: hermeticity::CONFIGURED_COVERAGE_TARGETS
                .iter()
                .map(|target| (*target).to_owned())
                .collect(),
            aquery_actions: observed_actions,
            effective_strategies: observed_execution,
            fallback_strategies: BTreeSet::new(),
            cquery_targets: observed_labels,
            action_environment: BTreeSet::new(),
            test_environment: BTreeSet::new(),
        };
        let inventory =
            hermeticity::complete_action_network_inventory_from_observation(&observation);
        hermeticity::validate_observed_action_network(&inventory, &observation)
            .expect("runtime cquery/aquery/execution evidence must reconcile exactly");

        let environment = run_bazel(
            &native_bazel,
            "stable",
            "build",
            &["//bazel/evidence:environment-probe-action"],
            None,
            &[],
        );
        assert!(
            environment.status.success(),
            "action environment probe failed"
        );
        assert_secret_absent(&secret, "environment stdout", &environment.stdout);
        assert_secret_absent(&secret, "environment stderr", &environment.stderr);
        let environment_output = root.join("bazel-bin/bazel/evidence/environment-action.txt");
        let environment_contents =
            fs::read_to_string(environment_output).expect("read action environment evidence");
        assert!(!environment_contents.contains(&secret));
        assert!(!environment_contents.contains(SECRET_NAME));
        assert!(!environment_contents.contains('='));
        let mut lines = environment_contents.lines();
        assert_eq!(lines.next(), Some("environment-names-only-v1"));
        let environment_names = lines.map(str::to_owned).collect::<Vec<_>>();
        assert!(!environment_names.is_empty());
        assert!(
            environment_names.windows(2).all(|pair| pair[0] <= pair[1]),
            "environment names are not deterministic"
        );

        let launcher = run_bazel(
            &native_bazel,
            "stable",
            "build",
            &[
                "//bazel/evidence:inherited-fd-launcher",
                "//bazel/evidence:sandbox-crash-plant",
            ],
            None,
            &[],
        );
        assert!(
            launcher.status.success(),
            "inherited-fd launcher build failed"
        );
        let launcher_path = root.join("bazel-bin/bazel/evidence/inherited-fd-launcher");
        let linux_sandbox = patched_linux_sandbox(&scratch, &bazel);
        let policy = bazel
            .parent()
            .and_then(Path::parent)
            .expect("pinned Bazel store root")
            .join("share/d2b/bazel/seccomp-policy.json");
        for mode in ["socket", "ring", "ring-sqpoll", "ring-registered-socket"] {
            let mut command = Command::new(&launcher_path);
            command.current_dir(&root);
            configure_environment(&mut command, &secret);
            command.env("D2B_BAZEL_SECCOMP_POLICY", &policy);
            command.env("D2B_BAZEL_STRATEGY_LOCK", "d2b-bazel-sandbox-v1");
            command.arg(mode);
            command.arg(&linux_sandbox);
            command.args([
                "-W",
                scratch.to_str().expect("UTF-8 native sandbox root"),
                "-w",
                scratch.to_str().expect("UTF-8 native sandbox root"),
                "-M",
                "/nix/store",
                "-m",
                "/nix/store",
                "--",
                "/bin/true",
            ]);
            let output = command.output().expect("run inherited descriptor plant");
            assert!(
                !output.status.success(),
                "inherited descriptor plant passed"
            );
            let expected = if mode == "socket" {
                "D2B-BZLNET-INHERITED-SOCKET outcome=inherited-socket action=refused"
            } else {
                "D2B-BZLNET-INHERITED-RING outcome=inherited-io-uring action=refused"
            };
            let stderr = String::from_utf8_lossy(&output.stderr);
            let typed_records = stderr
                .lines()
                .filter(|line| line.starts_with("D2B-BZLNET-"))
                .collect::<Vec<_>>();
            assert_eq!(
                typed_records,
                vec![expected],
                "{mode} did not emit one exact descriptor record"
            );
            assert_secret_absent(&secret, "descriptor stdout", &output.stdout);
            assert_secret_absent(&secret, "descriptor stderr", &output.stderr);
        }

        for (index, stage) in [
            "before-ready",
            "after-ready",
            "after-executed",
            "fd-audit",
            "during-grace",
            "direct-descendant",
            "double-fork-descendant",
        ]
        .into_iter()
        .enumerate()
        {
            let output = run_crash_stage(&bazel, &root, &scratch, stage, index, &secret);
            assert_secret_absent(&secret, "cleanup stdout", &output.stdout);
            assert_secret_absent(&secret, "cleanup stderr", &output.stderr);
            if stage == "fd-audit" {
                assert!(output.status.success(), "fd audit plant failed");
            } else {
                assert!(!output.status.success(), "crash plant unexpectedly passed");
            }
        }

        let crash_plant = root.join("bazel-bin/bazel/evidence/sandbox-crash-plant");
        let beyond =
            run_beyond_ceiling_direct(&linux_sandbox, &crash_plant, &policy, &scratch, &secret);
        assert!(
            !beyond.status.success(),
            "quarantined action became successful"
        );
        assert_secret_absent(&secret, "quarantine stdout", &beyond.stdout);
        assert_secret_absent(&secret, "quarantine stderr", &beyond.stderr);
        let stderr = String::from_utf8_lossy(&beyond.stderr);
        let pending = "D2B-BZLEXEC-SANDBOX-PENDING-KERNEL-CLEANUP state=pending-kernel-cleanup quarantine=entered-and-held owner=original-monitor wait-owner=original-monitor result=failed reuse=denied action=no-success-no-reuse runbook=docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine";
        let release = "D2B-BZLEXEC-SANDBOX-CONSUMING-REAP-RELEASE cleanup=complete-after-quarantine quarantine=entered-and-released-after-consuming-reap owner=original-monitor wait=consuming result=failed";
        assert_eq!(
            stderr.lines().filter(|line| *line == pending).count(),
            1,
            "pending record drift: {stderr}"
        );
        assert_eq!(
            stderr.lines().filter(|line| *line == release).count(),
            1,
            "release record drift: {stderr}"
        );
        assert!(
            stderr.find(pending).expect("pending quarantine record")
                < stderr.find(release).expect("consuming-reap release"),
            "quarantine released before consuming reap"
        );
    }
}
