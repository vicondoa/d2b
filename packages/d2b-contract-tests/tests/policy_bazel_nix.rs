//! Fixture-independent policy for the Spec 003 Nix and artifact boundary.
//!
//! These tests deliberately inspect the repository contract and one reviewed
//! C test tool. They do not use Rust FFI, Rust process-control primitives, or
//! rendered fixture paths.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use serde_json::Value;

const HOST_BROKER: &str = "nixos-modules/host-broker.nix";
const FLAKE: &str = "flake.nix";
const BASELINES: &str = "tests/golden/bazel-rust-artifact-baselines.json";
const TOOLCHAIN_GOLDEN: &str = "tests/golden/bazel-toolchain.json";
const SUPERVISOR_GOLDEN: &str = "tests/golden/bazel-exec-supervisor.json";
const SUPERVISOR_SOURCE: &str = "tests/tools/d2b-bazel-exec-supervisor/supervisor.c";
const NATIVE_MANIFEST: &str = "tests/golden/native-policy-check-manifest.json";

const WL_PROXY_HASH: &str = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";

fn contains_all(text: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| text.contains(needle))
}

fn count(text: &str, needle: &str) -> usize {
    text.match_indices(needle).count()
}

fn json(path: &str) -> Value {
    serde_json::from_str(&read_repo_file(path))
        .unwrap_or_else(|error| panic!("{path} must be valid JSON: {error}"))
}

fn sha256_file(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("sha256sum must be available for artifact authorization tests");
    assert!(
        output.status.success(),
        "sha256sum must hash the review file"
    );
    String::from_utf8(output.stdout)
        .expect("sha256sum output must be UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum output must contain a digest")
        .to_owned()
}

fn strings(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{pointer} must be an array"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("{pointer} contains a non-string"))
                .to_owned()
        })
        .collect()
}

#[test]
fn dedicated_derivations_use_the_root_source_lock_and_exact_selectors() {
    let host_broker = read_repo_file(HOST_BROKER);
    let flake = read_repo_file(FLAKE);

    assert!(contains_all(
        &host_broker,
        &[
            "src = packagesSrc;",
            "lockFile = ../packages/Cargo.lock;",
            "outputHashes.\"wl-proxy-0.1.2\"",
            "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=",
            "\"--package\"",
            "\"d2b-priv-broker\"",
            "\"--bin\"",
            "\"d2b-priv-broker\"",
            "\"--no-default-features\"",
        ]
    ));
    assert!(!host_broker.contains("sourceRoot = \"source/d2b-priv-broker\""));
    assert!(!host_broker.contains("../packages/d2b-priv-broker/Cargo.lock"));
    assert_eq!(count(&host_broker, WL_PROXY_HASH), 1);

    assert!(contains_all(
        &flake,
        &[
            "src = rustPackagesSrc;",
            "sourceRoot = \"d2b-rust-src/packages\";",
            "lockFile = ./packages/Cargo.lock;",
            "\"--package\"",
            "\"d2b-guest-shell-runner\"",
            "\"--bin\"",
            "\"d2b-guest-shell-runner\"",
            "\"--no-default-features\"",
            "\"--features\"",
            "\"real-libshpool\"",
        ]
    ));
    assert!(!flake.contains("./packages/d2b-guest-shell-runner/Cargo.lock"));
    assert!(count(&flake, WL_PROXY_HASH) >= 2);
}

