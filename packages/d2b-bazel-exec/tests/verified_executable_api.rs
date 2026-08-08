use std::{fs, path::PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source(relative: &str) -> String {
    fs::read_to_string(crate_root().join(relative)).expect("read owned source")
}

#[test]
fn verified_executable_has_no_production_mint_or_descriptor_surface() {
    let provider = source("src/provider.rs");
    assert!(provider.contains("pub struct VerifiedExecutable"));
    assert!(!provider.contains("pub fn new("));
    assert!(!provider.contains("pub fn path("));
    assert!(!provider.contains("pub fn as_raw_fd("));
    assert!(!provider.contains("pub fn as_fd("));
    assert!(!provider.contains("pub fn duplicate_for_mapping("));
    assert!(!provider.contains("impl Clone for VerifiedExecutable"));
    assert!(!provider.contains("impl Copy for VerifiedExecutable"));
    assert!(!provider.contains("impl std::fmt::Debug for VerifiedExecutable"));
    assert!(!provider.contains("impl std::fmt::Display for VerifiedExecutable"));
    assert!(!provider.contains("impl std::ops::Deref for VerifiedExecutable"));
    assert!(!provider.contains("impl From<"));
    assert!(!provider.contains("impl Default for VerifiedExecutable"));
    assert!(provider.contains("pub(crate) trait VerifiedExecutableMint"));
    assert!(!provider.contains("expected_digest"));
}

#[test]
fn rustdoc_compile_fail_examples_cover_the_downstream_seal() {
    let provider = source("src/provider.rs");
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
fn only_the_dependency_leaf_invokes_the_immutable_helper() {
    let execute = source("src/execute.rs");
    assert!(execute.contains("option_env!(\"D2B_BAZEL_EXEC_SUPERVISOR\")"));
    assert!(execute.contains("value.starts_with(\"/nix/store/\")"));
    assert!(!execute.contains("/nix/store/d2b-bazel-exec-supervisor/"));
    assert!(execute.contains("Command::new(helper)"));
    assert!(execute.contains("child_fd: PRIVATE_STATUS_FD"));
    assert!(execute.contains("child_fd: PRIVATE_EXECUTABLE_FD"));
    assert!(execute.contains("command.args(&plan.request.target_argv)"));
    assert!(execute.contains("read_status(status_reader)"));
    assert!(execute.contains("ensure_helper_status(status, terminal)"));
    assert!(!execute.contains("std::env::var"));
    assert!(!execute.contains("CARGO_BIN_EXE"));
    assert!(!execute.contains("TEST_SRCDIR"));
    assert!(!execute.contains("RUNFILES"));
    assert!(!execute.contains("worktree"));
}

#[test]
fn signal_handoff_restores_before_waiting_or_unlocking() {
    let execute = source("src/execute.rs");
    assert!(execute.contains("static PROCESS_LAUNCH_COORDINATOR"));
    assert!(execute.contains("thread_get_mask"));
    assert!(execute.contains("thread_block"));
    assert!(execute.contains("thread_set_mask"));
    assert!(execute.contains("let result = backend.spawn(plan)"));
    assert!(execute.contains("let restored = backend.restore_mask(snapshot)"));
    assert!(execute.contains("receipt.finish()"));
}

#[test]
fn production_backend_is_not_a_public_injection_trait() {
    let lib = source("src/lib.rs");
    let execute = source("src/execute.rs");
    assert!(!lib.contains("ExecutionBackend"));
    assert!(execute.contains("#[cfg(feature = \"test-support\")]"));
    assert!(execute.contains("pub trait ExecutionBackend"));
    assert!(!execute.contains("pub struct ProductionBackend"));
}

#[test]
fn no_first_party_unsafe_or_shelling_fixture_is_present() {
    for path in ["src/lib.rs", "src/provider.rs", "src/execute.rs"] {
        let text = source(path);
        assert!(!text.contains("unsafe {"));
        assert!(!text.contains("std::process::Command::new(\"cargo\")"));
        assert!(!text.contains("cargo build"));
    }
}
