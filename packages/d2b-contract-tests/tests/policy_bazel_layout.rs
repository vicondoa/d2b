#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::process::Command;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use serde_json::Value;

const EXCEPTION_MANIFEST: &str = "bazel/exceptions/manifest.json";
const PRODUCT_BUILD_ROOT: &str = "packages";
const GAZELLE_FIXTURE_ROOT: &str = "tests/fixtures/bazel/gazelle";

fn git_listed_files(roots: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args([
            "-c",
            "core.quotePath=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots)
        .output()
        .expect("run git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect()
}

fn root_workspace_members() -> BTreeSet<String> {
    let cargo = read_repo_file("Cargo.toml");
    let members = cargo
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .expect("root Cargo.toml must declare workspace members")
        .0;
    members
        .lines()
        .filter_map(|line| {
            let member = line.trim().trim_matches(',');
            let member = member.strip_prefix('"')?.strip_suffix('"')?;
            member.starts_with("packages/").then(|| member.to_owned())
        })
        .collect()
}

fn package_build_files() -> BTreeSet<String> {
    git_listed_files(&[PRODUCT_BUILD_ROOT])
        .into_iter()
        .filter(|path| path.starts_with("packages/") && path.ends_with("/BUILD.bazel"))
        .collect()
}

fn exception_manifest() -> Value {
    serde_json::from_str(&read_repo_file(EXCEPTION_MANIFEST))
        .expect("Bazel exception manifest must be valid JSON")
}

fn exception_paths() -> BTreeSet<String> {
    exception_manifest()
        .get("exceptions")
        .and_then(Value::as_array)
        .expect("Bazel exception manifest must contain an exceptions array")
        .iter()
        .map(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .expect("each Bazel exception must declare a path")
                .to_owned()
        })
        .collect()
}

fn assert_closed_exception_manifest(manifest: &Value) {
    let entries = manifest
        .get("exceptions")
        .and_then(Value::as_array)
        .expect("Bazel exception manifest must contain an exceptions array");
    let mut labels = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let context = format!("exceptions[{index}]");
        let object = entry
            .as_object()
            .unwrap_or_else(|| panic!("{context} must be an object"));
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{context}.path must be a string"));
        let label = object
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{context}.label must be a string"));
        let reason = object
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{context}.reason must be a string"));
        let marker = object
            .get("marker")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{context}.marker must be a string"));
        assert!(!reason.is_empty(), "{context}.reason must not be empty");
        assert!(
            marker.contains("keep"),
            "{context}.marker must be a keep marker"
        );
        assert!(
            label.starts_with("//") && label.contains(':'),
            "{context}.label must be an absolute Bazel label"
        );
        assert!(
            labels.insert(label),
            "duplicate Bazel exception label: {label}"
        );
        assert!(paths.insert(path), "duplicate Bazel exception path: {path}");
        assert!(
            repo_path_exists(path),
            "{context} points to missing BUILD file {path}"
        );
        assert!(
            read_repo_file(path).contains(marker),
            "{context} marker {marker:?} is absent from {path}"
        );
    }
}

fn assert_no_repository_owned_generation(source: &str) {
    for forbidden in [
        "WORKSPACE",
        "local_path_override",
        "archive_override",
        "git_override",
        "single_version_override",
        "multiple_version_override",
        "patch",
        "overlay",
        "postprocessor",
        "custom_generator",
        "fork",
    ] {
        assert!(
            !source.contains(forbidden),
            "Bazel source must not contain repository-owned generation escape hatch {forbidden:?}"
        );
    }
}

#[test]
fn production_bazel_layout_has_one_locked_root_authority() {
    for path in [
        ".bazelversion",
        ".bazelrc",
        "MODULE.bazel",
        "MODULE.bazel.lock",
        "BUILD.bazel",
        "bazel/platforms/BUILD.bazel",
        "bazel/toolchains/BUILD.bazel",
        "bazel/exceptions/BUILD.bazel",
        EXCEPTION_MANIFEST,
    ] {
        assert!(repo_path_exists(path), "missing U3 Bazel surface: {path}");
    }

    let module = read_repo_file("MODULE.bazel");
    assert_eq!(
        module.matches("crate.from_cargo(").count(),
        1,
        "the product graph must have exactly one crate_universe Cargo authority"
    );
    assert!(
        module.contains("cargo_lockfile = \"//:Cargo.lock\""),
        "crate_universe must resolve the repository-root Cargo.lock"
    );
    assert!(
        module.contains("manifests = [\"//:Cargo.toml\"]"),
        "crate_universe must resolve the repository-root Cargo.toml"
    );
    assert!(
        !module.contains("Cargo.guest.lock")
            && !module.contains("tests/fixtures/bazel/compat/Cargo.lock"),
        "the product crate_universe must not absorb reduced guest or compatibility locks"
    );
    assert!(
        module.contains("rust.toolchain(")
            && module.contains("edition = \"2024\"")
            && module.contains("versions = [\"1.97.0\"]"),
        "Bzlmod must pin the Rust toolchain to rust-toolchain.toml"
    );
    assert!(
        module.contains("register_toolchains(\"@rust_toolchains//:all\")"),
        "Bzlmod must register the pinned Rust toolchain repository"
    );

    let lock = read_repo_file("MODULE.bazel.lock");
    assert!(
        lock.contains("\"lockFileVersion\"")
            && lock.contains("FILE:@@//Cargo.lock")
            && lock.contains("FILE:@@//Cargo.toml"),
        "Bzlmod lock must bind the root Cargo authority"
    );
    assert!(
        !lock.contains("Cargo.guest.lock")
            && !lock.contains("tests/fixtures/bazel/compat/Cargo.lock"),
        "Bzlmod lock must not bind reduced guest or compatibility Cargo locks"
    );

    let bazelrc = read_repo_file(".bazelrc");
    assert!(
        bazelrc
            .lines()
            .filter(|line| line.trim() == "common --lockfile_mode=error")
            .count()
            == 1,
        "Bazel must fail closed on MODULE.bazel.lock drift"
    );

    let cargo = read_repo_file("Cargo.toml");
    for required in [
        "packages/d2b-priv-broker",
        "packages/d2b-guest-shell-runner",
    ] {
        assert!(
            cargo.contains(&format!("\"{required}\"")),
            "root Cargo workspace is missing {required}"
        );
    }
}

#[test]
fn production_bazel_platform_and_toolchain_packages_are_explicit() {
    let platforms = read_repo_file("bazel/platforms/BUILD.bazel");
    for required in [
        "platform(",
        "name = \"x86_64_linux\"",
        "name = \"aarch64_linux\"",
        "@platforms//cpu:x86_64",
        "@platforms//cpu:aarch64",
        "@platforms//os:linux",
    ] {
        assert!(
            platforms.contains(required),
            "platform package is missing {required}"
        );
    }

    let toolchains = read_repo_file("bazel/toolchains/BUILD.bazel");
    for required in [
        "alias(",
        "rust_toolchain_x86_64_linux",
        "rust_toolchain_aarch64_linux",
        "@rust_toolchains//:rust_linux_x86_64__x86_64-unknown-linux-gnu__stable",
        "@rust_toolchains//:rust_linux_aarch64__aarch64-unknown-linux-gnu__stable",
    ] {
        assert!(
            toolchains.contains(required),
            "toolchain package is missing {required}"
        );
    }
}

#[test]
fn every_root_product_member_has_generated_or_declared_build_ownership() {
    let members = root_workspace_members();
    let builds = package_build_files();
    let exceptions = exception_paths();
    for member in members {
        let build = format!("{member}/BUILD.bazel");
        assert!(
            builds.contains(&build) || exceptions.contains(&build),
            "root Cargo member has no Bazel BUILD ownership: {member}"
        );
    }
    for exception in exceptions {
        assert!(
            exception.starts_with("packages/") || exception.starts_with("bazel/"),
            "Bazel exception escapes the product graph: {exception}"
        );
    }
}

#[test]
fn exception_manifest_and_generated_ownership_are_closed() {
    let manifest = exception_manifest();
    assert_eq!(
        manifest.get("version").and_then(Value::as_u64),
        Some(1),
        "Bazel exception manifest version must be 1"
    );
    assert_closed_exception_manifest(&manifest);

    let listed = exception_paths();
    for path in package_build_files() {
        let source = read_repo_file(&path);
        if source.contains("# keep") {
            assert!(
                listed.contains(&path),
                "hand-written BUILD exception is not listed: {path}"
            );
        }
    }
}

#[test]
fn generated_package_builds_use_canonical_labels() {
    for path in package_build_files() {
        let source = read_repo_file(&path);
        assert!(
            !source.contains("../"),
            "Bazel BUILD ownership must not use invalid up-level labels: {path}"
        );
    }
}

#[test]
fn gazelle_fixture_covers_ordinary_and_exceptional_ownership() {
    for path in [
        "BUILD.bazel",
        "Cargo.lock",
        "Cargo.toml",
        "MODULE.bazel",
        "src/exception.rs",
        "src/lib.rs",
        "src/main.rs",
    ] {
        assert!(
            repo_path_exists(&format!("{GAZELLE_FIXTURE_ROOT}/{path}")),
            "missing Gazelle fixture surface: {GAZELLE_FIXTURE_ROOT}/{path}"
        );
    }

    let module = read_repo_file(&format!("{GAZELLE_FIXTURE_ROOT}/MODULE.bazel"));
    for required in [
        "bazel_dep(name = \"gazelle\", version = \"0.47.0\")",
        "bazel_dep(name = \"gazelle_rust\", version = \"0.1.0\")",
        "crate.from_cargo(",
        "versions = [\"1.97.0\"]",
        "register_toolchains(\"@rust_toolchains//:all\")",
    ] {
        assert!(
            module.contains(required),
            "Gazelle fixture MODULE.bazel is missing {required}"
        );
    }

    let build = read_repo_file(&format!("{GAZELLE_FIXTURE_ROOT}/BUILD.bazel"));
    for required in [
        "gazelle_binary(",
        "languages = [\"@gazelle_rust//rust_language\"]",
        "gazelle(",
        "rust_library(",
        "rust_binary(",
        "@crates//:serde_json",
        "# keep",
        "name = \"explicit_exception\"",
    ] {
        assert!(
            build.contains(required),
            "Gazelle fixture BUILD.bazel is missing {required}"
        );
    }
    for forbidden in ["genrule(", "custom_gazelle", "postprocess", "overlay"] {
        assert!(
            !build.contains(forbidden),
            "Gazelle fixture must not contain repository-owned generation {forbidden}"
        );
    }
}

#[test]
fn production_bazel_graph_has_no_legacy_workspace_or_generator_escape_hatch() {
    assert!(
        !repo_path_exists("WORKSPACE"),
        "root WORKSPACE is forbidden"
    );
    assert!(
        !repo_path_exists("WORKSPACE.bazel"),
        "root WORKSPACE.bazel is forbidden"
    );
    assert_no_repository_owned_generation(&read_repo_file("MODULE.bazel"));
    assert_no_repository_owned_generation(&read_repo_file("BUILD.bazel"));
    for path in [
        "bazel/platforms/BUILD.bazel",
        "bazel/toolchains/BUILD.bazel",
        "bazel/exceptions/BUILD.bazel",
    ] {
        assert_no_repository_owned_generation(&read_repo_file(path));
    }
}

#[cfg(test)]
mod negative_fixtures {
    use super::{assert_closed_exception_manifest, assert_no_repository_owned_generation};

    #[test]
    #[should_panic(expected = "repository-owned generation escape hatch")]
    fn workspace_escape_hatch_is_rejected() {
        assert_no_repository_owned_generation("WORKSPACE");
    }

    #[test]
    #[should_panic(expected = "repository-owned generation escape hatch")]
    fn custom_generator_escape_hatch_is_rejected() {
        assert_no_repository_owned_generation("custom_generator");
    }

    #[test]
    #[should_panic(expected = "marker")]
    fn exception_without_keep_marker_is_rejected() {
        let manifest = serde_json::json!({
            "version": 1,
            "exceptions": [{
                "label": "//packages/example:exception",
                "path": "packages/example/BUILD.bazel",
                "reason": "fixture",
                "marker": "generated"
            }]
        });
        assert_closed_exception_manifest(&manifest);
    }
}
