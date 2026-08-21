#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
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

fn make_variable_tokens(makefile: &str, name: &str) -> Vec<String> {
    let marker = format!("{name} := ");
    let mut tokens = Vec::new();
    let mut collecting = false;
    for line in makefile.lines() {
        if let Some(rest) = line.strip_prefix(&marker) {
            collecting = true;
            tokens.extend(
                rest.trim_end_matches('\\')
                    .split_whitespace()
                    .map(str::to_owned),
            );
            if !rest.trim_end().ends_with('\\') {
                break;
            }
            continue;
        }
        if !collecting {
            continue;
        }
        if !line.chars().next().is_some_and(char::is_whitespace) {
            break;
        }
        let rest = line.trim();
        tokens.extend(
            rest.trim_end_matches('\\')
                .split_whitespace()
                .map(str::to_owned),
        );
        if !rest.ends_with('\\') {
            break;
        }
    }
    tokens
}

fn make_target_blocks(makefile: &str) -> BTreeMap<String, String> {
    let mut blocks = BTreeMap::new();
    let mut current_targets = Vec::<String>::new();
    let mut current = String::new();

    let mut flush = |targets: &mut Vec<String>, body: &mut String| {
        for target in targets.drain(..) {
            blocks.insert(target, body.clone());
        }
        body.clear();
    };

    for line in makefile.lines() {
        let header = if !line.is_empty()
            && !line.chars().next().is_some_and(char::is_whitespace)
            && !line.starts_with('#')
            && !line.starts_with('.')
        {
            line.split_once(':').and_then(|(lhs, _)| {
                if lhs.is_empty()
                    || lhs.contains(char::is_whitespace)
                    || lhs.contains('$')
                    || line.contains(":=")
                {
                    None
                } else {
                    Some(lhs.to_owned())
                }
            })
        } else {
            None
        };

        if let Some(target) = header {
            flush(&mut current_targets, &mut current);
            current_targets.push(target);
        }
        if !current_targets.is_empty() {
            current.push_str(line);
            current.push('\n');
        }
    }
    flush(&mut current_targets, &mut current);
    blocks
}

fn test_suite_labels(build: &str, suite: &str) -> Vec<String> {
    let needle = format!("name = \"{suite}\"");
    let name_at = build
        .find(&needle)
        .unwrap_or_else(|| panic!("test suite {suite} is missing"));
    let start = build[..name_at]
        .rfind("test_suite(")
        .unwrap_or_else(|| panic!("test suite rule for {suite} is missing"));
    let end = build[name_at..]
        .find("\n)\n")
        .map(|offset| name_at + offset + 3)
        .unwrap_or(build.len());
    build[start..end]
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix("\","))
                .filter(|value| value.starts_with("//"))
                .map(str::to_owned)
        })
        .collect()
}

