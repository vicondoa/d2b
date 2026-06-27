//! Guest-control generated binding source policy lints migrated from the
//! retired guest binding bash gates. These are source greps over committed
//! generated bindings and crate manifests; generation determinism stays in
//! `tests/unit/gates/drift-check.sh`.

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `rg`'s
/// per-line evaluation for the retired shell gates.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

fn matching_lines(content: &str, pattern: &str) -> Vec<String> {
    let re = Regex::new(pattern).expect("valid regex");
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| re.is_match(line))
        .map(|(idx, line)| format!("{}:{line}", idx + 1))
        .collect()
}

fn assert_no_line_matches(content: &str, pattern: &str, context: &str) {
    if !any_line_matches(content, pattern) {
        return;
    }
    let matches = matching_lines(content, pattern);
    panic!(
        "{context}: forbidden pattern {pattern:?} matched:\n{}",
        matches.join("\n")
    );
}

#[test]
fn guest_proto_bindings_are_message_only_and_codegen_free() {
    let generated_file_rel = "packages/d2b-contracts/src/generated/guest_control.rs";
    let ipc_manifest_rel = "packages/d2b-contracts/Cargo.toml";
    let ipc_build_rs_rel = "packages/d2b-contracts/build.rs";

    assert!(
        repo_path_exists(generated_file_rel),
        "guest-proto-bindings: missing {generated_file_rel}"
    );
    assert!(
        repo_path_exists(ipc_manifest_rel),
        "guest-proto-bindings: missing {ipc_manifest_rel}"
    );

    let generated = read_repo_file(generated_file_rel);
    assert_no_line_matches(
        &generated,
        r"\bunsafe\b|allow\(unsafe_code\)|expect\(unsafe_code\)|allow\(clippy::all\)|allow\(unknown_lints\)",
        "guest-proto-bindings: generated bindings contain unsafe code or unsafe lint bypasses",
    );
    assert_no_line_matches(
        &generated,
        r"ttrpc|service GuestControl|GuestControl\\x12|Service|Client|Server|register_service|add_service|ServiceClient|ServiceServer",
        "guest-proto-bindings: generated guest-control bindings must stay message-only",
    );

    let ipc_manifest = read_repo_file(ipc_manifest_rel);
    assert_no_line_matches(
        &ipc_manifest,
        r"ttrpc",
        "guest-proto-bindings: d2b-contracts must not depend on ttrpc for message-only bindings",
    );
    assert!(
        !repo_path_exists(ipc_build_rs_rel),
        "guest-proto-bindings: d2b-contracts must not generate guest protobuf bindings during normal builds"
    );
    assert_no_line_matches(
        &ipc_manifest,
        r"^\[build-dependencies\]|protobuf-codegen|prost-build|tonic-build|\bprotoc\b",
        "guest-proto-bindings: d2b-contracts must keep protobuf code generation in xtask only",
    );
}

#[test]
fn guest_ttrpc_bindings_are_xtask_only_and_ipc_stays_ttrpc_free() {
    let generated_file_rel = "packages/d2b-guestd/src/generated/guest_control_ttrpc.rs";
    let guestd_manifest_rel = "packages/d2b-guestd/Cargo.toml";
    let guestd_build_rs_rel = "packages/d2b-guestd/build.rs";
    let ipc_manifest_rel = "packages/d2b-contracts/Cargo.toml";

    assert!(
        repo_path_exists(generated_file_rel),
        "guest-ttrpc-bindings: missing {generated_file_rel}"
    );
    assert!(
        repo_path_exists(guestd_manifest_rel),
        "guest-ttrpc-bindings: missing {guestd_manifest_rel}"
    );
    assert!(
        repo_path_exists(ipc_manifest_rel),
        "guest-ttrpc-bindings: missing {ipc_manifest_rel}"
    );

    let generated = read_repo_file(generated_file_rel);
    assert_no_line_matches(
        &generated,
        r"\bunsafe\b|allow\(unsafe_code\)|expect\(unsafe_code\)|allow\(clippy::all\)|allow\(unknown_lints\)|clipto_camel_casepy",
        "guest-ttrpc-bindings: generated service bindings contain unsafe code or broad lint bypasses",
    );
    assert!(
        !repo_path_exists(guestd_build_rs_rel),
        "guest-ttrpc-bindings: d2b-guestd must not generate ttRPC bindings during normal builds"
    );

    let guestd_manifest = read_repo_file(guestd_manifest_rel);
    assert_no_line_matches(
        &guestd_manifest,
        r"^\[build-dependencies\]|ttrpc-codegen|ttrpc-compiler|prost-build|\bprotoc\b",
        "guest-ttrpc-bindings: d2b-guestd must keep ttRPC code generation in xtask only",
    );

    let ipc_manifest = read_repo_file(ipc_manifest_rel);
    assert_no_line_matches(
        &ipc_manifest,
        r"ttrpc",
        "guest-ttrpc-bindings: d2b-contracts must remain message-only and ttrpc-free",
    );
}