#[test]
fn generic_nix_contexts_exclude_broker_and_guest_exactly() {
    let flake = read_repo_file(FLAKE);

    let build = flake
        .split_once("rust-build =")
        .and_then(|(_, rest)| rest.split_once("rust-tests ="))
        .map(|(section, _)| section)
        .expect("rust-build and rust-tests sections must exist");
    let tests = flake
        .split_once("rust-tests =")
        .and_then(|(_, rest)| rest.split_once("rust-clippy ="))
        .map(|(section, _)| section)
        .expect("rust-tests and rust-clippy sections must exist");
    let clippy = flake
        .split_once("rust-clippy =")
        .and_then(|(_, rest)| rest.split_once("guest-static-elf ="))
        .map(|(section, _)| section)
        .expect("rust-clippy and guest-static-elf sections must exist");

    for section in [build, tests, clippy] {
        assert!(
            count(section, "d2b-priv-broker") >= 1,
            "generic section must exclude broker"
        );
        assert!(
            count(section, "d2b-guest-shell-runner") >= 1,
            "generic section must exclude guest"
        );
    }
    assert!(tests.contains("d2b-contract-tests"));
}

#[test]
fn all_four_selected_contexts_and_six_native_checks_are_bound() {
    let flake = read_repo_file(FLAKE);
    let package_policy = read_repo_file("packages/xtask/src/package_policy.rs");
    let test_rust = read_repo_file("tests/test-rust.sh");
    let manifest = json(NATIVE_MANIFEST);

    for context in manifest["contexts"].as_array().expect("native contexts") {
        let path = context["policyInput"]
            .as_str()
            .expect("native context policy input");
        assert!(
            package_policy.contains(context["system"].as_str().unwrap()),
            "selected system missing from generator contract: {path}"
        );
    }
    assert!(contains_all(
        &package_policy,
        &[
            "BrokerProduction",
            "GuestProduction",
            "x86_64-linux",
            "aarch64-linux",
            "gnu",
            "musl",
            "real-libshpool",
        ]
    ));

    for check in manifest["nativeChecks"]
        .as_array()
        .expect("native checks")
        .iter()
        .map(|check| check.as_str().expect("native check name"))
    {
        assert!(flake.contains(check), "native flake check missing: {check}");
    }
    assert!(test_rust.contains("policy_metadata_path"));
    assert!(test_rust.contains("policy_lock_path"));
    assert!(test_rust.contains(NATIVE_MANIFEST));
}

