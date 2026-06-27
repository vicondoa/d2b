//! CLI-contract integration test, migrated from
//! tests/cli-rust-native-host-check.sh (the hardest CLI gate: it guards the
//! security-relevant `host check` host-posture probe battery).
//!
//! Two assertion families:
//!
//!   * CLI-local (hermetic): drive the real `d2b host check --read-only`
//!     binary with every probe pinned through `D2B_HOST_CHECK_FIXTURE`,
//!     covering the full exit-code lattice (0 pass / 1 warn / 2 fail /
//!     3 usage), strict deserialization of `--json` into
//!     `d2b_contracts::cli_output::HostCheckOutputV2` (the `deny_unknown_fields` DTOs make this
//!     the schema check the bash gate ran against
//!     docs/reference/cli-output/host-check.schema.json), the operator
//!     `internal-io` error envelope for forced probe failures, and `--human`
//!     severity grouping.
//!
//!   * Daemon-backed: spawn a real, KVM-free `d2bd serve --once` wired to
//!     the fixture bundle, talk to it through the bundled `d2bd
//!     test-client`, and assert the daemon `hostCheckResponse` shape
//!     (`summary.failures` / `summary.warnings` / `checks[]{name,status}`).
//!     These skip cleanly when the daemon-spawn harness
//!     (`D2B_TEST_D2BD_BIN`) is unavailable.
//!
//! Hermeticity note: the committed fixture-smoke `bundle.json` declares
//! ABSOLUTE artifact paths (`/etc/d2b/host.json`, ...). On a deployed
//! d2b host those files exist, so the bundle loader would resolve them and
//! leak the REAL host's posture into the test (the upstream bash gate is
//! non-hermetic this way). `build_hermetic_bundle_tree` copies the fixture
//! artifacts into a tempdir and rewrites `bundle.json` to relative paths so the
//! probe can only ever read the fixture artifacts.
//!
//! Requires D2B_FIXTURES (the fixture-smoke output dir), delivered by the
//! CLI-contract step in tests/tools/rust-workspace-checks.sh. When unset (e.g. the
//! plain `cargo test --workspace` pass) every test skips.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Map, Value, json};

use d2b_contracts::cli_output::{HostCheckOutputV2, HostCheckSeverityV2};

use common::TestPeer;

/// The fixture-smoke output dir, or `None` when D2B_FIXTURES is unset.
fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok()
}

/// Read the fixture-smoke `host.json` as a generic JSON value.
fn read_host_json(fixtures: &str) -> Value {
    let bytes = fs::read(format!("{fixtures}/host.json")).expect("read fixture host.json");
    serde_json::from_slice(&bytes).expect("decode fixture host.json")
}

/// Reproduce, in Rust, the `_d2b_host_check_sysctls_json` derivation from
/// tests/cli-rust-native-common.sh: the passing fixture's `sysctls` map MUST
/// mirror the bundle's declared sysctls so every fixture-backed probe returns
/// the documented value.
///
///   * each `environments[].ipv6Sysctls[]` entry contributes the five dotted
///     keys `<ifName>.{disable_ipv6,accept_ra,autoconf,addr_gen_mode,arp_ignore}`
///     with the declared (stringified) value;
///   * each `kernelModules[].sysctls[]` entry (`key=value`) contributes
///     `{key: value}`.
fn derive_sysctls(host: &Value) -> Map<String, Value> {
    let mut map = Map::new();
    if let Some(envs) = host.get("environments").and_then(Value::as_array) {
        for env in envs {
            let Some(list) = env.get("ipv6Sysctls").and_then(Value::as_array) else {
                continue;
            };
            for entry in list {
                let if_name = entry
                    .get("ifName")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                for (field, suffix) in [
                    ("disableIpv6", "disable_ipv6"),
                    ("acceptRa", "accept_ra"),
                    ("autoconf", "autoconf"),
                    ("addrGenMode", "addr_gen_mode"),
                    ("arpIgnore", "arp_ignore"),
                ] {
                    map.insert(
                        format!("{if_name}.{suffix}"),
                        Value::String(stringify(entry.get(field))),
                    );
                }
            }
        }
    }
    if let Some(modules) = host.get("kernelModules").and_then(Value::as_array) {
        for module in modules {
            let Some(sysctls) = module.get("sysctls").and_then(Value::as_array) else {
                continue;
            };
            for entry in sysctls {
                if let Some((key, value)) = entry.as_str().and_then(|raw| raw.split_once('=')) {
                    map.insert(key.to_owned(), Value::String(value.to_owned()));
                }
            }
        }
    }
    map
}