fn rule_tags(block: &str) -> BTreeSet<String> {
    let Some(tags_at) = block.find("tags = [") else {
        return BTreeSet::new();
    };
    let tags = &block[tags_at
        ..block[tags_at..]
            .find(']')
            .map(|end| end + tags_at)
            .unwrap_or(block.len())];
    tags.split('"')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value.to_owned()))
        .collect()
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
        !bazelrc.contains("CargoBuildScriptRun=+no-remote"),
        "Cargo build scripts must compile on BuildBuddy so C objects match the worker glibc"
    );
    assert!(
        !bazelrc.contains("Rustc=+no-remote"),
        "Rustc must use BuildBuddy remote execution and cache"
    );
    assert!(
        !bazelrc.contains("build:remote --strategy=TestRunner=local")
            && !bazelrc.contains("build:trusted-seed --strategy=TestRunner=local"),
        "remote and trusted-seed profiles must defer test locality to target tags"
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
        wrapper.contains("D2B_PROJECT_SHELL"),
        "the Bazel facade must validate the d2b shell marker"
    );
    assert!(
        !wrapper.contains("/nix/store/"),
        "the Bazel facade must not hard-code a Nix-store Bazel path"
    );
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
        .env("D2B_PROJECT_SHELL", "d2b")
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
        .env("D2B_PROJECT_SHELL", "d2b")
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
        .env("D2B_PROJECT_SHELL", "d2b")
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
fn bazel_check_rejects_an_incomplete_project_shell_contract() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("bazel-check-shell-contract-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create wrapper test scratch");
    let bazel = scratch.join("bazel");
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         exit 99\n",
    );

    let output = Command::new("bash")
        .arg(repo_root().join("tests/tools/bazel-check"))
        .args(["--profile", "local", "--", "//:test"])
        .env("D2B_BAZEL_BIN", &bazel)
        .env_remove("D2B_PROJECT_SHELL")
        .env("D2B_XTASK_BIN", env!("CARGO_BIN_EXE_xtask"))
        .env("D2B_BAZEL_CHECK_SCRATCH", scratch.join("evidence"))
        .output()
        .expect("run bazel-check");

    assert_eq!(output.status.code(), Some(76));
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostics.contains("D2B_PROJECT_SHELL"));
    assert!(!diagnostics.contains("exit 99"));
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn make_dispatches_multiple_goals_once_and_preserves_bazel_variables() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("make-dispatch-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create make dispatcher scratch");

    let bazel_log = scratch.join("bazel.log");
    let bazel = scratch.join("bazel");
    write_executable(
        &bazel,
        "#!/usr/bin/env bash\n\
         set -eu\n\
         bep=''\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf '%s\\n' \"$D2B_BAZEL_PROFILE|$PWD|$BAZEL_SH|$D2B_BAZEL_UNTRUSTED|$MAKEFLAGS|$*\" >> \"$D2B_FAKE_BAZEL_LOG\"\n\
         printf '{\"testResult\":{\"status\":\"PASSED\"}}\\n' > \"$bep\"\n",
    );
    let xtask = scratch.join("xtask");
    write_executable(
        &xtask,
        "#!/usr/bin/env bash\n\
         set -eu\n",
    );
    let nix_count = scratch.join("nix.count");
    let nix = scratch.join("nix");
    write_executable(
        &nix,
        "#!/bin/sh\n\
         set -eu\n\
         printf 'entered\\n' >> \"$D2B_FAKE_NIX_COUNT\"\n\
         while [ \"$#\" -gt 0 ] && [ \"$1\" != -c ]; do shift; done\n\
         [ \"$#\" -gt 0 ] || exit 91\n\
         shift\n\
         export D2B_PROJECT_SHELL=d2b\n\
         export D2B_MAKE_REENTRY=1\n\
         export D2B_BAZEL_BIN=\"$D2B_FAKE_BAZEL\"\n\
         export D2B_XTASK_BIN=\"$D2B_FAKE_XTASK\"\n\
         exec \"$@\"\n",
    );

    let mut path = scratch.display().to_string();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());
    let output = Command::new("make")
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "-j2",
            "D2B_MAKE_REENTRY=0",
            "D2B_BAZEL_TEST_TAG_FILTERS=dispatcher-filter",
            "test-lint",
            "test-policy",
        ])
        .env("PATH", &path)
        .env("D2B_FAKE_NIX_COUNT", &nix_count)
        .env("D2B_FAKE_BAZEL", &bazel)
        .env("D2B_FAKE_XTASK", &xtask)
        .env("D2B_FAKE_BAZEL_LOG", &bazel_log)
        .env("D2B_BAZEL_PROFILE", "local")
        .env("D2B_BAZEL_UNTRUSTED", "1")
        .env("BAZEL_SH", "/bin/bash")
        .env("IN_NIX_SHELL", "impure")
        .env(
            "D2B_BAZEL_CHECK_SCRATCH",
            scratch.join("bazel-check-evidence"),
        )
        .env_remove("D2B_PROJECT_SHELL")
        .env_remove("D2B_MAKE_REENTRY")
        .output()
        .expect("run make through the fake Nix shell");

    assert!(
        output.status.success(),
        "make dispatcher failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = std::fs::read_to_string(&nix_count)
        .expect("read Nix re-entry count")
        .lines()
        .count();
    assert_eq!(entries, 1, "multiple goals entered Nix more than once");
    let bazel_output = std::fs::read_to_string(&bazel_log).expect("read fake Bazel log");
    assert_eq!(bazel_output.lines().count(), 2);
    assert!(bazel_output.lines().all(|line| {
        line.contains("local")
            && line.contains(repo_root().to_str().expect("repository root path"))
            && line.contains("|/bin/bash|1|")
            && line.contains("-j2")
            && line.contains("--test_tag_filters=dispatcher-filter")
    }));

    let direct_output = Command::new("make")
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "test-lint",
        ])
        .env("PATH", &path)
        .env("D2B_FAKE_BAZEL", &bazel)
        .env("D2B_FAKE_XTASK", &xtask)
        .env("D2B_XTASK_BIN", &xtask)
        .env("D2B_FAKE_BAZEL_LOG", &bazel_log)
        .env("D2B_BAZEL_PROFILE", "local")
        .env("D2B_BAZEL_UNTRUSTED", "1")
        .env("BAZEL_SH", "/bin/bash")
        .env("D2B_BAZEL_TEST_TAG_FILTERS", "direct-filter")
        .env(
            "D2B_BAZEL_CHECK_SCRATCH",
            scratch.join("direct-bazel-check-evidence"),
        )
        .env("D2B_PROJECT_SHELL", "d2b")
        .env("D2B_BAZEL_BIN", &bazel)
        .env_remove("D2B_MAKE_REENTRY")
        .output()
        .expect("run make inside the d2b shell contract");
    assert!(
        direct_output.status.success(),
        "direct d2b-shell make failed: {}{}",
        String::from_utf8_lossy(&direct_output.stdout),
        String::from_utf8_lossy(&direct_output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&nix_count)
            .expect("read direct-shell Nix re-entry count")
            .lines()
            .count(),
        1,
        "a valid d2b shell must not enter Nix again"
    );
    let direct_bazel_output = std::fs::read_to_string(&bazel_log).expect("read direct Bazel log");
    assert_eq!(direct_bazel_output.lines().count(), 3);
    assert!(
        direct_bazel_output
            .lines()
            .last()
            .is_some_and(|line| line.contains("--test_tag_filters=direct-filter"))
    );
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn make_dry_run_does_not_enter_nix() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("make-dry-run-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create dry-run scratch");
    let nix = scratch.join("nix");
    let log = scratch.join("nix.log");
    write_executable(
        &nix,
        "#!/bin/sh\n\
         printf entered >> \"$D2B_DRY_RUN_LOG\"\n",
    );
    let mut path = scratch.display().to_string();
    path.push(':');
    path.push_str(&std::env::var("PATH").expect("PATH"));
    let output = Command::new("make")
        .args([
            "--no-print-directory",
            "-n",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "test-lint",
        ])
        .env("PATH", path)
        .env("D2B_DRY_RUN_LOG", &log)
        .env_remove("D2B_PROJECT_SHELL")
        .env_remove("D2B_MAKE_REENTRY")
        .output()
        .expect("run make dry-run");
    assert!(
        output.status.success(),
        "make dry-run failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !log.exists(),
        "dry-run must print the dispatcher without entering Nix"
    );
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn make_dispatch_requires_nix_outside_the_d2b_shell() {
    let make = [
        "/usr/bin/make",
        "/bin/make",
        "/run/current-system/sw/bin/make",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
    .expect("make must be installed for dispatcher coverage");
    let output = Command::new(make)
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "test-lint",
        ])
        .env("PATH", "/usr/bin:/bin")
        .env_remove("D2B_PROJECT_SHELL")
        .env_remove("D2B_MAKE_REENTRY")
        .env_remove("D2B_BAZEL_BIN")
        .output()
        .expect("run make without Nix in PATH");
    assert_eq!(
        output.status.code(),
        Some(2),
        "make should fail closed when Nix is unavailable: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Nix is required"),
        "missing-Nix failure should explain the required shell contract: {combined}"
    );
}

