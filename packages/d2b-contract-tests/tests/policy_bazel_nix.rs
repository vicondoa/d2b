//! Fixture-independent policy for the Spec 003 Nix and artifact boundary.
//!
//! These tests deliberately inspect the repository contract and one reviewed
//! C test tool. They do not use Rust FFI, Rust process-control primitives, or
//! rendered fixture paths.

use std::{
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
        assert_eq!(
            count(section, "d2b-priv-broker"),
            1,
            "generic section must exclude broker exactly once"
        );
        assert_eq!(
            count(section, "d2b-guest-shell-runner"),
            1,
            "generic section must exclude guest exactly once"
        );
    }
    assert!(tests.contains("d2b-contract-tests"));
}

#[test]
fn all_four_selected_contexts_and_six_native_checks_are_bound() {
    let flake = read_repo_file(FLAKE);
    let package_policy = read_repo_file("packages/xtask/src/package_policy.rs");
    let test_rust = read_repo_file("tests/test-rust.sh");

    for path in [
        "x86_64-linux/x86_64-unknown-linux-gnu/broker-production",
        "x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool",
        "aarch64-linux/aarch64-unknown-linux-gnu/broker-production",
        "aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool",
    ] {
        assert!(
            package_policy.contains(path.split('/').next().unwrap()),
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

    for check in [
        "broker-production-dependency-policy",
        "guest-shell-runner-static-dependency-policy",
        "broker-production-package-policy",
        "guest-real-libshpool-package-policy",
        "broker-host-artifact-contract",
        "guest-static-elf",
    ] {
        assert!(flake.contains(check), "native flake check missing: {check}");
    }
    for path in [
        "broker-production/policy/metadata.json",
        "broker-production/policy/Cargo.lock",
        "guest-real-libshpool/production/closure.json",
        "guest-real-libshpool/production/Cargo.lock",
        "guest-real-libshpool/policy/metadata.json",
        "guest-real-libshpool/policy/Cargo.lock",
    ] {
        assert!(test_rust.contains(path), "Cargo gate input missing: {path}");
    }
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
        assert!(!baselines.to_string().contains("/nix/store/"));
        for row in rows {
            assert!(row["system"].is_string());
            assert!(row["artifact"].is_string());
            assert!(row["binaryBytes"].is_u64());
            assert!(row["closureCount"].is_u64());
            assert!(row["closureSha256"].is_string());
            assert!(row["selectedPolicyDigest"].is_string());
            assert!(row["measurementCommand"].is_string());
            assert!(row["candidateCommit"].is_string());
            assert!(row.get("rowAllowance").is_none());
            assert!(row.get("sizeAllowance").is_none());
            assert!(row["sizeGrowthAuthorization"].is_null());
        }
    }
}

#[test]
fn size_growth_authorization_has_only_the_closed_positive_delta_source() {
    let baseline = SizeRow {
        system: "x86_64-linux",
        artifact: "guest-static-elf",
        prior_bytes: 100,
    };
    assert!(size_authorization_valid(&baseline, 100, None));
    assert!(size_authorization_valid(
        &baseline,
        107,
        Some(&authorization(
            "x86_64-linux",
            "guest-static-elf",
            100,
            107,
            7,
            "reviews/artifact-growth.md",
            "candidate-digest",
            "review-digest",
        ))
    ));

    let negative = [
        size_authorization_valid(&baseline, 107, None),
        size_authorization_valid(
            &baseline,
            107,
            Some(
                &authorization(
                    "x86_64-linux",
                    "guest-static-elf",
                    100,
                    107,
                    7,
                    "reviews/artifact-growth.md",
                    "candidate-digest",
                    "review-digest",
                )
                .with_decision("denied"),
            ),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "aarch64-linux",
                "guest-static-elf",
                100,
                107,
                7,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "x86_64-linux",
                "broker-host-artifact-contract",
                100,
                107,
                7,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "x86_64-linux",
                "guest-static-elf",
                99,
                107,
                8,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "x86_64-linux",
                "guest-static-elf",
                100,
                108,
                7,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "x86_64-linux",
                "guest-static-elf",
                100,
                107,
                8,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            107,
            Some(&authorization(
                "x86_64-linux",
                "guest-static-elf",
                100,
                107,
                7,
                "/absolute/review.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
        size_authorization_valid(
            &baseline,
            108,
            Some(&authorization(
                "x86_64-linux",
                "guest-static-elf",
                100,
                107,
                7,
                "reviews/artifact-growth.md",
                "candidate-digest",
                "review-digest",
            )),
        ),
    ];
    assert!(negative.iter().all(|valid| !valid));
}

#[derive(Clone, Copy)]
struct SizeRow {
    system: &'static str,
    artifact: &'static str,
    prior_bytes: u64,
}

struct Authorization {
    value: Value,
}

impl Authorization {
    fn with_decision(mut self, decision: &str) -> Self {
        self.value["decision"] = Value::String(decision.to_owned());
        self
    }
}

fn authorization(
    system: &str,
    artifact: &str,
    prior: u64,
    new: u64,
    delta: u64,
    rationale: &str,
    candidate: &str,
    review: &str,
) -> Authorization {
    Authorization {
        value: serde_json::json!({
            "system": system,
            "artifact": artifact,
            "priorBinaryBytes": prior,
            "newBinaryBytes": new,
            "deltaBytes": delta,
            "rationalePath": rationale,
            "candidateContentSha256": candidate,
            "reviewRecordSha256": review,
            "decision": "approved",
        }),
    }
}

fn size_authorization_valid(
    row: &SizeRow,
    realized_bytes: u64,
    authorization: Option<&Authorization>,
) -> bool {
    let Some(authorization) = authorization else {
        return realized_bytes <= row.prior_bytes;
    };
    let value = &authorization.value;
    value["system"] == row.system
        && value["artifact"] == row.artifact
        && value["priorBinaryBytes"] == row.prior_bytes
        && value["newBinaryBytes"] == realized_bytes
        && value["deltaBytes"].as_u64() == realized_bytes.checked_sub(row.prior_bytes)
        && realized_bytes > row.prior_bytes
        && value["rationalePath"]
            .as_str()
            .is_some_and(|path| !Path::new(path).is_absolute())
        && value["candidateContentSha256"]
            .as_str()
            .is_some_and(|digest| !digest.is_empty() && !digest.contains('/'))
        && value["reviewRecordSha256"]
            .as_str()
            .is_some_and(|digest| !digest.is_empty() && !digest.contains('/'))
        && value["decision"] == "approved"
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
use Fcntl qw(O_CREAT O_RDONLY O_TRUNC O_WRONLY);
use POSIX qw(SIG_BLOCK SIGHUP SIGINT SIGQUIT SIGTERM dup2 sigprocmask);

my ($_label, $provider, $replacement, $status, $stdin, $stdout, $stderr, $supervisor, $script) = @ARGV;
my $managed = POSIX::SigSet->new(SIGHUP, SIGINT, SIGTERM, SIGQUIT);
sigprocmask(SIG_BLOCK, $managed) or die "signal mask";

sysopen(my $provider_fd, $provider, O_RDONLY) or die "provider";
rename($replacement, $provider) or die "rebind";
sysopen(my $status_fd, $status, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "status";
sysopen(my $stdin_fd, $stdin, O_RDONLY) or die "stdin";
sysopen(my $stdout_fd, $stdout, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "stdout";
sysopen(my $stderr_fd, $stderr, O_WRONLY | O_CREAT | O_TRUNC, 0600)
    or die "stderr";

dup2(fileno($provider_fd), 9) or die "provider fd";
dup2(fileno($status_fd), 8) or die "status fd";
dup2(fileno($stdin_fd), 0) or die "stdin fd";
dup2(fileno($stdout_fd), 1) or die "stdout fd";
dup2(fileno($stderr_fd), 2) or die "stderr fd";
close($provider_fd);
close($status_fd);
close($stdin_fd);
close($stdout_fd);
close($stderr_fd);

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
        "host-backed supervisor must succeed"
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
            b'B', b'S', 1, 3, 0, 0, 0,
        ]
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
