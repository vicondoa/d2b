#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use serde_json::Value;

const EXCEPTION_MANIFEST: &str = "bazel/exceptions/manifest.json";
const PRODUCT_BUILD_ROOT: &str = "packages";
const STANDARD_CARGO_SOURCE_GLOBS: [&str; 7] = [
    "\"Cargo.lock\"",
    "\"Cargo.toml\"",
    "\"benches/**/*.rs\"",
    "\"build.rs\"",
    "\"examples/**/*.rs\"",
    "\"src/**/*.rs\"",
    "\"tests/**/*.rs\"",
];

fn git_listed_files(roots: &[&str]) -> Vec<String> {
    if std::env::var_os("TEST_SRCDIR").is_some()
        || std::env::var_os("RUNFILES_DIR").is_some()
        || std::env::var_os("D2B_REPO_ROOT").is_some()
    {
        let repo = repo_root();
        let mut files = Vec::new();
        for root in roots {
            let path = repo.join(root);
            collect_files(&repo, &path, &mut files);
        }
        files.sort();
        return files;
    }
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

fn collect_files(repo: &Path, path: &Path, files: &mut Vec<String>) {
    if path.is_file() {
        files.push(
            path.strip_prefix(repo)
                .expect("repository file is below repository root")
                .to_string_lossy()
                .into_owned(),
        );
        return;
    }
    let mut entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read repository directory {}: {error}", path.display()))
        .map(|entry| entry.expect("read repository directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        collect_files(repo, &entry, files);
    }
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
        module.matches("cargo_lock = \"//:Cargo.lock\"").count(),
        1,
        "the product graph must have exactly one root rules_rs Cargo authority"
    );
    assert!(
        module.contains("cargo_lock = \"//:Cargo.lock\""),
        "rules_rs must resolve the repository-root Cargo.lock"
    );
    assert!(
        module.contains("cargo_toml = \"//:Cargo.toml\""),
        "rules_rs must resolve the repository-root Cargo.toml"
    );
    for required in [
        "use_extension(\"@rules_rs//rs:rules_rust.bzl\", \"rules_rust\")",
        "use_repo(rules_rust, \"rules_rust\")",
        "use_extension(\"@rules_rs//rs/toolchains:module_extension.bzl\", \"toolchains\")",
        "use_repo(toolchains, \"default_rust_toolchains\")",
        "use_extension(\"@rules_rs//rs:extensions.bzl\", \"crate\")",
        "use_repo(crate, \"crates\")",
    ] {
        assert!(
            module.contains(required),
            "Bzlmod is missing rules_rs helper {required}"
        );
    }
    assert!(
        !module.contains("crate.spec("),
        "Bazel dependency inputs must not splice crate.spec entries into Cargo.lock"
    );
    assert!(
        !module.contains("Cargo.guest.lock")
            && !module.contains("tests/fixtures/bazel/compat/Cargo.lock"),
        "the product crate_universe must not absorb reduced guest or compatibility locks"
    );
    assert!(
        module.contains("toolchains.toolchain(")
            && module.contains("edition = \"2024\"")
            && module.contains("version = \"1.97.0\""),
        "Bzlmod must pin the Rust toolchain to rust-toolchain.toml"
    );
    assert!(
        module.contains("register_toolchains(")
            && module.contains("@default_rust_toolchains//...")
            && module.contains("@llvm//toolchain:all"),
        "Bzlmod must register the pinned Rust toolchain repository"
    );

    let lock = read_repo_file("MODULE.bazel.lock");
    assert!(
        lock.contains("\"lockFileVersion\"")
            && lock.contains("@@rules_rs+//rs:extensions.bzl%crate")
            && lock.contains("@@rules_rs+//rs/toolchains:module_extension.bzl%toolchains"),
        "Bzlmod lock must bind the rules_rs extensions"
    );
    assert!(
        !read_repo_file("Cargo.lock").contains("direct-cargo-bazel-deps"),
        "root Cargo.lock must not contain crate_universe synthetic direct dependencies"
    );
    assert!(
        !lock.contains("bazel/checks/rust/Cargo.lock")
            && !lock.contains("Cargo.guest.lock")
            && !lock.contains("tests/fixtures/bazel/compat/Cargo.lock"),
        "Bzlmod lock must not bind a second product Cargo authority"
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
    assert!(
        !bazelrc.lines().any(|line| {
            matches!(
                line.trim(),
                "common --action_env=PATH" | "common --test_env=PATH"
            )
        }),
        "Bazel must not vary action keys with the caller PATH"
    );

    let gitignore = read_repo_file(".gitignore");
    assert!(
        gitignore.lines().any(|line| line.trim() == "/bazel-*"),
        "repository policy must ignore Bazel workspace output links"
    );
    let bazelignore = read_repo_file(".bazelignore");
    assert!(
        bazelignore.lines().any(|line| line.trim() == ".scratch"),
        "Bazel must ignore local scratch output trees during workspace queries"
    );

    let root_build = read_repo_file("BUILD.bazel");
    assert!(
        !root_build.contains("gazelle") && !root_build.contains("# gazelle:"),
        "the root BUILD graph must not retain Gazelle targets or directives"
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
fn guest_libshpool_feature_boundary_is_preserved_in_cargo_and_bazel() {
    let cargo = read_repo_file("packages/d2b-guest-shell-runner/Cargo.toml");
    assert!(
        cargo.contains("real-libshpool = [\"dep:libshpool\"]"),
        "guest Cargo feature must own the libshpool dependency"
    );
    assert!(
        cargo.contains("libshpool = { version = \"0.11.0\", optional = true }"),
        "guest libshpool dependency must remain optional"
    );

    let build = read_repo_file("packages/d2b-guest-shell-runner/BUILD.bazel");
    assert!(
        build.contains("\"//bazel/checks/rust:guest-real-libshpool\": [")
            && build.contains("@crates//:libshpool"),
        "Bazel real-libshpool context must provide libshpool"
    );
    for required in [
        "@crates//:anyhow",
        "@crates//:clap",
        "@crates//:anyhow",
        "@crates//:clap",
    ] {
        assert!(
            build.contains(required),
            "guest Bazel feature contexts must declare {required}"
        );
    }
    assert!(
        build.contains("\"//conditions:default\": ["),
        "Bazel guest target must have an explicit default dependency context"
    );
    assert!(
        !build.contains("@bazel_only_crates"),
        "Bazel guest feature contexts must use the root Cargo crate authority"
    );
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
        "@default_rust_toolchains//rustc:default_linux_x86_64_1_97_0_rust_toolchain",
        "@default_rust_toolchains//rustc:default_linux_aarch64_1_97_0_rust_toolchain",
    ] {
        assert!(
            toolchains.contains(required),
            "toolchain package is missing {required}"
        );
    }
}

#[test]
fn every_root_product_member_has_explicit_build_ownership() {
    let members = root_workspace_members();
    let builds = package_build_files();
    for member in members {
        let build = format!("{member}/BUILD.bazel");
        assert!(
            builds.contains(&build),
            "root Cargo member has no Bazel BUILD ownership: {member}"
        );
    }
}

#[test]
fn every_root_product_member_uses_standard_cargo_source_globs() {
    for member in root_workspace_members() {
        let build = format!("{member}/BUILD.bazel");
        let source = read_repo_file(&build);
        assert!(
            source.contains("name = \"cargo_workspace_sources\"")
                && source.contains("srcs = glob(")
                && source.contains("allow_empty = True"),
            "first-party BUILD must declare the standard Cargo source filegroup: {build}"
        );
        for required in STANDARD_CARGO_SOURCE_GLOBS {
            assert!(
                source.contains(required),
                "first-party BUILD is missing standard source glob {required}: {build}"
            );
        }
    }
}

#[test]
fn exception_manifest_is_closed() {
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
                "hand-written BUILD exception marker is not listed: {path}"
            );
        }
    }
    for exception in listed {
        assert!(
            exception.starts_with("packages/") || exception.starts_with("bazel/"),
            "Bazel exception escapes the product graph: {exception}"
        );
    }
}

#[test]
fn package_builds_use_canonical_labels() {
    for path in package_build_files() {
        let source = read_repo_file(&path);
        assert!(
            !source.contains("../"),
            "Bazel BUILD ownership must not use invalid up-level labels: {path}"
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
