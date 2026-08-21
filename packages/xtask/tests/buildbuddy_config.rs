#![forbid(unsafe_code)]

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }

    for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root is not discoverable")
}

fn read_text(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn read_json(relative: &str) -> Value {
    serde_json::from_str(&read_text(relative))
        .unwrap_or_else(|error| panic!("parse {relative}: {error}"))
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
}

fn object<'a>(value: &'a Value, context: &str) -> &'a serde_json::Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object"))
}

#[test]
fn committed_profiles_share_authentication_and_worker_policy() {
    let bazelrc = read_text(".bazelrc");
    for (profile, marker) in [
        ("common", "common "),
        ("local", "build:local "),
        ("remote", "build:remote "),
        ("trusted-seed", "build:trusted-seed "),
    ] {
        assert!(
            bazelrc.contains(marker),
            ".bazelrc must define the {profile} profile"
        );
    }
    assert!(
        bazelrc.contains("try-import %workspace%/.bazelrc.user"),
        "private user configuration must be an optional import"
    );
    assert!(
        bazelrc.contains("--remote_download_outputs=minimal"),
        "remote profiles must use minimal output downloads"
    );
    assert!(
        bazelrc.contains("build --stamp=no"),
        "BuildBuddy metadata must not change unstamped action behavior"
    );
    assert!(
        !bazelrc.contains("CargoBuildScriptRun=+no-remote"),
        "Cargo build scripts must compile on BuildBuddy so C objects match the worker glibc"
    );
    assert!(
        !bazelrc.contains("Rustc=+no-remote"),
        "Rustc must use BuildBuddy remote execution and cache"
    );
    assert!(
        bazelrc.contains("--jobs=50"),
        "remote profiles should cap concurrency"
    );
    assert!(
        bazelrc.contains("--remote_cache_compression"),
        "remote profiles should compress cache blobs"
    );
    assert!(
        bazelrc.contains("--remote_download_minimal")
            || bazelrc.contains("--remote_download_outputs=minimal"),
        "remote profiles must avoid downloading unused outputs"
    );
    assert!(
        bazelrc.contains("--remote_retries=0"),
        "the wrapper owns the single local retry"
    );
    assert!(
        bazelrc.contains("--credential_helper="),
        "remote authentication must use Bazel's credential helper"
    );
    assert!(
        !bazelrc.contains("--remote_header") && !bazelrc.contains("--bes_header"),
        "header flags are forbidden"
    );
    assert!(
        !bazelrc.contains("--repo_contents_cache="),
        "repository content caching is not a remote profile feature"
    );
    assert!(
        !bazelrc
            .lines()
            .any(|line| line.contains("--experimental_") && line.contains("remote")),
        "experimental remote features must remain disabled"
    );
    assert!(
        !bazelrc.contains("build:qualification"),
        "obsolete qualification profile must remain absent"
    );
    let platforms = read_text("bazel/platforms/BUILD.bazel");
    assert!(
        platforms.matches("d2b-bazel-worker/v1").count() >= 2,
        "remote platforms must pin the immutable worker-image contract"
    );
    assert!(
        read_text("nix/bazel-worker-image.nix").contains("d2b-bazel-worker/v1"),
        "Nix must expose the worker-image contract"
    );
    assert!(
        read_text("flake.nix").contains("bazel-worker-image"),
        "the flake must wire the worker-image contract"
    );

    let user_example = read_text(".bazelrc.user.example");
    assert!(
        user_example.contains("--credential_helper="),
        "the user example must contain a credential-helper placeholder"
    );
    assert!(
        !user_example.contains("--remote_header") && !user_example.contains("--bes_header"),
        "the user example must not teach header authentication"
    );
    assert!(
        !user_example.contains("x-buildbuddy-api-key"),
        "the user example must not contain a credential header"
    );

    let wrapper = read_text("tests/tools/bazel-check");
    assert!(
        wrapper.contains("D2B_BAZEL_FALLBACK_ISOLATE_RC=1")
            && wrapper.contains("--nosystem_rc")
            && wrapper.contains("--nohome_rc")
            && wrapper.contains("local fallback rejects an external workspace Bazel rc"),
        "the one local fallback must ignore external Bazel rc files"
    );
    assert!(
        !wrapper.contains("--dispatch-evidence"),
        "BEP file presence alone must not suppress pre-dispatch fallback"
    );
    assert!(
        wrapper.contains("command_flags+=(--shell_executable=/bin/bash)"),
        "remote Bazel actions must use the worker's shell path"
    );
    assert!(
        wrapper.contains(
            "/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$test_path",
        ),
        "remote test runners must receive worker-standard PATH entries"
    );
}