#[test]
fn guest_elf_and_broker_linkage_contract_is_closed_without_store_diagnostics() {
    let flake = read_repo_file(FLAKE);
    assert!(contains_all(
        &flake,
        &[
            "guest-static-elf",
            "broker-host-artifact-contract",
            "ET_DYN",
            "EM_X86_64",
            "EM_AARCH64",
            "PT_INTERP",
            "DT_NEEDED",
            "readelf",
            "staticRustFlags =",
            "-C relocation-model=pie -C link-self-contained=yes",
            "-C linker=${pkgs.llvmPackages.clang-unwrapped}/bin/clang",
            "-C link-arg=-Wl,-pie",
            "closure",
            "selectedPolicyDigest",
            "sizeGrowthAuthorization",
        ]
    ));
    assert!(!flake.contains("/nix/store/"));
    assert!(!flake.contains("echo \"$bin:"));
    assert!(!flake.contains("printf '%s\\n' \"$bin"));

    if repo_path_exists(BASELINES) {
        let baselines = json(BASELINES);
        let rows = baselines["rows"]
            .as_array()
            .expect("artifact baseline rows must be an array");
        assert_eq!(rows.len(), 4);
        let pairs = rows
            .iter()
            .map(|row| {
                format!(
                    "{}/{}",
                    row["system"].as_str().unwrap_or_default(),
                    row["artifact"].as_str().unwrap_or_default()
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(pairs.len(), 4);
        assert_eq!(
            pairs,
            BTreeSet::from([
                "x86_64-linux/broker-host-artifact-contract".to_owned(),
                "x86_64-linux/guest-static-elf".to_owned(),
                "aarch64-linux/broker-host-artifact-contract".to_owned(),
                "aarch64-linux/guest-static-elf".to_owned(),
            ])
        );
        assert!(!baselines.to_string().contains("/nix/store/"));
        for row in rows {
            assert!(row["system"].is_string());
            assert!(row["artifact"].is_string());
            assert!(row["binaryBytes"].is_u64());
            assert_eq!(row["binarySha256"].as_str().unwrap_or_default().len(), 64);
            assert!(row["elfType"].is_string());
            assert!(row["elfMachine"].is_string());
            assert!(row["closureCount"].is_u64());
            assert_eq!(row["closureSha256"].as_str().unwrap().len(), 64);
            assert_eq!(row["selectedPolicyDigest"].as_str().unwrap().len(), 64);
            assert!(row["measurementCommand"].is_string());
            assert!(row["candidateCommit"].is_string());
            let expected_machine = match row["system"].as_str() {
                Some("x86_64-linux") => "EM_X86_64",
                Some("aarch64-linux") => "EM_AARCH64",
                other => panic!("unexpected artifact system {other:?}"),
            };
            assert_eq!(row["elfMachine"], expected_machine);
            assert!(row.get("rowAllowance").is_none());
            assert!(row.get("sizeAllowance").is_none());
            assert!(row.get("sizeGrowthAuthorization").is_some());
            if row["artifact"] == "broker-host-artifact-contract" {
                assert_eq!(row["elfType"], "ET_DYN");
                assert!(row["interpreter"].is_string());
                let expected_interpreter = match row["system"].as_str() {
                    Some("x86_64-linux") => "ld-linux-x86-64.so.2",
                    Some("aarch64-linux") => "ld-linux-aarch64.so.1",
                    other => panic!("unexpected artifact system {other:?}"),
                };
                assert_eq!(row["interpreter"], expected_interpreter);
                assert!(row["needed"].is_array());
                let needed = row["needed"].as_array().unwrap();
                let mut sorted = needed.clone();
                sorted.sort_by_key(|entry| entry.as_str().unwrap_or_default().to_owned());
                assert_eq!(*needed, sorted);
            } else {
                assert_eq!(row["elfType"], "ET_DYN");
                assert!(row.get("interpreter").is_none() || row["interpreter"].is_null());
                assert_eq!(row["needed"], serde_json::json!([]));
            }
        }
    }
}

#[test]
fn size_growth_authorization_has_only_the_closed_positive_delta_source() {
    let baseline = SizeRow {
        system: "x86_64-linux",
        artifact: "guest-static-elf",
        prior_bytes: 100,
        candidate_digest: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        rationale_path: "LICENSE",
        review_digest: "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
    };
    let approved = AuthorizationSpec::for_row(&baseline);
    assert!(size_authorization_valid(
        &baseline,
        100,
        baseline.candidate_digest,
        None
    ));
    assert!(size_authorization_valid(
        &baseline,
        107,
        baseline.candidate_digest,
        Some(&authorization(approved))
    ));

    let negative = [
        size_authorization_valid(&baseline, 107, baseline.candidate_digest, None),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(approved).with_decision("denied")),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                candidate: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                review: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(
                &authorization(AuthorizationSpec {
                    system: "aarch64-linux",
                    ..approved
                })
                .with_extra(),
            ),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                artifact: "broker-host-artifact-contract",
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                prior: 99,
                delta: 8,
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                new: 108,
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                delta: 8,
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                rationale: "/absolute/review.md",
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            107,
            baseline.candidate_digest,
            Some(&authorization(AuthorizationSpec {
                rationale: "reviews/missing.md",
                ..approved
            })),
        ),
        size_authorization_valid(
            &baseline,
            108,
            baseline.candidate_digest,
            Some(&authorization(approved)),
        ),
    ];
    assert!(negative.iter().all(|valid| !valid));
}

#[derive(Clone, Copy)]
struct SizeRow {
    system: &'static str,
    artifact: &'static str,
    prior_bytes: u64,
    candidate_digest: &'static str,
    rationale_path: &'static str,
    review_digest: &'static str,
}

struct Authorization {
    value: Value,
}

#[derive(Clone, Copy)]
struct AuthorizationSpec<'a> {
    system: &'a str,
    artifact: &'a str,
    prior: u64,
    new: u64,
    delta: u64,
    rationale: &'a str,
    candidate: &'a str,
    review: &'a str,
}

