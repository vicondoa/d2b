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
        "cqueryTargets": ["//bazel/evidence:action-probes"],
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
        "cqueryTargets": ["//bazel/evidence:action-probes"],
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
    for code in [
        "D2B-BZLNET-PREFLIGHT",
        "D2B-BZLNET-INHERITED-SOCKET",
        "D2B-BZLNET-INHERITED-RING",
        "D2BFailInheritedDescriptorPreflight",
    ] {
        assert!(patch.contains(code), "missing typed descriptor code {code}");
    }
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
    ] {
        assert!(
            wrapper.contains(forbidden),
            "wrapper does not reject {forbidden}"
        );
    }
    assert!(!bazelrc.contains("--action_env="));
    assert!(!bazelrc.contains("--test_env="));
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
    use std::{
        env,
        fs::{self, File, OpenOptions},
        io::{self, Read, Write},
        os::unix::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        process::{Command, Output, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

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

    fn configure_environment(command: &mut Command) {
        command.env_clear();
        for name in ["PATH", "JAVA_HOME", "TMPDIR"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
    }

    fn bazel_arguments(scratch: &Path, label: &str) -> Vec<String> {
        vec![
            "--batch".to_owned(),
            format!("--output_user_root={}", scratch.join("user").display()),
            format!("--output_base={}", scratch.join("base").display()),
            "build".to_owned(),
            "--noenable_bzlmod".to_owned(),
            "--enable_workspace".to_owned(),
            format!("--sandbox_writable_path={}", scratch.display()),
            format!(
                "--execution_log_json_file={}",
                scratch.join("execution-log.json").display()
            ),
            label.to_owned(),
        ]
    }

    fn run_bazel(
        bazel: &Path,
        root: &Path,
        scratch: &Path,
        label: &str,
        defines: &[(&str, &str)],
    ) -> Output {
        let mut command = Command::new(bazel);
        command.current_dir(root);
        configure_environment(&mut command);
        let mut arguments = bazel_arguments(scratch, label);
        let label = arguments.pop().expect("Bazel label");
        for (name, value) in defines {
            arguments.push(format!("--define={name}={value}"));
        }
        command.args(arguments);
        command.arg(label);
        command.output().expect("run pinned Bazel action")
    }

    fn run_cquery(bazel: &Path, root: &Path, scratch: &Path, expression: &str) -> Output {
        let mut command = Command::new(bazel);
        command.current_dir(root);
        configure_environment(&mut command);
        command.args([
            "--batch".to_owned(),
            format!("--output_user_root={}", scratch.join("user").display()),
            format!("--output_base={}", scratch.join("base").display()),
            "cquery".to_owned(),
            "--noenable_bzlmod".to_owned(),
            "--enable_workspace".to_owned(),
            "--output=label".to_owned(),
            expression.to_owned(),
        ]);
        command.output().expect("run pinned Bazel cquery")
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
        let install_root = scratch.join("user/install");
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
        configure_environment(&mut command);
        command.args(bazel_arguments(
            scratch,
            "//bazel/evidence:crash-plant-action",
        ));
        for (name, value) in defines {
            command.arg(format!("--define={name}={value}"));
        }
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

    #[test]
    #[ignore = "requires a native patched Bazel and Linux namespace support"]
    fn native_bazel_runs_network_descriptor_and_cleanup_plants() {
        let root = repository_root();
        let scratch = scratch_root(&root);
        let bazel = bazel_executable();

        let network = run_bazel(
            &bazel,
            &root,
            &scratch,
            "//bazel/evidence:network-plant-action",
            &[],
        );
        assert!(network.status.success(), "network denial plant failed");

        let probes = run_bazel(
            &bazel,
            &root,
            &scratch,
            "//bazel/evidence:action-probes",
            &[],
        );
        assert!(probes.status.success(), "effective strategy probes failed");
        let query = run_cquery(
            &bazel,
            &root,
            &scratch,
            "deps(//bazel/evidence:action-probes, 1)",
        );
        assert!(query.status.success(), "configured-target cquery failed");
        let query_output = String::from_utf8_lossy(&query.stdout);
        for label in [
            "//bazel/evidence:stable-rustc",
            "//bazel/evidence:nightly-rustc",
        ] {
            assert!(
                query_output.contains(label),
                "cquery omitted governed target"
            );
        }
        let execution_log =
            fs::read_to_string(scratch.join("execution-log.json")).expect("read aquery evidence");
        assert!(
            execution_log.contains("\"runner\": \"linux-sandbox\""),
            "effective execution did not use linux-sandbox"
        );
        for mnemonic in ["StableRustc", "NightlyRustc", "StableTest", "NightlyTest"] {
            assert!(
                execution_log.contains(&format!("\"mnemonic\": \"{mnemonic}\"")),
                "aquery execution evidence omitted action"
            );
        }

        let environment = run_bazel(
            &bazel,
            &root,
            &scratch,
            "//bazel/evidence:environment-probe-action",
            &[],
        );
        assert!(
            environment.status.success(),
            "action environment probe failed"
        );
        let environment_output = root.join("bazel-bin/bazel/evidence/environment-probe-action.txt");
        let environment_names = fs::read_to_string(environment_output)
            .expect("read action environment evidence")
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_owned()))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            environment_names.iter().all(|name| {
                matches!(
                    name.as_str(),
                    "BASH_ENV"
                        | "PATH"
                        | "PWD"
                        | "TMPDIR"
                        | "ZERO_AR_DATE"
                        | "__ETC_PROFILE_DONE"
                        | "__ETC_PROFILE_SOURCED"
                )
            }),
            "action environment contains an ambient variable"
        );

        let launcher = run_bazel(
            &bazel,
            &root,
            &scratch,
            "//bazel/evidence:inherited-fd-launcher",
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
        for mode in ["socket", "ring"] {
            let mut command = Command::new(&launcher_path);
            command.current_dir(&root);
            configure_environment(&mut command);
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
                "/run/current-system/sw/bin/true",
            ]);
            let output = command.output().expect("run inherited descriptor plant");
            assert!(
                !output.status.success(),
                "inherited descriptor plant passed"
            );
            let expected = if mode == "socket" {
                "D2B-BZLNET-INHERITED-SOCKET"
            } else {
                "D2B-BZLNET-INHERITED-RING"
            };
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(expected),
                "missing inherited descriptor code"
            );
        }

        for (index, stage) in [
            "before-ready",
            "after-ready",
            "after-executed",
            "fd-audit",
            "during-grace",
            "direct-descendant",
            "double-fork-descendant",
            "beyond-ceiling",
        ]
        .into_iter()
        .enumerate()
        {
            let output = run_crash_stage(&bazel, &root, &scratch, stage, index);
            if stage == "fd-audit" {
                assert!(output.status.success(), "fd audit plant failed");
            } else {
                assert!(!output.status.success(), "crash plant unexpectedly passed");
            }
        }
    }
}