/// `jq`-style `tostring`: integers render without a decimal point.
fn stringify(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
        None => "null".to_owned(),
    }
}

/// The passing host-check fixture (mirrors
/// `d2b_write_host_check_fixture_pass`), parameterised by the bundle-derived
/// `sysctls` map.
fn pass_fixture_value(sysctls: Map<String, Value>) -> Value {
    json!({
        "kernelRelease": "6.8.12-d2b",
        "cgroupV2Present": true,
        "cpuVendor": "intel",
        "loadedModules": [
            "kvm", "kvm_intel", "tun", "vhost_net", "fuse", "nf_tables", "bridge",
            "br_netfilter", "i915", "amdgpu", "nvidia", "nvidia_modeset", "nvidia_uvm",
            "nvidia_drm", "usbip_host"
        ],
        "nftHasD2bTable": true,
        "firewalldActive": false,
        "ufwActive": false,
        "sysctls": Value::Object(sysctls),
    })
}

/// Serialize `value` to `dir/<name>` and return its path.
fn write_fixture(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        serde_json::to_vec_pretty(value).expect("serialize fixture"),
    )
    .expect("write fixture file");
    path
}

/// Copy the fixture-smoke artifacts into `dir` and rewrite `bundle.json` so its
/// `hostPath` / `processesPath` / `privilegesPath` are RELATIVE — see the
/// module-level hermeticity note. When `drift` is set, additionally rewrite
/// `closures/corp-vm.json` to break runner parity (mirrors
/// `d2b_cli_smoke_bundle_tree_runner_drift`).
fn build_hermetic_bundle_tree(fixtures: &str, dir: &Path, drift: bool) {
    fs::create_dir_all(dir.join("closures")).expect("mk closures dir");

    // fs::write (not fs::copy) so the destinations are writable (the
    // nix-store sources are 0444) and re-rewritable for the drift case.
    for name in [
        "host.json",
        "processes.json",
        "manifest.json",
        "privileges.json",
    ] {
        if let Ok(bytes) = fs::read(format!("{fixtures}/{name}")) {
            fs::write(dir.join(name), bytes).unwrap_or_else(|err| panic!("write {name}: {err}"));
        }
    }
    for entry in fs::read_dir(format!("{fixtures}/closures")).expect("read fixture closures dir") {
        let entry = entry.expect("closures dir entry");
        let bytes = fs::read(entry.path()).expect("read fixture closure");
        fs::write(dir.join("closures").join(entry.file_name()), bytes).expect("write closure");
    }

    let bundle_bytes =
        fs::read(format!("{fixtures}/bundle.json")).expect("read fixture bundle.json");
    let mut bundle: Value = serde_json::from_slice(&bundle_bytes).expect("decode bundle.json");
    let obj = bundle.as_object_mut().expect("bundle is a JSON object");
    obj.insert("hostPath".to_owned(), json!("host.json"));
    obj.insert("processesPath".to_owned(), json!("processes.json"));
    obj.insert("privilegesPath".to_owned(), json!("privileges.json"));
    fs::write(
        dir.join("bundle.json"),
        serde_json::to_vec_pretty(&bundle).expect("serialize bundle.json"),
    )
    .expect("write rewritten bundle.json");

    if drift {
        let path = dir.join("closures").join("corp-vm.json");
        let bytes = fs::read(&path).expect("read corp-vm closure");
        let mut closure: Value = serde_json::from_slice(&bytes).expect("decode corp-vm closure");
        let obj = closure.as_object_mut().expect("closure is a JSON object");
        obj.insert("runnerParityOk".to_owned(), json!(false));
        let drifted = obj
            .get("runnerParityPath")
            .and_then(Value::as_str)
            .map(|current| format!("{current}-drift"))
            .expect("corp-vm closure declares runnerParityPath");
        obj.insert("runnerParityPath".to_owned(), json!(drifted));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&closure).expect("serialize drift closure"),
        )
        .expect("write drift closure");
    }
}

/// A prepared, hermetic host-check scenario: a tempdir holding the rewritten
/// bundle tree plus the chosen fixture file.
struct Scenario {
    _tmp: tempfile::TempDir,
    tree: PathBuf,
    fixture: PathBuf,
}