impl<'a> AuthorizationSpec<'a> {
    fn for_row(row: &'a SizeRow) -> Self {
        Self {
            system: row.system,
            artifact: row.artifact,
            prior: row.prior_bytes,
            new: 107,
            delta: 7,
            rationale: row.rationale_path,
            candidate: row.candidate_digest,
            review: row.review_digest,
        }
    }
}

impl Authorization {
    fn with_decision(mut self, decision: &str) -> Self {
        self.value["decision"] = Value::String(decision.to_owned());
        self
    }

    fn with_extra(mut self) -> Self {
        self.value["allowanceSource"] = Value::String("row".to_owned());
        self
    }
}

fn authorization(spec: AuthorizationSpec<'_>) -> Authorization {
    Authorization {
        value: serde_json::json!({
            "system": spec.system,
            "artifact": spec.artifact,
            "priorBinaryBytes": spec.prior,
            "newBinaryBytes": spec.new,
            "deltaBytes": spec.delta,
            "rationalePath": spec.rationale,
            "candidateContentSha256": spec.candidate,
            "reviewRecordSha256": spec.review,
            "decision": "approved",
        }),
    }
}

fn size_authorization_valid(
    row: &SizeRow,
    realized_bytes: u64,
    realized_digest: &str,
    authorization: Option<&Authorization>,
) -> bool {
    let Some(authorization) = authorization else {
        return realized_bytes <= row.prior_bytes;
    };
    let value = &authorization.value;
    let Some(object) = value.as_object() else {
        return false;
    };
    let fields = [
        "system",
        "artifact",
        "priorBinaryBytes",
        "newBinaryBytes",
        "deltaBytes",
        "rationalePath",
        "candidateContentSha256",
        "reviewRecordSha256",
        "decision",
    ];
    if object.len() != fields.len() || !fields.iter().all(|field| object.contains_key(*field)) {
        return false;
    }
    let rationale_path = value["rationalePath"].as_str();
    let rationale_is_repository_relative = rationale_path.is_some_and(|path| {
        let relative = Path::new(path);
        let repository = repo_root();
        relative.is_relative()
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
            && fs::canonicalize(repository.join(relative))
                .is_ok_and(|resolved| resolved.starts_with(&repository))
    });
    value["system"] == row.system
        && value["artifact"] == row.artifact
        && value["priorBinaryBytes"] == row.prior_bytes
        && value["newBinaryBytes"] == realized_bytes
        && value["deltaBytes"].as_u64() == realized_bytes.checked_sub(row.prior_bytes)
        && realized_bytes > row.prior_bytes
        && rationale_is_repository_relative
        && value["candidateContentSha256"]
            .as_str()
            .is_some_and(|digest| digest == realized_digest && is_hex(digest, 64))
        && value["reviewRecordSha256"].as_str().is_some_and(|digest| {
            digest == sha256_file(&repo_root().join(rationale_path.expect("path checked")))
        })
        && value["decision"] == "approved"
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[test]
fn artifact_contract_binds_real_content_and_structural_policy_inputs() {
    let flake = read_repo_file(FLAKE);
    for needle in [
        "builtins.fromJSON",
        "builtins.fromTOML",
        "policyInputCorpusGate",
        "artifactBaselineGate",
        "actual_binary_sha",
        "sha256sum \"$binary\"",
        "authorizationCandidate",
        "authorizationReviewDigest",
        "expected-e-machine",
        "actual_e_machine",
        "builtins.hashFile \"sha256\"",
        "binarySha256",
        "closure_count",
        "closure_sha",
        "policy_sha",
    ] {
        assert!(flake.contains(needle), "flake lost binding {needle}");
    }
    assert!(!flake.contains("grep -Fq '\"system\":"));
    assert!(!flake.contains("grep -Fq '\"target\":"));
    assert!(!flake.contains("phase-valid"));
}

#[test]
fn size_authorization_cannot_replay_one_review_as_two_authorities() {
    let baseline = SizeRow {
        system: "x86_64-linux",
        artifact: "guest-static-elf",
        prior_bytes: 100,
        candidate_digest: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        rationale_path: "LICENSE",
        review_digest: "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
    };
    let approved = authorization(AuthorizationSpec::for_row(&baseline));
    let mut authorities = BTreeSet::new();
    let records = [approved.value.clone(), approved.value];
    for value in &records {
        authorities.insert(serde_json::to_string(value).expect("authorization JSON"));
    }
    assert_eq!(
        authorities.len(),
        1,
        "replayed authority must not become two independent records"
    );
    assert_ne!(authorities.len(), records.len());
}

#[test]
fn prep_owned_bazel_and_supervisor_identity_records_are_bound_unchanged() {
    let toolchain = json(TOOLCHAIN_GOLDEN);
    let supervisor = json(SUPERVISOR_GOLDEN);
    for value in [&toolchain, &supervisor] {
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            strings(value, "/supportedSystems"),
            ["x86_64-linux", "aarch64-linux"]
        );
        assert!(!value.to_string().contains("/nix/store/"));
    }
    assert_eq!(toolchain["derivationSha256Method"], "raw-drv-file-sha256");
    assert_eq!(supervisor["derivationSha256Method"], "raw-drv-file-sha256");
    assert_eq!(supervisor["protocol"]["privateExecutableFd"], 9);
    assert_eq!(supervisor["protocol"]["statusFd"], 8);
    assert_eq!(
        strings(&supervisor, "/sourceFiles"),
        [
            "tests/tools/d2b-bazel-exec-supervisor/supervisor.c",
            "tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c",
        ]
    );
}