#[test]
fn redaction_failure_never_emits_captured_evidence() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("bazel-check-redaction-test-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create wrapper test scratch");
    let bazel = scratch.join("bazel");
    let xtask = scratch.join("xtask");
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf 'RAW-LOG-SENTINEL\\n'\n\
         printf 'RAW-BEP-SENTINEL\\n' > \"$bep\"\n\
         printf 'remote execution started authorization: Bearer dispatch-token UNAUTHENTICATED\n'\n\
         exit 1\n",
    );
    write_executable(&xtask, "#!/usr/bin/env bash\nexit 1\n");

    let output = Command::new("bash")
        .arg(repo_root().join("tests/tools/bazel-check"))
        .args(["--profile", "local", "--", "//:test"])
        .env("D2B_BAZEL_BIN", &bazel)
        .env("D2B_XTASK_BIN", &xtask)
        .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join("evidence"))
        .output()
        .expect("run bazel-check");

    assert_eq!(output.status.code(), Some(75));
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostics.contains("evidence redaction failed"));
    assert!(!diagnostics.contains("RAW-LOG-SENTINEL"));
    assert!(!diagnostics.contains("RAW-BEP-SENTINEL"));
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn dispatch_evidence_survives_log_redaction_before_classification() {
    let scratch = repo_root().join(".scratch").join(format!(
        "bazel-check-dispatch-redaction-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).expect("create wrapper test scratch");
    let bazel = scratch.join("bazel");
    let credential = scratch.join("credential");
    std::fs::write(&credential, "synthetic-token\n").expect("write credential");
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf 'remote execution started authorization: Bearer synthetic-secret UNAUTHENTICATED\\n'\n\
         printf '{\"id\":{\"started\":{\"uuid\":\"dispatch\"}}}\\n' > \"$bep\"\n\
         exit 1\n",
    );

    let output = Command::new("bash")
        .arg(repo_root().join("tests/tools/bazel-check"))
        .args(["--profile", "remote", "--", "//:test"])
        .env("D2B_BAZEL_BIN", &bazel)
        .env("D2B_XTASK_BIN", env!("CARGO_BIN_EXE_xtask"))
        .env("D2B_BUILDBUDDY_CREDENTIAL_FILE", &credential)
        .env("D2B_BAZEL_UNTRUSTED", "0")
        .env("GITHUB_ACTIONS", "false")
        .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join("evidence"))
        .output()
        .expect("run bazel-check");

    assert!(!output.status.success());
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostics.contains("refusing local retry"));
    assert!(!diagnostics.contains("retrying the identical target set locally"));
    assert!(!diagnostics.contains("synthetic-secret"));
    assert!(!diagnostics.contains("dispatch-token"));
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn successful_bazel_requires_a_test_result_event() {
    let scratch = repo_root().join(".scratch").join(format!(
        "bazel-check-startup-only-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).expect("create wrapper test scratch");
    let bazel = scratch.join("bazel");
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf 'startup only\\n'\n\
         printf '{\"id\":{\"started\":{\"uuid\":\"startup-only\"}}}\\n' > \"$bep\"\n\
         exit 0\n",
    );

    let output = Command::new("bash")
        .arg(repo_root().join("tests/tools/bazel-check"))
        .args(["--profile", "local", "--", "//:test"])
        .env("D2B_BAZEL_BIN", &bazel)
        .env("D2B_XTASK_BIN", env!("CARGO_BIN_EXE_xtask"))
        .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join("evidence"))
        .output()
        .expect("run bazel-check");

    assert!(!output.status.success());
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostics.contains("no testResult event"));
    assert!(!diagnostics.contains("bazel-check: local passed"));
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn policy_preserves_remote_profiles_and_trust_partition() {
    let policy = read_json("tests/golden/bazel/cache-policy.json");
    let remote = object(
        object(&policy, "cache policy")
            .get("remote")
            .expect("remote policy"),
        "remote policy",
    );
    assert_eq!(
        remote.get("auth").and_then(Value::as_str),
        Some("credential-helper")
    );
    assert_eq!(
        remote.get("remoteDownloadOutputs").and_then(Value::as_str),
        Some("minimal")
    );
    assert_eq!(
        remote.get("workerImageContract").and_then(Value::as_str),
        Some("d2b-bazel-worker/v1")
    );
    assert!(
        policy["profiles"]["remote"]["namespace"]
            .as_str()
            .is_some_and(|namespace| namespace.contains("/worker-v1/minimal/lock-v1"))
    );
    assert_eq!(
        policy["profiles"]["trusted-seed"]["remoteCacheAsync"].as_bool(),
        Some(false)
    );
    assert!(
        remote["experimentalFeatures"]
            .as_array()
            .expect("experimental feature list")
            .is_empty()
    );

    let trusted = object(
        object(&policy, "cache policy")
            .get("trustedInjection")
            .expect("trusted injection policy"),
        "trusted injection policy",
    );
    assert_eq!(
        trusted.get("protectedRef").and_then(Value::as_str),
        Some("refs/heads/v3")
    );
    assert_eq!(
        trusted.get("untrustedCredential").and_then(Value::as_str),
        Some("none")
    );
    assert!(
        trusted["allowedSecurityDigests"]
            .as_array()
            .expect("security digest allowlist")
            .iter()
            .all(|digest| digest
                .as_str()
                .is_some_and(|value| value.starts_with("sha256:")))
    );
}