#[test]
fn make_reentry_rejects_an_incomplete_d2b_shell_contract() {
    let output = Command::new("make")
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "test-lint",
        ])
        .env("PATH", std::env::var("PATH").expect("PATH"))
        .env("D2B_MAKE_REENTRY", "1")
        .env("D2B_PROJECT_SHELL", "d2b")
        .env_remove("D2B_BAZEL_BIN")
        .output()
        .expect("run make with an incomplete re-entry contract");
    assert_eq!(
        output.status.code(),
        Some(2),
        "incomplete re-entry should fail during Makefile parsing: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("shell contract is incomplete"),
        "failure should identify the incomplete d2b shell contract: {combined}"
    );
}

#[test]
fn make_reentry_does_not_retry_a_failed_nix_shell() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("make-failed-reentry-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create failed-reentry scratch");
    let nix = scratch.join("nix");
    let log = scratch.join("nix.log");
    write_executable(
        &nix,
        "#!/bin/sh\n\
         printf entered >> \"$D2B_FAILED_REENTRY_LOG\"\n\
         exit 41\n",
    );
    let mut path = scratch.display().to_string();
    path.push(':');
    path.push_str(&std::env::var("PATH").expect("PATH"));
    let output = Command::new("make")
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "test-lint",
        ])
        .env("PATH", path)
        .env("D2B_FAILED_REENTRY_LOG", &log)
        .env_remove("D2B_PROJECT_SHELL")
        .env_remove("D2B_MAKE_REENTRY")
        .output()
        .expect("run make through a failing Nix shell");
    assert!(
        !output.status.success(),
        "a failed Nix re-entry must fail the public Make target"
    );
    assert_eq!(
        std::fs::read_to_string(&log).expect("read failed-reentry log"),
        "entered",
        "failed shell re-entry must not recurse or retry"
    );
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn focused_bazel_shell_exports_the_complete_facade_contract() {
    let flake = read_text("flake.nix");
    let bazel_start = flake
        .find("bazel = pkgs.mkShellNoCC")
        .expect("focused Bazel shell definition");
    let bazel_end = flake[bazel_start..]
        .find("gas-city = pkgs.mkShell")
        .map(|offset| bazel_start + offset)
        .expect("focused Bazel shell boundary");
    let bazel_shell = &flake[bazel_start..bazel_end];
    for dependency in [
        "pkgs.bash",
        "pkgs.coreutils",
        "pkgs.findutils",
        "pkgs.gawk",
        "pkgs.git",
        "pkgs.gnugrep",
        "pkgs.gnused",
        "pkgs.gnumake",
        "pkgs.jq",
        "pkgs.rustup",
    ] {
        assert!(
            bazel_shell.contains(dependency),
            "focused Bazel shell must provide {dependency}"
        );
    }
    for export in [
        "D2B_PROJECT_SHELL=d2b",
        "D2B_BAZEL_BIN",
        "BAZEL_SH",
        "D2B_BAZEL_TEST_PATH",
    ] {
        assert!(
            bazel_shell.contains(export),
            "focused Bazel shell must export {export}"
        );
    }
}