#[test]
fn supervisor_source_has_the_closed_host_output_contract() {
    let source = read_repo_file(SUPERVISOR_SOURCE);
    assert_eq!(count(&source, "ptrace("), 4);
    assert!(contains_all(
        &source,
        &[
            "pipe2(exec_pipe, O_CLOEXEC | O_NONBLOCK)",
            "D2B_PRIVATE_EXECUTABLE_FD 9",
            "D2B_STATUS_FD 8",
            "\"D2BS\"",
            "\"D2BE\"",
            "D2B_READY",
            "D2B_EXECUTED",
            "D2B_EXITED",
            "D2B_SIGNALED",
            "execveat",
            "AT_EMPTY_PATH",
            "setpgid(0, 0)",
            "setpgid(child, child)",
            "getpgid(child)",
            "SIGCHLD",
            "SFD_CLOEXEC",
            "D2B-BZLEXEC-HELPER-SIGNAL-INHERITED-IGNORED",
            "D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION",
            "D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH",
            "D2B-BZLEXEC-HELPER-EXEC-PARTIAL",
            "D2B-BZLEXEC-HELPER-EXEC-OVERLONG",
            "D2B-BZLEXEC-HELPER-EXEC-EPIPE",
            "D2B-BZLEXEC-HELPER-SIGNAL-FORWARD",
            "D2B-BZLEXEC-HELPER-DEADLINE",
            "D2B-BZLEXEC-HELPER-REAP",
        ]
    ));
    for forbidden in [
        "fexecve",
        "/proc/self/fd",
        "PTRACE_ATTACH",
        "PTRACE_SEIZE",
        "CAP_SYS_PTRACE",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden supervisor operation {forbidden}"
        );
    }
}