impl Scenario {
    /// Build a scenario whose fixture is the passing fixture optionally mutated
    /// by `mutate`, against a normal or runner-drift bundle tree.
    fn new(
        fixtures: &str,
        drift: bool,
        fixture_name: &str,
        mutate: impl FnOnce(&mut Value),
    ) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tree = tmp.path().join("tree");
        build_hermetic_bundle_tree(fixtures, &tree, drift);

        let host = read_host_json(fixtures);
        let mut value = pass_fixture_value(derive_sysctls(&host));
        mutate(&mut value);
        let fixture = write_fixture(tmp.path(), fixture_name, &value);

        Scenario {
            _tmp: tmp,
            tree,
            fixture,
        }
    }

    /// Run `d2b host check --read-only <args>` against this scenario.
    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_d2b"));
        cmd.args(["host", "check", "--read-only"])
            .args(args)
            .env("D2B_MANIFEST_PATH", self.tree.join("manifest.json"))
            .env("D2B_BUNDLE_PATH", self.tree.join("bundle.json"))
            .env("D2B_HOST_CHECK_FIXTURE", &self.fixture);
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        cmd.output().expect("spawn d2b host check")
    }
}

/// Strictly deserialize a `--json` stdout into `HostCheckOutputV2`. Because the
/// V2 DTOs are `deny_unknown_fields`, a successful parse is equivalent to
/// validating against docs/reference/cli-output/host-check.schema.json.
fn parse_output(out: &Output) -> HostCheckOutputV2 {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "host check --json did not match HostCheckOutputV2: {err}\nstdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Parse the stderr operator-error envelope emitted for forced probe failures.
fn parse_error_envelope(out: &Output) -> Value {
    serde_json::from_slice(&out.stderr).unwrap_or_else(|err| {
        panic!(
            "host check error envelope was not valid JSON: {err}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

// --- CLI-local cases -------------------------------------------------------

#[test]
fn host_check_pass_fixture_exits_zero_and_matches_schema() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-pass.json", |_| {});
    let out = scenario.run(&["--json"], &[]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "pass fixture exits 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = parse_output(&out);
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.summary.fail, 0, "pass fixture has no failures");
    assert_eq!(output.summary.warn, 0, "pass fixture has no warnings");
    assert!(output.summary.pass > 0, "pass fixture records passes");
}

#[test]
fn host_check_built_in_kernel_modules_pass() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    // Drop kvm_intel from loadedModules and mark it built-in instead.
    let scenario = Scenario::new(&fixtures, false, "host-builtin.json", |value| {
        let modules = value["loadedModules"]
            .as_array()
            .expect("loadedModules array")
            .iter()
            .filter(|m| m.as_str() != Some("kvm_intel"))
            .cloned()
            .collect::<Vec<_>>();
        value["loadedModules"] = Value::Array(modules);
        value["builtInModules"] = json!(["kvm_intel"]);
    });
    let out = scenario.run(&["--json"], &[]);

    assert_eq!(out.status.code(), Some(0), "built-in modules still exit 0");
    let output = parse_output(&out);
    let finding = output
        .findings
        .iter()
        .find(|f| f.id == "kernel-module:kvm_intel")
        .expect("kernel-module:kvm_intel finding present");
    assert_eq!(finding.severity, HostCheckSeverityV2::Pass);
    assert!(
        finding.message.contains("built into the running kernel"),
        "built-in module message regressed: {}",
        finding.message
    );
}

#[test]
fn host_check_ufw_active_warns() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-pass.json", |_| {});
    let out = scenario.run(&["--json"], &[("D2B_TEST_UFW_ACTIVE", "1")]);

    assert_eq!(out.status.code(), Some(1), "ufw-active warning exits 1");
    let output = parse_output(&out);
    let finding = output
        .findings
        .iter()
        .find(|f| f.id == "firewall-coexistence")
        .expect("firewall-coexistence finding present");
    assert_eq!(finding.severity, HostCheckSeverityV2::Warn);
    assert_eq!(finding.message, "firewalld_active=false ufw_active=true");
}

#[test]
fn host_check_systemctl_unavailable_warns() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-pass.json", |_| {});
    let out = scenario.run(&["--json"], &[("D2B_TEST_SYSTEMCTL_UNAVAILABLE", "1")]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "systemctl-unavailable warns (exit 1) instead of claiming inactive firewalls"
    );
    let output = parse_output(&out);
    let finding = output
        .findings
        .iter()
        .find(|f| f.id == "firewall-coexistence")
        .expect("firewall-coexistence finding present");
    assert_eq!(finding.severity, HostCheckSeverityV2::Warn);
    assert!(
        finding.message.contains("could not be fully determined"),
        "systemctl-unavailable message regressed: {}",
        finding.message
    );
    assert_eq!(
        finding.detail.as_deref(),
        Some("systemctl probe unavailable on this host")
    );
}

#[test]
fn host_check_runner_parity_drift_warns_without_strict() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, true, "host-pass.json", |_| {});
    let out = scenario.run(&["--json"], &[]);

    assert_eq!(out.status.code(), Some(1), "runner drift warns (exit 1)");
    let output = parse_output(&out);
    assert!(output.summary.warn > 0, "runner drift records a warning");
    assert_eq!(
        output.summary.fail, 0,
        "runner drift is advisory without --strict"
    );
    assert!(
        output
            .findings
            .iter()
            .any(|f| f.id == "runner-parity" && f.severity == HostCheckSeverityV2::Warn),
        "runner-parity warn finding present"
    );
}