#[test]
fn default_shell_includes_the_pinned_bazel_contract() {
    let flake = read_text("flake.nix");
    let default_start = flake
        .find("default = pkgs.mkShell")
        .expect("default development shell definition");
    let default_end = flake[default_start..]
        .find("nix-unit = pkgs.mkShellNoCC")
        .map(|offset| default_start + offset)
        .expect("default development shell boundary");
    let default_shell = &flake[default_start..default_end];
    assert!(
        default_shell.contains("bazel920"),
        "default development shell must include the pinned Bazel package"
    );
    for export in ["D2B_PROJECT_SHELL=d2b", "D2B_BAZEL_BIN", "BAZEL_SH"] {
        assert!(
            default_shell.contains(export),
            "default development shell must export {export}"
        );
    }
}

#[test]
fn ci_uses_public_make_aliases_without_nested_nix_develop_wrappers() {
    let workflow = read_text(".github/workflows/pr-l1-static-fast.yml");
    assert!(
        workflow
            .lines()
            .filter(|line| line.contains("nix develop"))
            .all(|line| !line.contains("make")),
        "CI must use the public Make dispatcher instead of per-target Nix wrappers"
    );
    assert!(
        workflow.contains("D2B_BAZEL_PROFILE: local")
            && workflow.contains("D2B_BAZEL_UNTRUSTED: \"1\""),
        "CI must keep the local and untrusted BuildBuddy boundary"
    );
    let make_runs = workflow
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("run: make ") || trimmed == "run: make"
        })
        .count();
    assert!(
        make_runs >= 10,
        "the Layer-1 workflow should exercise public Make aliases directly (found {make_runs})"
    );
}