#[test]
fn developer_profiles_publish_the_tested_checkout_metadata() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("bazel-check-metadata-test-{}", std::process::id()));
    let bin = scratch.join("bin");
    std::fs::create_dir_all(&bin).expect("create metadata test bin directory");
    let bazel = bin.join("bazel");
    let git = bin.join("git");
    let xtask = bin.join("xtask");
    let credential = scratch.join("credential");
    std::fs::write(&credential, "synthetic-token\n").expect("write credential");
    write_executable(
        &git,
        "#!/usr/bin/env bash\n\
         if [ -n \"${GIT_DIR:-}${GIT_WORK_TREE:-}${GIT_COMMON_DIR:-}\" ]; then\n\
           case \"$*\" in\n\
             *'rev-parse --show-toplevel'*) printf '%s\\n' '/foreign-checkout' ;;\n\
             *'config --local --get remote.origin.url'*|*'remote get-url origin'*) printf '%s\\n' 'git@github.com:vicondoa/d2b.git' ;;\n\
             *'status --porcelain=v2 --branch --untracked-files=no -z'*) printf '# branch.oid %s\\0# branch.head %s\\0' 'ffffffffffffffffffffffffffffffffffffffff' 'v3' ;;\n\
             *'check-ref-format --branch'*) exit 0 ;;\n\
             *) exit 1 ;;\n\
           esac\n\
           exit 0\n\
         fi\n\
         case \"$*\" in\n\
           *'rev-parse --show-toplevel'*) printf '%s\\n' \"$2\" ;;\n\
           *'config --local --get remote.origin.url'*|*'remote get-url origin'*) printf '%s\\n' 'git@github.com:vicondoa/d2b.git' ;;\n\
           *'status --porcelain=v2 --branch --untracked-files=no -z'*) printf '# branch.oid %s\\0# branch.head %s\\0' '0123456789abcdef0123456789abcdef01234567' 'feat/issue+446@meta=one,two' ;;\n\
           *'check-ref-format --branch'*) exit 0 ;;\n\
           *) exit 1 ;;\n\
         esac\n",
    );
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         printf '%s\\n' \"$@\" > \"${D2B_CAPTURE_ARGS:?}\"\n\
         after_separator=0\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --) after_separator=1 ;;\n\
             --build_metadata=*) [ \"$after_separator\" -eq 0 ] || exit 64 ;;\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf '{\"testResult\":{\"status\":\"PASSED\"}}\\n' > \"$bep\"\n",
    );
    write_executable(
        &xtask,
        "#!/usr/bin/env bash\n\
         if [ \"$2\" = check-security ]; then exit 0; fi\n\
         if [ \"$2\" = redact-log ] && [ \"$4\" != \"$6\" ]; then cp -- \"$4\" \"$6\"; fi\n",
    );
    let relative_xtask = xtask
        .strip_prefix(&scratch)
        .expect("xtask stub must be below the test scratch directory");

    let run = |profile: &str, capture: &Path, trusted: bool| {
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("tests/tools/bazel-check"))
            .args(["--profile", profile, "--", "//:test"])
            .current_dir(&scratch)
            .env("D2B_BAZEL_BIN", &bazel)
            .env("D2B_XTASK_BIN", relative_xtask)
            .env("D2B_BUILDBUDDY_CREDENTIAL_FILE", &credential)
            .env("D2B_BAZEL_UNTRUSTED", "0")
            .env("GITHUB_ACTIONS", "false")
            .env("GIT_DIR", scratch.join("foreign.git"))
            .env("GIT_WORK_TREE", scratch.join("foreign-worktree"))
            .env("GIT_COMMON_DIR", scratch.join("foreign-common.git"))
            .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join(profile))
            .env("D2B_CAPTURE_ARGS", capture)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            );
        if trusted {
            command
                .env("D2B_BAZEL_TRUSTED", "1")
                .env("GITHUB_REF", "refs/heads/v3");
        }
        command.output().expect("run bazel-check metadata profile")
    };

    let remote_args = scratch.join("remote.args");
    let output = run("remote", &remote_args, false);
    assert!(output.status.success(), "remote profile failed: {output:?}");
    let trusted_args = scratch.join("trusted-seed.args");
    let output = run("trusted-seed", &trusted_args, true);
    assert!(
        output.status.success(),
        "trusted-seed profile failed: {output:?}"
    );
    let local_args = scratch.join("local.args");
    let output = run("local", &local_args, false);
    assert!(output.status.success(), "local profile failed: {output:?}");

    let expected = [
        "--build_metadata=REPO_URL=https://github.com/vicondoa/d2b",
        "--build_metadata=COMMIT_SHA=0123456789abcdef0123456789abcdef01234567",
        "--build_metadata=BRANCH_NAME=feat/issue+446@meta=one,two",
    ];
    let remote = std::fs::read_to_string(&remote_args).expect("read remote Bazel args");
    let trusted = std::fs::read_to_string(&trusted_args).expect("read trusted Bazel args");
    let local = std::fs::read_to_string(&local_args).expect("read local Bazel args");
    assert!(
        !local.contains("--build_metadata="),
        "local profile must not publish developer metadata"
    );
    let remote_metadata = remote
        .lines()
        .filter(|argument| argument.starts_with("--build_metadata="))
        .collect::<Vec<_>>();
    let trusted_metadata = trusted
        .lines()
        .filter(|argument| argument.starts_with("--build_metadata="))
        .collect::<Vec<_>>();
    assert_eq!(remote_metadata, expected);
    assert_eq!(trusted_metadata, expected);
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn invalid_checkout_metadata_is_omitted_explicitly() {
    let scratch = repo_root().join(".scratch").join(format!(
        "bazel-check-detached-metadata-test-{}",
        std::process::id()
    ));
    let bin = scratch.join("bin");
    std::fs::create_dir_all(&bin).expect("create detached metadata test bin directory");
    let bazel = bin.join("bazel");
    let git = bin.join("git");
    let xtask = bin.join("xtask");
    let credential = scratch.join("credential");
    std::fs::write(&credential, "synthetic-token\n").expect("write credential");
    write_executable(
        &git,
        "#!/usr/bin/env bash\n\
         mode=\"$(cat -- \"$(dirname -- \"$0\")/git-mode\")\"\n\
         [ \"$mode\" != unavailable ] || exit 1\n\
         case \"$*\" in\n\
           *'rev-parse --show-toplevel'*)\n\
             if [ \"$mode\" = foreign-root ]; then printf '%s\\n' '/foreign-checkout'; else printf '%s\\n' \"$2\"; fi\n\
             ;;\n\
           *'config --local --get remote.origin.url'*|*'remote get-url origin'*)\n\
             if [ \"$mode\" = foreign-origin ]; then printf '%s\\n' 'https://example.invalid/fork.git'; else printf '%s\\n' 'https://github.com/vicondoa/d2b.git'; fi\n\
             ;;\n\
           *'status --porcelain=v2 --branch --untracked-files=no -z'*)\n\
             oid='0123456789abcdef0123456789abcdef01234567'\n\
             head='feat/issue-446-buildbuddy-metadata'\n\
             case \"$mode\" in\n\
               invalid-commit) oid='not-a-commit' ;;\n\
               detached) head='(detached)' ;;\n\
               invalid-branch) head='bad branch' ;;\n\
               changing-head)\n\
                 marker=\"$(dirname -- \"$0\")/git-changing-head\"\n\
                 if [ -e \"$marker\" ]; then head='other'; else : > \"$marker\"; fi\n\
                 ;;\n\
             esac\n\
             printf '# branch.oid %s\\0# branch.head %s\\0' \"$oid\" \"$head\"\n\
             ;;\n\
           *'check-ref-format --branch'*)\n\
             [ \"$mode\" != invalid-branch ]\n\
             ;;\n\
           *) exit 1 ;;\n\
         esac\n",
    );
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         printf '%s\\n' \"$@\" > \"${D2B_CAPTURE_ARGS:?}\"\n\
         after_separator=0\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --) after_separator=1 ;;\n\
             --build_metadata=*) [ \"$after_separator\" -eq 0 ] || exit 64 ;;\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf '{\"testResult\":{\"status\":\"PASSED\"}}\\n' > \"$bep\"\n",
    );
    write_executable(
        &xtask,
        "#!/usr/bin/env bash\n\
         if [ \"$2\" = check-security ]; then exit 0; fi\n\
         if [ \"$2\" = redact-log ] && [ \"$4\" != \"$6\" ]; then cp -- \"$4\" \"$6\"; fi\n",
    );

    for (mode, expected_diagnostic) in [
        (
            "unavailable",
            "Git checkout is unavailable or does not match the tested repository",
        ),
        (
            "foreign-root",
            "Git checkout is unavailable or does not match the tested repository",
        ),
        ("foreign-origin", "Git origin is not the canonical d2b repository"),
        ("invalid-commit", "tested commit is unavailable"),
        ("detached", "detached HEAD"),
        ("invalid-branch", "branch name is not approved"),
        (
            "changing-head",
            "checkout changed while metadata was collected",
        ),
    ] {
        std::fs::write(bin.join("git-mode"), mode).expect("write Git test mode");
        let _ = std::fs::remove_file(bin.join("git-changing-head"));
        let capture = scratch.join(format!("{mode}.args"));
        let _ = std::fs::remove_file(&capture);
        let output = Command::new("bash")
            .arg(repo_root().join("tests/tools/bazel-check"))
            .args(["--profile", "remote", "--", "//:test"])
            .env("D2B_BAZEL_BIN", &bazel)
            .env("D2B_XTASK_BIN", &xtask)
            .env("D2B_BUILDBUDDY_CREDENTIAL_FILE", &credential)
            .env("D2B_BAZEL_UNTRUSTED", "0")
            .env("GITHUB_ACTIONS", "false")
            .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join(mode))
            .env("D2B_CAPTURE_ARGS", &capture)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .unwrap_or_else(|error| panic!("run bazel-check {mode} profile: {error}"));
        assert!(
            output.status.success(),
            "{mode} profile failed: {output:?}"
        );
        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            diagnostics.contains(expected_diagnostic),
            "{mode} diagnostics omitted {expected_diagnostic:?}: {diagnostics}"
        );
        assert!(
            capture.is_file(),
            "{mode} did not invoke Bazel or create its argument capture"
        );
        let args = std::fs::read_to_string(&capture)
            .unwrap_or_else(|error| panic!("read {mode} Bazel args: {error}"));
        assert!(
            !args.contains("--build_metadata="),
            "{mode} must omit all BuildBuddy metadata"
        );
    }
    let _ = std::fs::remove_dir_all(scratch);
}
