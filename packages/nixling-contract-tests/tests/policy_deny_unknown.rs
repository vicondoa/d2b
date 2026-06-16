//! Schema-strictness source-lint gate (the "H-group"), migrated from the
//! `tests/static-invariant-deny-unknown-fields-w3.sh` bash gate. The test reads
//! the real `nixling-core` host-schema sources and asserts every
//! security-sensitive host / host_w3 DTO carries
//! `#[serde(deny_unknown_fields)]`, so a regression that silently drops the
//! attribute from a DTO fails the static gate.
//!
//! This crate runs only from `tests/rust-workspace-checks.sh` against the real
//! checkout (it is excluded from the hermetic Nix sandbox workspace build), so
//! repo-file access via the `nixling_contract_tests` helpers is sound here.
//!
//! Migrated gates:
//!   * tests/static-invariant-deny-unknown-fields-w3.sh -> host_schema_dtos_carry_deny_unknown_fields
//!
//! NOT migrated (reclassified): `tests/static-invariant-deny-unknown-fields.sh`
//! is NOT a static source/schema grep gate. It shells out to
//! `nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])"` to perform full
//! JSON Schema Draft 2020-12 validation (instance synthesis, `$ref`/`oneOf`/
//! `anyOf`/`allOf` resolution, plus guest-control string/chunk/terminal bounds)
//! against committed fixtures under `tests/fixtures/deny-unknown`. It needs
//! nix + python + the `jsonschema` validator at runtime, so it cannot be
//! faithfully reproduced as a static-lint test in this crate and stays a bash
//! gate.

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// The security-sensitive broker / schema-drift DTOs that MUST carry
/// `#[serde(deny_unknown_fields)]`. Membership and order mirror the bash gate's
/// `W3_DTOS` array exactly.
const W3_DTOS: &[&str] = &[
    // host_w3.rs — security-sensitive broker and schema drift types:
    "IfNameMapping",
    "BridgePortFlagsW3",
    "KernelModuleEntry",
    "RouteIntent",
    "SysctlIntent",
    "HostsEntry",
    "NmUnmanagedEntry",
    "FirewallCoexistencePolicy",
    // host.rs — additions to HostJson:
    "HostChConfig",
];

/// Whether `content` declares the struct, mirroring `grep -qE "^pub struct
/// $dto\b"`: some line must begin with `pub struct <dto>` at a word boundary.
fn file_declares_struct(content: &str, dto: &str) -> bool {
    let re = Regex::new(&format!(r"^pub struct {}\b", regex::escape(dto)))
        .expect("valid struct-decl regex");
    content.lines().any(|line| re.is_match(line))
}

/// 0-based line index of the `pub struct <dto>` declaration, mirroring the awk
/// match `/^pub struct/ && $0 ~ "pub struct <dto>"` (the first line that begins
/// with `pub struct` and contains `pub struct <dto>`).
fn struct_decl_index(content: &str, dto: &str) -> Option<usize> {
    let needle = format!("pub struct {dto}");
    content
        .lines()
        .position(|line| line.starts_with("pub struct") && line.contains(&needle))
}

/// Whether any of the (up to) 10 lines immediately preceding the struct
/// declaration carries the `deny_unknown_fields` attribute. This reproduces the
/// awk ring buffer `lines[NR % 10]` that retains the last 10 lines read before
/// the `exit` on the struct declaration: for a declaration at index `decl_idx`
/// those are indices `decl_idx-10 ..= decl_idx-1` (fewer near the top of file).
/// It is tolerant of the canonical multi-line
/// `#[serde(rename_all = "...", deny_unknown_fields)]` form.
fn has_deny_attr_within_10_preceding(content: &str, decl_idx: usize) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let start = decl_idx.saturating_sub(10);
    lines[start..decl_idx]
        .iter()
        .any(|line| line.contains("deny_unknown_fields"))
}

// ---------------------------------------------------------------------------
// Migrated from tests/static-invariant-deny-unknown-fields-w3.sh.
//
// Every security-sensitive DTO under `nixling-core::host_w3` (plus the
// `HostChConfig` addition to `HostJson` in host.rs) must carry
// `#[serde(deny_unknown_fields)]`, per the AGENTS.md "Manifest bundle" policy
// and schema drift rules. For each named struct the gate selects the source
// file that declares it (host_w3.rs first, then host.rs) and asserts the
// attribute appears within the 10 lines immediately preceding the struct
// declaration.
// ---------------------------------------------------------------------------
#[test]
fn host_schema_dtos_carry_deny_unknown_fields() {
    let host_w3_rel = "packages/nixling-core/src/host_w3.rs";
    let host_rel = "packages/nixling-core/src/host.rs";

    // The bash gate skipped (exit 0) when either source was absent. In this
    // canon-asserting crate we hard-assert their presence instead: their
    // absence is a real regression, matching the template's hard-assert style.
    assert!(
        repo_path_exists(host_w3_rel),
        "host-schema deny-unknown-fields: missing {host_w3_rel}"
    );
    assert!(
        repo_path_exists(host_rel),
        "host-schema deny-unknown-fields: missing {host_rel}"
    );

    let host_w3 = read_repo_file(host_w3_rel);
    let host = read_repo_file(host_rel);

    for dto in W3_DTOS {
        // File selection mirrors `grep -qE "^pub struct $dto\b"`: host_w3.rs
        // first, then host.rs.
        let (src, src_rel) = if file_declares_struct(&host_w3, dto) {
            (&host_w3, host_w3_rel)
        } else if file_declares_struct(&host, dto) {
            (&host, host_rel)
        } else {
            panic!("host schema DTO '{dto}' not found in host_w3.rs nor host.rs");
        };

        let decl_idx = struct_decl_index(src, dto).unwrap_or_else(|| {
            panic!("host schema DTO '{dto}' declaration line not found in {src_rel}")
        });

        assert!(
            has_deny_attr_within_10_preceding(src, decl_idx),
            "host schema DTO '{dto}' is missing #[serde(deny_unknown_fields)] \
             (no deny_unknown_fields attribute within the 10 lines preceding the struct \
             declaration in {src_rel})"
        );
    }
}