#[test]
fn make_dispatch_classification_covers_bazel_and_recursive_validation_targets() {
    let makefile = read_text("Makefile");
    let classes = [
        (
            "D2B_MAKE_BAZEL_TARGETS",
            make_variable_tokens(&makefile, "D2B_MAKE_BAZEL_TARGETS"),
        ),
        (
            "D2B_MAKE_LOCAL_TARGETS",
            make_variable_tokens(&makefile, "D2B_MAKE_LOCAL_TARGETS"),
        ),
        (
            "D2B_MAKE_UTILITY_TARGETS",
            make_variable_tokens(&makefile, "D2B_MAKE_UTILITY_TARGETS"),
        ),
    ];
    let mut owners = BTreeMap::<String, &str>::new();
    for (class, targets) in &classes {
        for target in targets {
            assert!(
                owners.insert(target.clone(), class).is_none(),
                "{target} appears in multiple Make dispatcher classes"
            );
        }
    }
    let blocks = make_target_blocks(&makefile);
    for target in owners.keys() {
        assert!(
            blocks.contains_key(target),
            "{target} is classified but has no Make target definition"
        );
    }

    assert!(
        classes[1]
            .1
            .iter()
            .any(|target| target == "heavy-lane-perf"),
        "heavy-lane-perf must remain an explicit local dispatcher target"
    );
    assert!(
        !owners.contains_key("heavy-gate-provision") && !owners.contains_key("clean"),
        "maintenance targets must not be classified merely by their names"
    );

    for (target, block) in &blocks {
        if target == "__d2b_make_dispatch" {
            continue;
        }
        let executable_lines = block
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        let invokes_bazel = executable_lines.contains("tests/tools/bazel-check")
            || executable_lines.contains("$(BAZEL_RUN)")
            || executable_lines.contains("$(BAZEL_BIN)");
        if invokes_bazel {
            assert!(
                owners.contains_key(target),
                "Bazel-owned target {target} is missing from the dispatcher classes"
            );
        }

        let references_classified_target = owners.keys().find(|candidate| {
            candidate.as_str() != target.as_str()
                && executable_lines
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
                    .any(|word| word == candidate.as_str())
        });
        if let Some(referenced) = references_classified_target {
            assert!(
                owners.contains_key(target),
                "validation target {target} reaches classified target {referenced} but is not classified"
            );
        }
    }
}

#[test]
fn make_dispatch_preserves_mixed_local_and_utility_goals_with_one_shell_entry() {
    let scratch = repo_root()
        .join(".scratch")
        .join(format!("make-dispatch-mixed-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create mixed dispatcher scratch");

    let nix_count = scratch.join("nix.count");
    let bazel_log = scratch.join("bazel.log");
    let fake_bazel = scratch.join("bazel");
    let fake_nix = scratch.join("nix");
    let fake_xtask = scratch.join("xtask");
    let heavy_gate_bin = format!(
        "HEAVY_GATE_BIN={}",
        fake_xtask.to_str().expect("fake xtask path")
    );
    let make_wrapper = scratch.join("Makefile");
    std::fs::write(
        &make_wrapper,
        format!(
            "include {}\n\
             heavy-lane-perf: override D2B_BAZEL_TEST_TAG_FILTERS := target-specific-filter\n",
            repo_root().join("Makefile").display()
        ),
    )
    .expect("write target-specific Make wrapper");
    let recursive_make = format!("MAKE=make -f {}", make_wrapper.display());
    write_executable(
        &fake_xtask,
        "#!/bin/sh\n\
         set -eu\n\
         if [ \"${1:-}\" = bazel-evidence ] && [ \"${2:-}\" = redact-log ]; then exit 0; fi\n\
         if [ \"${1:-}\" = heavy-gate ] && [ \"${2:-}\" = verify-slot ]; then exit 0; fi\n\
         exit 90\n",
    );
    write_executable(
        &fake_bazel,
        "#!/usr/bin/env bash\n\
         set -eu\n\
         bep=''\n\
         for arg in \"$@\"; do\n\
           case \"$arg\" in\n\
             --build_event_json_file=*) bep=\"${arg#*=}\" ;;\n\
           esac\n\
         done\n\
         printf '%s\\n' \"$D2B_BAZEL_PROFILE|$PWD|$BAZEL_SH|$D2B_BAZEL_UNTRUSTED|$MAKEFLAGS|$*\" >> \"$D2B_FAKE_BAZEL_LOG\"\n\
         if [ \"${1:-}\" = build ]; then exit 0; fi\n\
         printf '{\"testResult\":{\"status\":\"PASSED\"}}\\n' > \"$bep\"\n",
    );
    write_executable(
        &fake_nix,
        "#!/bin/sh\n\
         set -eu\n\
         printf 'entered\\n' >> \"$D2B_FAKE_NIX_COUNT\"\n\
         while [ \"$#\" -gt 0 ] && [ \"$1\" != -c ]; do shift; done\n\
         [ \"$#\" -gt 0 ] || exit 91\n\
         shift\n\
         export D2B_PROJECT_SHELL=d2b\n\
         export D2B_MAKE_REENTRY=1\n\
         export D2B_BAZEL_BIN=\"$D2B_FAKE_BAZEL\"\n\
         export D2B_XTASK_BIN=\"$D2B_FAKE_XTASK\"\n\
         exec \"$@\"\n",
    );

    let mut path = scratch.display().to_string();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());
    let output = Command::new("make")
        .args([
            "--no-print-directory",
            "-C",
            repo_root().to_str().expect("repository root path"),
            "-f",
            make_wrapper
                .to_str()
                .expect("target-specific Make wrapper path"),
            "-j2",
            recursive_make.as_str(),
            "D2B_MAKE_REENTRY=0",
            "D2B_BAZEL_PROFILE=local",
            heavy_gate_bin.as_str(),
            "heavy-lane-perf",
            "heavy-gate-build",
        ])
        .env("PATH", &path)
        .env("D2B_FAKE_NIX_COUNT", &nix_count)
        .env("D2B_FAKE_BAZEL", &fake_bazel)
        .env("D2B_FAKE_XTASK", &fake_xtask)
        .env("D2B_FAKE_BAZEL_LOG", &bazel_log)
        .env("D2B_BAZEL_UNTRUSTED", "1")
        .env("BAZEL_SH", "/bin/bash")
        .env_remove("D2B_PROJECT_SHELL")
        .env_remove("D2B_MAKE_REENTRY")
        .output()
        .expect("run mixed local and utility Make goals");

    assert!(
        output.status.success(),
        "mixed Make goals failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&nix_count)
            .expect("read mixed Nix re-entry count")
            .lines()
            .count(),
        1,
        "mixed local and utility goals entered Nix more than once"
    );
    let log = std::fs::read_to_string(&bazel_log).expect("read mixed Bazel log");
    assert_eq!(
        log.lines().count(),
        2,
        "mixed goals should run one Bazel utility build and one test"
    );
    assert!(
        log.lines().all(|line| {
            line.contains("local") && line.contains("|/bin/bash|1|") && line.contains("-j2")
        }),
        "mixed goals did not preserve profile, trust, and parallelism: {log}"
    );
    assert!(
        log.lines()
            .any(|line| line.contains("--test_tag_filters=target-specific-filter")),
        "mixed goals did not preserve the target-specific Bazel filter: {log}"
    );
    let _ = std::fs::remove_dir_all(scratch);
}

