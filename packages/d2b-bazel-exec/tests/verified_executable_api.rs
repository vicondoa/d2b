use std::{
    fs,
    path::{Path, PathBuf},
};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_source(relative: &str) -> String {
    fs::read_to_string(crate_root().join(relative)).expect("read owned source")
}

fn source_files() -> [(&'static str, String); 3] {
    [
        ("src/lib.rs", read_source("src/lib.rs")),
        ("src/provider.rs", read_source("src/provider.rs")),
        ("src/execute.rs", read_source("src/execute.rs")),
    ]
}

#[test]
fn verified_executable_is_a_closed_capability_root() {
    let provider = read_source("src/provider.rs");
    assert!(provider.contains("pub struct VerifiedExecutable"));
    assert!(!provider.contains("pub fn new("));
    assert!(!provider.contains("pub fn path("));
    assert!(!provider.contains("pub fn as_raw_fd("));
    assert!(!provider.contains("pub fn as_fd("));
    assert!(!provider.contains("impl Clone for VerifiedExecutable"));
    assert!(!provider.contains("impl Copy for VerifiedExecutable"));
    assert!(!provider.contains("impl std::fmt::Debug for VerifiedExecutable"));
    assert!(!provider.contains("impl std::fmt::Display for VerifiedExecutable"));
    assert!(!provider.contains("impl std::ops::Deref for VerifiedExecutable"));
    assert!(!provider.contains("impl From<"));
    assert!(!provider.contains("impl Default for VerifiedExecutable"));
    assert_eq!(
        provider.matches("impl VerifiedExecutable").count(),
        1,
        "the capability has one private implementation block and no trait allowlist"
    );
    let implementation = provider
        .split("impl VerifiedExecutable")
        .nth(1)
        .and_then(|rest| rest.split("/// Verify").next())
        .expect("implementation block");
    assert!(!implementation.contains("pub fn "));
    assert!(implementation.contains("pub(crate) fn "));
}

#[test]
fn rustdoc_compile_fail_examples_cover_the_downstream_seal() {
    let provider = read_source("src/provider.rs");
    assert!(provider.matches("```compile_fail").count() >= 5);
    for required in [
        "VerifiedExecutable::new",
        "as_raw_fd",
        "Borrow",
        "AsFd",
        "format!",
        "Display",
        "clone",
        "Default",
        "serde::Serialize",
        "PathBuf",
        "Deref",
    ] {
        assert!(
            provider.contains(required),
            "missing focused compile-fail example for {required}"
        );
    }
}

#[test]
fn one_dependency_leaf_owns_the_only_consuming_api_and_safe_mapping() {
    let provider = read_source("src/provider.rs");
    let execute = read_source("src/execute.rs");
    assert!(provider.contains("pub struct VerifiedExecutable"));
    assert_eq!(
        execute.matches("pub fn execute_verified").count(),
        1,
        "the capability has exactly one public consuming function"
    );
    assert!(execute.contains("VerifiedExecutable"));
    assert!(execute.contains("command_fds::{CommandFdExt, FdMapping}"));
    assert!(execute.contains("fd_mappings"));
    assert!(execute.contains("PRIVATE_EXECUTABLE_FD"));
    assert!(execute.contains("preserves_standard_streams"));
    assert!(execute.contains("IMMUTABLE_SUPERVISOR_PATH"));
    assert!(!execute.contains("RUNFILES"));
    assert!(!execute.contains("CARGO_BIN_EXE"));
    assert!(!execute.contains("TEST_SRCDIR"));
    assert!(!execute.contains("worktree"));
}

#[test]
fn reviewed_dependencies_and_invocation_policy_are_pinned() {
    let manifest = fs::read_to_string(crate_root().join("Cargo.toml")).expect("manifest");
    assert!(manifest.contains("command-fds = { workspace = true }"));
    assert!(manifest.contains("nix = { version = \"=0.29.0\""));
    assert!(manifest.contains("\"signal\""));
    assert!(manifest.contains("\"fs\""));
    assert!(manifest.contains("\"process\""));
    let execute = read_source("src/execute.rs");
    assert_eq!(
        execute
            .matches("Command::new(IMMUTABLE_SUPERVISOR_PATH)")
            .count(),
        1
    );
    assert!(
        !execute.contains("Command::new(")
            || execute.contains("Command::new(IMMUTABLE_SUPERVISOR_PATH)")
    );
    assert!(!execute.contains("std::env::var"));
    assert!(!execute.contains("set_var"));
}

#[test]
fn signal_handoff_has_one_process_wide_guard_and_safe_mask_api() {
    let execute = read_source("src/execute.rs");
    assert!(execute.contains("static PROCESS_LAUNCH_COORDINATOR"));
    assert!(execute.contains("thread_get_mask"));
    assert!(execute.contains("thread_block"));
    assert!(execute.contains("thread_set_mask"));
    assert!(execute.contains("let result = spawn()"));
    assert!(execute.contains("let restored = backend.restore_mask(snapshot)"));
    assert!(execute.contains("HandoffError::GuardPoisoned"));
    for forbidden in [
        concat!("pre", "_exec"),
        concat!("sig", "action"),
        concat!("signal", " ", "disposition"),
        concat!("raw", " ", "fork"),
    ] {
        assert!(
            !execute.contains(forbidden),
            "forbidden first-party process primitive {forbidden}"
        );
    }
}

#[test]
fn no_first_party_unsafe_or_cargo_shelling_fixture_is_present() {
    for (path, source) in source_files() {
        let unsafe_block = format!("{} {{", "unsafe");
        let unsafe_allow = format!("{}_code = \"{}\"", "unsafe", "allow");
        assert!(
            !source.contains(&unsafe_block) && !source.contains(&unsafe_allow),
            "{path} must remain safe Rust"
        );
        assert!(!source.contains("std::process::Command::new(\"cargo\")"));
        assert!(!source.contains("cargo build"));
    }
    let tests_dir = crate_root().join("tests");
    let entries = fs::read_dir(tests_dir).expect("test directory");
    assert!(
        entries.filter_map(Result::ok).all(|entry| entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "rs")),
        "the execution crate contains no shelling fixture"
    );
}

