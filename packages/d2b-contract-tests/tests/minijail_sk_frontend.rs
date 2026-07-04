//! Contract tests for the `security-key-frontend` minijail profile and the
//! `ProcessRole::SecurityKeyFrontend` DAG role.
//!
//! The sk-frontend is a **no-runner host tracking node**: d2bd watches the
//! vsock socket path for readiness but does not spawn a host-side process.
//! The actual CTAPHID relay binary runs inside the guest VM supervised by
//! the guest's systemd. Consequently:
//!
//!   * The minijail profile entry exists in `minijail-profiles.nix` for the
//!     `sk-frontend` node ID (every DAG node must have a profile entry, even
//!     no-runner nodes, so the `profileFor name id` lookup in
//!     `processes-json.nix::mkProcessNode` does not panic at eval time).
//!   * The profile carries `role = "security-key-frontend"`.
//!   * The profile carries a `seccompPolicyRef` (must be present; the
//!     framework's profile schema requires it).
//!   * The profile does NOT declare `capabilities =` — mkProfile defaults to
//!     `[]`, which is correct: the host daemon only watches a path for
//!     readiness and never touches a device.
//!   * `ProcessRole::SecurityKeyFrontend` exists as a Rust enum variant in
//!     `d2b-core` (compile-time check via `use`).
//!
//! These are always-on (no D2B_FIXTURES / D2B_FIXTURES_FULL needed) source-grep
//! checks, which run on any host with a Rust toolchain.

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use d2b_core::processes::ProcessRole;
use regex::Regex;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";

fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

fn extract_block(content: &str, start_pat: &str, end_pat: &str) -> Option<String> {
    let start_re = Regex::new(start_pat).expect("valid start regex");
    let end_re = Regex::new(end_pat).expect("valid end regex");
    let mut active = false;
    let mut block: Vec<&str> = Vec::new();
    for line in content.lines() {
        if !active && start_re.is_match(line) {
            active = true;
        }
        if active {
            block.push(line);
            if end_re.is_match(line) {
                return Some(block.join("\n"));
            }
        }
    }
    None
}

/// `ProcessRole::SecurityKeyFrontend` is a real Rust variant in `d2b-core`.
/// This test is a compile-time assertion: if the variant is removed or renamed,
/// this file will fail to compile.
#[test]
fn security_key_frontend_process_role_variant_exists() {
    // Compile-time: if this match arm cannot be constructed, the build fails.
    let role = ProcessRole::SecurityKeyFrontend;
    // Confirm the serde round-trip: the variant must serialize as the kebab-case
    // string that processes.json emits (set by `#[serde(rename_all = "kebab-case")]`
    // on the enum).
    let serialized = serde_json::to_string(&role).expect("serialize ProcessRole");
    assert_eq!(
        serialized, "\"security-key-frontend\"",
        "ProcessRole::SecurityKeyFrontend must serialize as \"security-key-frontend\""
    );
}

/// `minijail-profiles.nix` contains a profile block for the `sk-frontend`
/// DAG node, keyed by `profileIdFor name "sk-frontend"`.
#[test]
fn sk_frontend_profile_block_exists() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "minijail-profiles.nix not found at {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    assert!(
        any_line_matches(&src, r#"profileIdFor name "sk-frontend""#),
        "no sk-frontend profile block found in {MINIJAIL_PROFILES_NIX}; \
         every DAG node (even no-runner ones) must have a profile entry so \
         mkProcessNode's profileFor lookup succeeds at Nix eval time"
    );
}

/// The sk-frontend profile declares `role = "security-key-frontend"`.
#[test]
fn sk_frontend_profile_role_correct() {
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let block = extract_block(
        &src,
        r#"profileIdFor name "sk-frontend""#,
        r"^\s*};\s*$",
    )
    .expect("could not extract sk-frontend profile block from minijail-profiles.nix");

    assert!(
        any_line_matches(&block, r#"role\s*=\s*"security-key-frontend""#),
        "sk-frontend profile block does not declare role = \"security-key-frontend\"; \
         block content:\n{block}"
    );
}

/// The sk-frontend profile carries a `seccompPolicyRef` (required by the
/// framework's profile schema).
#[test]
fn sk_frontend_profile_has_seccomp_policy_ref() {
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let block = extract_block(
        &src,
        r#"profileIdFor name "sk-frontend""#,
        r"^\s*};\s*$",
    )
    .expect("could not extract sk-frontend profile block from minijail-profiles.nix");

    assert!(
        any_line_matches(&block, r#"seccompPolicyRef\s*="#),
        "sk-frontend profile block is missing seccompPolicyRef; \
         block content:\n{block}"
    );
}

/// The sk-frontend profile does NOT declare `capabilities =`.
/// The host daemon performs no privileged operations for a no-runner tracking
/// node; mkProfile's default `[]` is the correct capability set.
#[test]
fn sk_frontend_profile_no_host_capabilities() {
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let block = extract_block(
        &src,
        r#"profileIdFor name "sk-frontend""#,
        r"^\s*};\s*$",
    )
    .expect("could not extract sk-frontend profile block from minijail-profiles.nix");

    assert!(
        !any_line_matches(&block, r"capabilities\s*="),
        "sk-frontend profile unexpectedly declares host capabilities; \
         the node is a no-runner tracker — capabilities must remain at mkProfile \
         default []. block content:\n{block}"
    );
}