#[test]
fn real_supervisor_rebinds_by_open_file_description_and_emits_exact_output() {
    let root = repo_root();
    let source = root.join(SUPERVISOR_SOURCE);
    let scratch = ScratchDir::new(root.join(".scratch/policy-bazel-nix-supervisor"));
    let supervisor = scratch.path().join("supervisor");
    let provider = scratch.path().join("provider");
    let replacement = scratch.path().join("replacement");
    let status = scratch.path().join("status");
    let helper_error = scratch.path().join("helper-error");
    let stdin = scratch.path().join("stdin");
    let stdout = scratch.path().join("stdout");
    let stderr = scratch.path().join("stderr");

    let compile = Command::new("cc")
        .current_dir(scratch.path())
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            "-Wno-unused-result",
            "-fno-pie",
            "-no-pie",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&supervisor)
        .output()
        .expect("reviewed C compiler must be available");
    assert!(
        compile.status.success(),
        "reviewed C supervisor must compile"
    );

    let shell = fs::canonicalize("/bin/sh").expect("host shell must resolve");
    fs::copy(shell, &provider).expect("host shell must be copyable");
    fs::copy(&supervisor, &replacement).expect("replacement executable must be copyable");
    fs::write(&stdin, "supervisor-stdin\n").expect("stdin fixture");

    let launcher = r#"
use strict;
use warnings;
use Fcntl qw(F_DUPFD O_CREAT O_RDONLY O_TRUNC O_WRONLY);
use POSIX qw(SIG_BLOCK SIGHUP SIGINT SIGQUIT SIGTERM dup2 sigprocmask);

my ($_label, $provider, $replacement, $status, $helper_error, $stdin, $stdout, $stderr, $supervisor, $script) = @ARGV;
my $managed = POSIX::SigSet->new(SIGHUP, SIGINT, SIGTERM, SIGQUIT);
sigprocmask(SIG_BLOCK, $managed) or die "signal mask";

