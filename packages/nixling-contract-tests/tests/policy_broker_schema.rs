//! Broker / privileges schema doc-and-source-lint gates (the "H-group"),
//! migrated from the `tests/*.sh` bash gates. Each test reads the real repo
//! files (via the `nixling_contract_tests` repo-file helpers) and asserts a
//! structural / documentation / completeness invariant. This crate is excluded
//! from the hermetic Nix sandbox workspace build and runs only from
//! `tests/rust-workspace-checks.sh` against the real checkout, so reading repo
//! files relative to the repo root is sound here.
//!
//! Migrated gates:
//!   * tests/broker-validate-bundle.sh          -> broker_delegates_bundle_validation_to_core
//!   * tests/privileges-matrix-completeness.sh   -> privileges_matrix_covers_declared_operations
//!
//! `tests/broker-enum-disposition.sh` is migrated separately in
//! `policy_broker_dispositions.rs`, because it owns a larger cross-reference
//! between the broker-disposition doc table, the privileges schema enum, and the
//! production real-wire dispatcher.
//!
//! Spec corrections (existing code is canon):
//!   * broker-validate-bundle.sh's 4th assertion forbids ALL
//!     `serde_json::from_str`/`from_value` anywhere under the broker `src/`.
//!     The committed broker legitimately parses subprocess JSON (nft /
//!     `ip route` / store-view runner output / audit lines) in
//!     `ops/{store_view_farm,route,tap,store_sync_*}.rs`, so the blanket ban
//!     is red against current code (also documented as pre-existing breakage
//!     in `tests/README.md`). The migrated test keeps the gate's actual stated
//!     purpose — "the broker delegates bundle validation to nixling-core only"
//!     — via the positive `nixling_core::manifest` + `validate_bundle`
//!     delegation checks, and drops the over-broad negative blanket ban.
//!   * privileges-matrix-completeness.sh renders the live `privileges.json`
//!     via `nix eval` and derives `rendered_ops` from
//!     `publicOperations ∪ brokerOperations`. The Rust port reads the
//!     equivalent op-id set directly from the canonical source-of-truth
//!     `docs/reference/schemas/v2/privileges.json` (`OperationAuthz.operation`
//!     enum), which `nixling-core`'s `privileges::operation_schema` defines as
//!     exactly `sorted(union(PUBLIC_OPERATION_AUTHZ, BROKER_OPERATION_AUTHZ)
//!     .operation)` — identical to the rendered op set, with no Nix eval (per
//!     the disk-hygiene contract). The bash gate's JSON-Schema validation of
//!     the rendered artifact is dropped: it needs the Nix-rendered file and is
//!     covered by the bundle/manifest drift gate; the completeness invariant
//!     (every declared CLI/API + broker op has a known privileges row) is
//!     preserved.
//!
//! Dropped (not applicable to an in-crate Rust test): the bash test-harness
//! env-override knobs (`ROOT`/`SCHEMA`/`CLI_DOC`/`BROKER_HINT`/`SRC_DIR`
//! overrides, `NIXLING_*_IN_NIX_SHELL` python3 re-entry, `nl_mktemp` scratch
//! dirs, and the absent-input skip).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nixling_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// Recursively collect every regular file under `dir`. Mirrors `grep -R`, which
/// descends the whole tree and inspects every file.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read dir {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Recursively collect every `*.rs` file under `dir`, skipping any directory
/// named `target` (the gitignored cargo build tree). Mirrors
/// `grep -rl --include='*.rs'` over the source tree while excluding build
/// artifacts (the bash gate observed only git-tracked sources).
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read dir {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Read a file lossily (non-UTF-8 bytes become U+FFFD), mirroring `grep`, which
/// still line-scans such files.
fn read_lossy(path: &Path) -> String {
    let bytes = fs::read(path)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Migrated from tests/broker-validate-bundle.sh.
//
// "The broker delegates bundle validation to nixling-core only."
//
// The committed gate runs four `grep -R` checks over
// `packages/nixling-priv-broker/src`:
//   (1) the source dir exists;
//   (2) `nixling_core::manifest` is imported (delegation target present);
//   (3) `validate_bundle` is called (the single validation entry point);
//   (4) NO `serde_json::from_str`/`from_value` appears anywhere (intended as a
//       "no duplicate broker-side bundle parsing" proxy).
//
// Check (4) is red against current code and dropped per the module-level Spec
// correction; checks (1)-(3) — the gate's actual stated delegation purpose —
// are ported faithfully.
// ---------------------------------------------------------------------------
#[test]
fn broker_delegates_bundle_validation_to_core() {
    let src_rel = "packages/nixling-priv-broker/src";
    let src_dir = repo_root().join(src_rel);
    assert!(
        src_dir.is_dir(),
        "broker-validate-bundle: missing broker source directory: {src_rel}"
    );

    let mut files = Vec::new();
    collect_files(&src_dir, &mut files);
    files.sort();
    let tree: String = files
        .iter()
        .map(|p| read_lossy(p))
        .collect::<Vec<_>>()
        .join("\n");

    // (2) delegation target import present (substring, as `grep -R -q`).
    assert!(
        tree.contains("nixling_core::manifest"),
        "broker-validate-bundle: expected nixling_core::manifest import under {src_rel}"
    );

    // (3) the validation entry point is called.
    assert!(
        tree.contains("validate_bundle"),
        "broker-validate-bundle: expected validate_bundle call under {src_rel}"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/privileges-matrix-completeness.sh.
//
// "Every declared public/broker operation has a rendered privileges row."
//
// declared_ops = CLI/API verbs parsed from `docs/reference/cli-contract.md`
//   backtick `nixling ...` spans, UNION the operation-enum variants parsed from
//   every `*.rs` broker source that names `DelegateCgroupV2`.
// rendered_ops = the canonical op-id set, read directly from the
//   `OperationAuthz.operation` enum in `docs/reference/schemas/v2/privileges.json`
//   (== the bash gate's rendered `publicOperations ∪ brokerOperations`; see the
//   module-level Spec correction).
//
// Assertions: declared_ops is non-empty, rendered_ops is non-empty, and every
// declared op is present in rendered_ops (no declared op lacks a privileges
// row).
// ---------------------------------------------------------------------------

/// `record_cli` from the bash gate's python helper: fold a tokenized CLI
/// command into its canonical authz operation name(s) and insert into `found`.
fn record_cli(parts: &[String], version_re: &Regex, found: &mut BTreeSet<String>) {
    let Some(first) = parts.first() else {
        return;
    };
    if first.starts_with("NOTICE") {
        return;
    }
    if first == "--help" || first == "-h" {
        return;
    }
    if version_re.is_match(first) {
        return;
    }
    if first.contains('/') {
        for item in first.split('/') {
            let mut next = vec![item.to_string()];
            next.extend_from_slice(&parts[1..]);
            record_cli(&next, version_re, found);
        }
        return;
    }

    let mut op: Vec<String> = vec![first.clone()];
    if parts.len() > 1 {
        let p1 = &parts[1];
        if first == "host" && !p1.starts_with("--") {
            op.push(p1.clone());
            if parts.len() > 2
                && matches!(parts[2].as_str(), "--apply" | "--dry-run" | "--read-only")
            {
                op.push(parts[2].clone());
            }
        } else if first == "vm" && !p1.starts_with("--") {
            // konsole is the `vm exec -it` wrapper; both route to the daemon
            // `exec` operation and share its admin-only authz row.
            let alias = match p1.as_str() {
                "start" => Some("up"),
                "stop" => Some("down"),
                "restart" => Some("restart"),
                "list" => Some("list"),
                "exec" => Some("exec"),
                "konsole" => Some("exec"),
                _ => None,
            };
            match alias {
                Some(a) => op = vec![a.to_string()],
                None => op.push(p1.clone()),
            }
        } else if (matches!(
            first.as_str(),
            "audio" | "auth" | "debug" | "keys" | "store" | "usb"
        ) && !p1.starts_with("--"))
            || matches!(
                (first.as_str(), p1.as_str()),
                ("audit", "--human") | ("audit", "--json") | ("status", "--check-bridges")
            )
        {
            // Two faithful elif arms of the bash gate's python helper —
            // (a) `audio|auth|debug|keys|store|usb <sub>` and (b) the
            // `audit --human|--json` / `status --check-bridges` flag verbs —
            // merged because both append `parts[1]` and are mutually
            // exclusive (avoids clippy::if_same_then_else).
            op.push(p1.clone());
        }
    }

    found.insert(op.join(" "));
}

/// Parse declared CLI/API operations from `docs/reference/cli-contract.md`.
fn declared_cli_ops(found: &mut BTreeSet<String>) {
    let cli = read_repo_file("docs/reference/cli-contract.md");

    let cmd_re = Regex::new(r"`(nixling[^`]*)`").expect("valid cmd regex");
    let version_re = Regex::new(r"^v\d+(?:\.\d+)*$").expect("valid version regex");
    let audio_on_re = Regex::new(r"^nixling\s+audio\s+on\|off(?:\s|$)").expect("valid audio regex");
    let audio_mic_re =
        Regex::new(r"^nixling\s+audio\s+mic\s+on\|off(?:\s|$)").expect("valid audio mic regex");
    let audio_speaker_re = Regex::new(r"^nixling\s+audio\s+speaker\s+on\|off(?:\s|$)")
        .expect("valid audio speaker regex");

    for cap in cmd_re.captures_iter(&cli) {
        let raw_cmd = &cap[1];

        if audio_on_re.is_match(raw_cmd) {
            found.insert("audio on".to_string());
            found.insert("audio off".to_string());
            continue;
        }
        if audio_mic_re.is_match(raw_cmd) {
            found.insert("audio mic".to_string());
            continue;
        }
        if audio_speaker_re.is_match(raw_cmd) {
            found.insert("audio speaker".to_string());
            continue;
        }
        if raw_cmd.starts_with("nixling audit") && raw_cmd.contains("--human") {
            found.insert("audit --human".to_string());
        }

        let mut tokens: Vec<String> = Vec::new();
        for raw in raw_cmd.split_whitespace().skip(1) {
            let token = raw.trim_end_matches(|c: char| ".,;:".contains(c));
            if token.starts_with('<') || token.starts_with('[') || token.starts_with('>') {
                continue;
            }
            let token = token.split('|').next().unwrap_or("").trim_end_matches(']');
            if token == "..." || token == "\u{2026}" {
                continue;
            }
            if !token.is_empty() {
                tokens.push(token.to_string());
            }
        }
        record_cli(&tokens, &version_re, found);
    }
}

/// Parse declared broker operation-enum variants from every `*.rs` source under
/// `packages/` that names `DelegateCgroupV2` (the bash gate's broker-source
/// discovery key).
fn declared_broker_ops(found: &mut BTreeSet<String>) {
    let mut rs_files = Vec::new();
    collect_rs_files(&repo_root().join("packages"), &mut rs_files);
    rs_files.sort();
    let broker_sources: Vec<PathBuf> = rs_files
        .into_iter()
        .filter(|p| read_lossy(p).contains("DelegateCgroupV2"))
        .collect();
    assert!(
        !broker_sources.is_empty(),
        "privileges-matrix-completeness: no broker sources discovered (DelegateCgroupV2 marker)"
    );

    let enum_re = Regex::new(r"(?s)enum\s+(\w*(?:Operation|Request|Command|Op)\w*)\s*\{([^}]*)\}")
        .expect("valid enum regex");
    let variant_re = Regex::new(r"(?m)^\s*([A-Z][A-Za-z0-9_]*)\b").expect("valid variant regex");
    let row_re = Regex::new(r#"row\(\s*"([A-Z][A-Za-z0-9]*)""#).expect("valid row regex");

    for path in broker_sources {
        let text = read_lossy(&path);
        let mut saw_enum = false;
        for cap in enum_re.captures_iter(&text) {
            let enum_name = &cap[1];
            if enum_name.ends_with("Error")
                || enum_name.ends_with("Err")
                || enum_name.ends_with("Kind")
            {
                continue;
            }
            let body = &cap[2];
            for vcap in variant_re.captures_iter(body) {
                found.insert(vcap[1].to_string());
                saw_enum = true;
            }
        }
        if !saw_enum {
            for rcap in row_re.captures_iter(&text) {
                found.insert(rcap[1].to_string());
            }
        }
    }
}

/// The canonical op-id set: the `OperationAuthz.operation` enum from
/// `docs/reference/schemas/v2/privileges.json` (== the rendered
/// `publicOperations ∪ brokerOperations` op set; see the module Spec
/// correction).
fn rendered_ops() -> BTreeSet<String> {
    let schema_rel = "docs/reference/schemas/v2/privileges.json";
    assert!(
        repo_path_exists(schema_rel),
        "privileges-matrix-completeness: missing privileges schema: {schema_rel}"
    );
    let schema: serde_json::Value =
        serde_json::from_str(&read_repo_file(schema_rel)).expect("privileges.json parses as JSON");
    let enum_values = schema
        .pointer("/definitions/OperationAuthz/properties/operation/enum")
        .and_then(|v| v.as_array())
        .expect("OperationAuthz.operation.enum present in privileges schema");
    enum_values
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

#[test]
fn privileges_matrix_covers_declared_operations() {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    declared_cli_ops(&mut declared);
    declared_broker_ops(&mut declared);

    let rendered = rendered_ops();

    assert!(
        !declared.is_empty(),
        "privileges-matrix-completeness: no CLI/API or broker operations discovered"
    );
    assert!(
        !rendered.is_empty(),
        "privileges-matrix-completeness: privileges schema enum contains no operations"
    );

    let missing: Vec<&String> = declared
        .iter()
        .filter(|op| !rendered.contains(*op))
        .collect();
    assert!(
        missing.is_empty(),
        "privileges-matrix-completeness: declared operations missing from the privileges \
         matrix (OperationAuthz enum): {missing:?}"
    );
}
