//! Test taxonomy and gate-placement policy.
//!
//! Layer-1 coverage belongs to the repository's closed Rust/Nix taxonomy.
//! Top-level shell files are only orchestrators or explicitly documented
//! Layer-2/manual entrypoints; a new ad-hoc shell gate must fail closed.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};

const NON_LAYER1_SHELLS: &[&str] = &[
    "audio.sh",
    "audit-forwarding.sh",
    "d2b-store.sh",
    "hardware-smoke-gpu-yubikey.sh",
    "live-vm-smoke.sh",
    "network-isolation.sh",
    "runner.sh",
    "static-timing.sh",
    "swtpm-persistence-smoke.sh",
    "test-drift.sh",
    "test-flake-list.sh",
    "test-flake.sh",
    "test-hardware.sh",
    "test-host-integration.sh",
    "test-integration.sh",
    "test-lint.sh",
    "test-nix-unit.sh",
    "test-policy.sh",
    "test-proofs.sh",
    "test-rust.sh",
];

fn top_level_executable_shells() -> Vec<String> {
    fs::read_dir(repo_root().join("tests"))
        .expect("read tests directory")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            (file_type.is_file()
                && name.ends_with(".sh")
                && is_executable(entry.path().metadata().ok()?.permissions().mode()))
            .then_some(name)
        })
        .collect()
}

#[cfg(unix)]
fn is_executable(mode: u32) -> bool {
    mode & 0o100 != 0
}

#[cfg(not(unix))]
fn is_executable(_mode: u32) -> bool {
    true
}

fn shell_is_taxonomy_member(name: &str, static_source: &str) -> bool {
    name == "lib.sh"
        || name == "static.sh"
        || NON_LAYER1_SHELLS.contains(&name)
        || static_source.contains(&format!("tests/{name}"))
        || static_source.contains(&format!("\"{name}\""))
}

#[test]
fn repository_test_taxonomy_forbids_new_ad_hoc_top_level_shell_gates() {
    let agents = read_repo_file("tests/AGENTS.md");
    assert!(agents.contains("New coverage MUST land as a Layer-1 test"));
    assert!(agents.contains("closed set") && agents.contains("a new `tests/*.sh`"));
    assert!(repo_path_exists("tests/unit/meta/BUILD.bazel"));

    let static_source = read_repo_file("tests/static.sh");
    let unknown = top_level_executable_shells()
        .into_iter()
        .filter(|name| !shell_is_taxonomy_member(name, &static_source))
        .collect::<Vec<_>>();
    assert!(
        unknown.is_empty(),
        "top-level executable shell gate is outside the closed test taxonomy: {unknown:?}"
    );
}

#[test]
fn synthetic_unregistered_shell_gate_is_rejected() {
    let static_source = "bash tests/static.sh\n";
    assert!(!shell_is_taxonomy_member(
        "new-security-gate.sh",
        static_source
    ));
    assert!(shell_is_taxonomy_member("test-policy.sh", static_source));
}