#[test]
fn host_check_runner_parity_drift_fails_under_strict() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, true, "host-pass.json", |_| {});
    let out = scenario.run(&["--strict", "--json"], &[]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "runner drift is fatal under --strict"
    );
    let output = parse_output(&out);
    assert!(
        output.summary.fail > 0,
        "strict runner drift records a failure"
    );
    assert!(
        output
            .findings
            .iter()
            .any(|f| f.id == "runner-parity" && f.severity == HostCheckSeverityV2::Fail),
        "runner-parity fail finding present under --strict"
    );
}

#[test]
fn host_check_hard_failure_exits_two() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    // An unsupported (too-old) kernel is a hard failure.
    let scenario = Scenario::new(&fixtures, false, "host-fail.json", |value| {
        value["kernelRelease"] = json!("6.5.0-d2b");
    });
    let out = scenario.run(&["--json"], &[]);

    assert_eq!(out.status.code(), Some(2), "hard failure exits 2");
    let output = parse_output(&out);
    assert!(output.summary.fail > 0, "hard failure records a failure");
}

#[test]
fn host_check_modules_probe_error_envelope() {
    assert_probe_error_envelope(
        "host-modules-error.json",
        "forced /proc/modules read failure",
        |value| {
            value["loadedModules"] = Value::Null;
            value["loadedModulesError"] = json!("forced /proc/modules read failure");
        },
    );
}

#[test]
fn host_check_nft_probe_error_envelope() {
    assert_probe_error_envelope("host-nft-error.json", "forced nft probe failure", |value| {
        value["nftHasD2bTable"] = Value::Null;
        value["nftError"] = json!("forced nft probe failure");
    });
}

#[test]
fn host_check_ufw_probe_error_envelope() {
    assert_probe_error_envelope("host-ufw-error.json", "forced ufw probe failure", |value| {
        value["ufwActive"] = Value::Null;
        value["ufwError"] = json!("forced ufw probe failure");
    });
}

/// Shared body for the three forced-probe-failure cases: the probe error must
/// surface as exit 1 with the `internal-io` operator envelope on stderr
/// (owningCommand "host check", code 50, message carrying the forced reason).
fn assert_probe_error_envelope(
    fixture_name: &str,
    expected_reason: &str,
    mutate: impl FnOnce(&mut Value),
) {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, fixture_name, mutate);
    let out = scenario.run(&["--json"], &[]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "forced probe failure surfaces as an internal error (exit 1)"
    );
    assert!(
        out.stdout.is_empty(),
        "a forced probe failure must not print a report body"
    );
    let envelope = parse_error_envelope(&out);
    assert_eq!(envelope["kind"], "internal-io");
    assert_eq!(envelope["owningCommand"], "host check");
    assert_eq!(envelope["code"], 50);
    let message = envelope["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(expected_reason),
        "error envelope message should carry the forced reason {expected_reason:?}; got: {message}"
    );
}

#[test]
fn host_check_usage_error_exits_three() {
    if fixtures_dir().is_none() {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    }
    // clap usage errors (unknown flags) are exit 3 regardless of fixtures.
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["host", "check", "--bogus"])
        .output()
        .expect("spawn d2b host check --bogus");
    assert_eq!(out.status.code(), Some(3), "usage errors exit 3");
}