fn collect_governed_files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == ".git" || name == "target" || name == ".scratch")
        {
            continue;
        }
        if path.is_dir() {
            collect_governed_files(&path, output);
            continue;
        }
        let governed = path.file_name().is_some_and(|name| name == "Makefile")
            || matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("rs" | "bzl" | "sh" | "yml" | "yaml" | "nix" | "toml")
            );
        if governed {
            output.push(path);
        }
    }
}

#[test]
fn helper_invocation_site_is_closed_to_one_typed_rust_consumer() {
    let mut files = Vec::new();
    collect_governed_files(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root"),
        &mut files,
    );
    let invocation = "Command::new(IMMUTABLE_SUPERVISOR_PATH)";
    let mut sites = Vec::new();
    for path in files {
        if path.ends_with("tests/verified_executable_api.rs") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        if source.contains(invocation) {
            sites.push(path);
        }
        if source.contains("d2b-bazel-exec-supervisor")
            && (source.contains("RUNFILES_DIR")
                || source.contains("TEST_SRCDIR")
                || source.contains("CARGO_BIN_EXE")
                || source.contains("worktree"))
        {
            panic!("helper identity must not be routed through a path fallback");
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "helper invocation site is not closed: {sites:?}"
    );
    assert!(sites[0].ends_with("d2b-bazel-exec/src/execute.rs"));
}

#[test]
fn owned_api_has_no_public_descriptor_or_conversion_text() {
    let provider = read_source("src/provider.rs");
    let public_lines = provider
        .lines()
        .filter(|line| line.trim_start().starts_with("pub "))
        .collect::<Vec<_>>();
    assert!(
        public_lines
            .iter()
            .all(|line| !line.contains("descriptor") && !line.contains("path"))
    );
    assert!(!provider.contains("impl serde::Serialize"));
    assert!(!provider.contains("impl serde::Deserialize"));
    let _ = Path::new("src/provider.rs");
}