fn build_rule_block<'a>(build: &'a str, target: &str) -> &'a str {
    let needle = format!("name = \"{target}\"");
    let name_at = build
        .find(&needle)
        .unwrap_or_else(|| panic!("target {target} is missing from BUILD file"));
    let start = build[..name_at]
        .rfind("rust_test(")
        .or_else(|| build[..name_at].rfind("sh_test("))
        .or_else(|| build[..name_at].rfind("filegroup("))
        .unwrap_or_else(|| panic!("rule start for {target} is missing"));
    let end = build[name_at..]
        .find("\n)\n")
        .map(|offset| name_at + offset + 3)
        .unwrap_or(build.len());
    &build[start..end]
}

#[test]
fn audited_local_rust_suite_is_complete_and_tag_driven() {
    let build = read_text("bazel/checks/rust/BUILD.bazel");
    let d2b = read_text("packages/d2b/BUILD.bazel");
    let labels = test_suite_labels(&build, "portable_rust_local");
    let mut unique = BTreeSet::new();
    assert!(!labels.is_empty(), "portable_rust_local must not be empty");
    for label in labels {
        assert!(
            unique.insert(label.clone()),
            "{label} is duplicated in portable_rust_local"
        );
        let relative = label
            .strip_prefix("//")
            .unwrap_or_else(|| panic!("invalid local Rust label {label}"));
        let (package, target) = relative
            .split_once(':')
            .unwrap_or_else(|| panic!("local Rust label has no target: {label}"));
        let package_build = read_text(&format!("{package}/BUILD.bazel"));
        let block = build_rule_block(&package_build, target);
        let tags = rule_tags(block);
        assert!(
            tags.contains("local") || tags.contains("no-remote-exec"),
            "{label} is in portable_rust_local without a locality tag"
        );
        assert!(
            !(tags.contains("local") && tags.contains("no-remote-exec")),
            "{label} redundantly combines local and no-remote-exec"
        );
    }
    let makefile = read_text("Makefile");
    let make_targets = make_target_blocks(&makefile);
    let main = make_targets
        .get("test-rust-main")
        .expect("test-rust-main Make target");
    assert!(
        main.contains("-local,-no-remote-exec"),
        "remote Rust main must exclude both local tag classes"
    );
    let hermetic = build_rule_block(&d2b, "auth_status_contract");
    assert!(
        !hermetic.contains("\"local\"") && !hermetic.contains("no-remote-cache"),
        "untagged hermetic tests must remain eligible for remote execution"
    );
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