#[test]
fn host_check_human_groups_findings_by_severity() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-pass.json", |_| {});
    let out = scenario.run(&["--human"], &[]);

    assert_eq!(out.status.code(), Some(0), "pass fixture --human exits 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("summary: pass="),
        "--human prints the severity summary line; got:\n{stdout}"
    );
    assert!(
        stdout.contains("PASS"),
        "--human groups findings under a PASS header; got:\n{stdout}"
    );
}

// --- Daemon-backed cases ---------------------------------------------------

#[test]
fn daemon_host_check_pass_runs_full_probe_battery() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-pass.json", |_| {});
    let Some(daemon) =
        common::spawn_d2bd_host_check(&scenario.tree, &scenario.fixture, &TestPeer::launcher())
    else {
        eprintln!("SKIP: D2B_TEST_D2BD_BIN unset (daemon-spawn harness unavailable)");
        return;
    };

    let response = common::daemon_host_check_response(&daemon.socket_path, false);
    let _ = daemon.wait();

    assert_eq!(response["summary"]["failures"].as_u64(), Some(0));
    assert_eq!(response["summary"]["warnings"].as_u64(), Some(0));
    for name in [
        "kernel-module:kvm",
        "nftables-table",
        "firewall-coexistence",
        "runner-parity",
    ] {
        assert!(
            has_check(&response, name),
            "daemon host-check battery missing check {name}; checks:\n{}",
            check_names(&response).join(", ")
        );
    }
}

#[test]
fn daemon_host_check_fails_closed_on_old_kernel_and_missing_cgroup_v2() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, false, "host-daemon-fail.json", |value| {
        value["kernelRelease"] = json!("6.5.0-d2b");
        value["cgroupV2Present"] = json!(false);
    });
    let Some(daemon) =
        common::spawn_d2bd_host_check(&scenario.tree, &scenario.fixture, &TestPeer::launcher())
    else {
        eprintln!("SKIP: D2B_TEST_D2BD_BIN unset (daemon-spawn harness unavailable)");
        return;
    };

    let response = common::daemon_host_check_response(&daemon.socket_path, false);
    let _ = daemon.wait();

    assert!(
        response["summary"]["failures"].as_u64().unwrap_or(0) >= 2,
        "old kernel + missing cgroup-v2 yield >= 2 failures; summary:\n{}",
        response["summary"]
    );
    assert_eq!(check_status(&response, "kernel-version"), Some("fail"));
    assert_eq!(check_status(&response, "cgroup-v2"), Some("fail"));
}

#[test]
fn daemon_host_check_strict_runner_parity_fails() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let scenario = Scenario::new(&fixtures, true, "host-pass.json", |_| {});
    let Some(daemon) =
        common::spawn_d2bd_host_check(&scenario.tree, &scenario.fixture, &TestPeer::launcher())
    else {
        eprintln!("SKIP: D2B_TEST_D2BD_BIN unset (daemon-spawn harness unavailable)");
        return;
    };

    let response = common::daemon_host_check_response(&daemon.socket_path, true);
    let _ = daemon.wait();

    assert!(
        response["summary"]["failures"].as_u64().unwrap_or(0) > 0,
        "strict runner-parity drift yields a failure; summary:\n{}",
        response["summary"]
    );
    assert_eq!(check_status(&response, "runner-parity"), Some("fail"));
}

/// Status string of the first `checks[]` entry named `name`, if any.
fn check_status<'a>(response: &'a Value, name: &str) -> Option<&'a str> {
    response["checks"]
        .as_array()?
        .iter()
        .find(|check| check.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|check| check.get("status").and_then(Value::as_str))
}

/// Whether any `checks[]` entry is named `name`.
fn has_check(response: &Value, name: &str) -> bool {
    response["checks"]
        .as_array()
        .map(|checks| {
            checks
                .iter()
                .any(|check| check.get("name").and_then(Value::as_str) == Some(name))
        })
        .unwrap_or(false)
}

/// All `checks[]` names, for diagnostics on assertion failure.
fn check_names(response: &Value) -> Vec<String> {
    response["checks"]
        .as_array()
        .map(|checks| {
            checks
                .iter()
                .filter_map(|check| check.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