sysopen(my $provider_fd, $provider, O_RDONLY) or die "provider";
rename($replacement, $provider) or die "rebind";
sysopen(my $status_fd, $status, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "status";
sysopen(my $helper_error_fd, $helper_error, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "helper error";
sysopen(my $stdin_fd, $stdin, O_RDONLY) or die "stdin";
sysopen(my $stdout_fd, $stdout, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "stdout";
sysopen(my $stderr_fd, $stderr, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "stderr";

my @mappings = (
    [$provider_fd, 9, "provider"],
    [$status_fd, 8, "status"],
    [$helper_error_fd, 10, "helper error"],
    [$stdin_fd, 0, "stdin"],
    [$stdout_fd, 1, "stdout"],
    [$stderr_fd, 2, "stderr"],
);
my @sources = map {
    my $duplicate = fcntl($_->[0], F_DUPFD, 20);
    defined($duplicate) or die "$_->[2] duplicate";
    $duplicate;
} @mappings;
for my $index (0 .. $#mappings) {
    dup2($sources[$index], $mappings[$index]->[1])
        or die "$mappings[$index]->[2] fd";
}
POSIX::close($_) for @sources;

exec { $supervisor } $supervisor, "opened-provider", "-c", $script
    or die "exec";
"#;
    let target_script = r#"
read -r line
printf 'stdout:%s\n' "$line"
printf 'stderr:%s\n' "$line" >&2
test ! -e /proc/self/fd/8
test ! -e /proc/self/fd/9
"#;
    let result = Command::new("perl")
        .args([
            "-e",
            launcher,
            "d2b-supervisor-host-contract",
            provider.to_str().expect("provider path is UTF-8"),
            replacement.to_str().expect("replacement path is UTF-8"),
            status.to_str().expect("status path is UTF-8"),
            helper_error.to_str().expect("helper error path is UTF-8"),
            stdin.to_str().expect("stdin path is UTF-8"),
            stdout.to_str().expect("stdout path is UTF-8"),
            stderr.to_str().expect("stderr path is UTF-8"),
            supervisor.to_str().expect("supervisor path is UTF-8"),
            target_script,
        ])
        .output()
        .expect("host-backed supervisor must start");

    assert!(
        result.status.success(),
        "host-backed supervisor must succeed: status={:?} launcher-stderr={} helper-error={:?} status-bytes={:?}",
        result.status,
        String::from_utf8_lossy(&result.stderr),
        fs::read(&helper_error).unwrap_or_default(),
        fs::read(&status).unwrap_or_default(),
    );
    assert_eq!(
        fs::read_to_string(&stdout).expect("stdout capture"),
        "stdout:supervisor-stdin\n"
    );
    assert_eq!(
        fs::read_to_string(&stderr).expect("stderr capture"),
        "stderr:supervisor-stdin\n"
    );
    assert_eq!(
        fs::read(&status).expect("status capture"),
        [
            b'D', b'2', b'B', b'S', 1, 1, 0, 0, b'D', b'2', b'B', b'S', 1, 2, 0, 0, b'D', b'2',
            b'B', b'S', 1, 3, 0, 1, 0,
        ]
    );
    assert!(
        fs::read(&helper_error)
            .expect("helper error capture")
            .is_empty()
    );
}

#[test]
fn framed_status_and_exec_error_mutations_are_fail_closed() {
    let frames = [
        status_frame(1, &[]),
        status_frame(2, &[]),
        status_frame(3, &[0]),
    ]
    .concat();
    assert_eq!(frames.len(), 25);
    assert_eq!(
        decode_status(&frames).expect("coalesced status"),
        vec![1, 2, 3]
    );
    for split in 0..=frames.len() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&frames[..split]);
        bytes.extend_from_slice(&frames[split..]);
        assert_eq!(
            decode_status(&bytes).expect("fragmented status"),
            vec![1, 2, 3]
        );
    }
    let mut mutations = Vec::new();
    let mut bad_magic = frames.clone();
    bad_magic[0] = b'X';
    mutations.push(bad_magic);
    let mut bad_type = frames.clone();
    bad_type[5] = 9;
    mutations.push(bad_type);
    let mut duplicate_ready = status_frame(2, &[]);
    duplicate_ready.extend_from_slice(&frames);
    mutations.push(duplicate_ready);
    let mut trailing = frames.clone();
    trailing.extend_from_slice(&[0]);
    mutations.push(trailing);
    for mutation in mutations {
        assert!(decode_status(&mutation).is_err());
    }

    let record = [b'D', b'2', b'B', b'E', 1, 1, 0, 7];
    assert_eq!(decode_exec_error(&record, true), Some(7));
    assert_eq!(decode_exec_error(&record[..3], true), None);
    assert_eq!(decode_exec_error(&record, false), None);
    let mut overlong = record.to_vec();
    overlong.push(0);
    assert_eq!(decode_exec_error(&overlong, true), None);
}

fn status_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![b'D', b'2', b'B', b'S', 1, kind, 0, 0];
    if kind >= 3 {
        frame.extend_from_slice(payload);
    }
    frame
}

fn decode_status(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    if bytes.len() > 27 {
        return Err(());
    }
    let mut position = 0;
    let mut expected = 1;
    let mut kinds = Vec::new();
    while position < bytes.len() {
        if bytes.len() - position < 8 || &bytes[position..position + 4] != b"D2BS" {
            return Err(());
        }
        if bytes[position + 4] != 1 || bytes[position + 5] != expected {
            return Err(());
        }
        let length = if expected >= 3 { 9 } else { 8 };
        if bytes.len() - position < length {
            return Err(());
        }
        if expected >= 3 && bytes[position + 8] > 64 {
            return Err(());
        }
        kinds.push(expected);
        position += length;
        expected += 1;
        if expected == 4 && position != bytes.len() {
            return Err(());
        }
    }
    if expected != 4 {
        return Err(());
    }
    Ok(kinds)
}

fn decode_exec_error(bytes: &[u8], eof: bool) -> Option<u8> {
    if bytes.len() != 8
        || &bytes[..4] != b"D2BE"
        || bytes[4] != 1
        || bytes[5] != 1
        || bytes[6] != 0
        || !eof
    {
        return None;
    }
    Some(bytes[7])
}

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(path: PathBuf) -> Self {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create policy scratch directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
