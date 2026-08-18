#![forbid(unsafe_code)]

use std::path::PathBuf;

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
