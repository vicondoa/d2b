//! Policy/source-lint gates over the `nixos-modules/` tree and the Rust
//! workspace dependency graph (the "H-group"), migrated from the
//! `tests/*.sh` bash gates. Each test reads the real repo files and asserts a
//! structural/source invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access — and
//! shelling out to `git` for the gitignore-respecting file enumeration that the
//! bash gates got from `rg` — is sound here.
//!
//! Migrated gates:
//!   * tests/legacy-group-name-denylist.sh    -> legacy_group_name_denylist
//!   * tests/vm-submodule-cutover-eval.sh      -> vm_submodule_cutover
//!   * tests/static-rust-dependency-direction.sh -> static_rust_dependency_direction

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::repo_root;
use regex::Regex;

/// Read a repo-relative file, returning `None` when the path is absent or not
/// valid UTF-8 (binary files are skipped, mirroring `rg`/`grep -I`).
fn read_repo_file_opt(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join(rel)).ok()
}

/// Enumerate repo-relative tracked + untracked-non-ignored files under the
/// given pathspecs via `git ls-files`. This mirrors `rg`'s default behaviour
/// (respects `.gitignore`, so build artifacts under `target/` and Nix `result`
/// symlinks are excluded) that the original bash denylist gate relied on.
fn git_listed_files(roots: &[&str]) -> Vec<String> {
    let root = repo_root();
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("-c")
        .arg("core.quotePath=false")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots)
        .output()
        .expect("run `git ls-files`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut files: BTreeSet<String> = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.is_empty() {
            files.insert(line.to_string());
        }
    }
    files.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Migrated from tests/legacy-group-name-denylist.sh.
//
// Asserts no live references to the legacy `nixling-launcher{,s}` group names
// remain in source under `nixos-modules`, `packages`, and `tests`. The
// allowlist is matched against full `path:lineno:content` lines (anchored
// `^...$`), NOT as a substring — ported verbatim from the bash gate, with one
// addition: this Rust port file is self-allowlisted, exactly as the bash gate
// allowlisted itself (the denylist patterns it carries literally contain the
// legacy names).
// ---------------------------------------------------------------------------
#[test]
fn legacy_group_name_denylist() {
    let search = legacy_group_search();
    let allowlist = legacy_group_allowlist();

    let mut violations: Vec<String> = Vec::new();
    for rel in git_listed_files(&["nixos-modules", "packages", "tests"]) {
        let Some(content) = read_repo_file_opt(&rel) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if !search.is_match(line) {
                continue;
            }
            let candidate = format!("{rel}:{}:{line}", idx + 1);
            if !allowlist.is_match(&candidate) {
                violations.push(candidate);
            }
        }
    }

    assert!(
        violations.is_empty(),
        "legacy nixling-launcher{{,s}} references found:\n{}",
        violations.join("\n")
    );
}

/// Negative coverage migrated from the retired
/// `tests/legacy-group-name-denylist-self-test.sh`: a forbidden
/// `nixling-launcher` reference in a non-allowlisted source path must be
/// flagged, while an allowlisted (migration-tombstone) reference must not.
#[test]
fn legacy_group_name_denylist_rejects_forbidden_line() {
    let search = legacy_group_search();
    let allowlist = legacy_group_allowlist();

    let forbidden = "packages/forbidden.rs:1:const BAD: &str = \"nixling-launcher\";";
    assert!(
        search.is_match(forbidden),
        "search must match the forbidden line"
    );
    assert!(
        !allowlist.is_match(forbidden),
        "a forbidden nixling-launcher reference in a non-allowlisted path must be flagged"
    );

    let allowed = "nixos-modules/host-users.nix:42:    nixling-launcher = { };";
    assert!(
        allowlist.is_match(allowed),
        "the host-users.nix migration-tombstone line must stay allowlisted"
    );
}

/// The line-matching search for legacy group names (`nixling-launcher{,s}`).
fn legacy_group_search() -> Regex {
    Regex::new(r"nixling-launcher(s)?").expect("valid search regex")
}

/// Full-line (`^...$`-anchored, matched against `path:lineno:content`) allowlist
/// of permitted legacy-group-name references — a verbatim port of the bash
/// gate's `allowlist=(...)` array.
fn legacy_group_allowlist() -> Regex {
    let allowlist_patterns = [
        r"nixos-modules/host-activation\.nix:[0-9]+:[[:space:]]*(legacyLauncherGid|legacyLaunchersGid|getent group|for legacy_name in nixling-launcher nixling-launchers; do).*",
        r"nixos-modules/host-activation-helper/.*",
        r"packages/nixling-host-activation-helper/.*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# nixling-launcher\{,s\} → nixling rename\. No module references the[[:space:]]*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*nixling-launcher = \{ \};[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# nixling-launcher\{,s\} → nixling rename\. No module references the[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*users\.groups\.nixling-launchers = \{ \};[[:space:]]*",
        r"packages/nixling-core/src/privileges\.rs:[0-9]+:.*nixling-launcher.*",
        r"packages/nixling-ipc/src/broker_wire\.rs:[0-9]+:.*nixling-launcher.*",
        r"packages/nixling-priv-broker/src/bootstrap\.rs:[0-9]+:.*nixling-launcher.*",
        r"nixos-modules/privileges-json\.nix:[0-9]+:.*nixling-launcher.*",
        r"tests/legacy-group-name-denylist(-self-test)?\.sh:[0-9]+:.*",
        r"tests/group-rename-semantic-eval\.sh:[0-9]+:.*",
        // Migration bookkeeping (the ledger + per-script retirement records)
        // legitimately *describes* the retired legacy-group-name gates and their
        // successors; it is not live config usage, so exempt it (this is also
        // future-proof against other legacy-name gate retirements).
        r"tests/migration-ledger\.toml:[0-9]+:.*",
        r"tests/migration-state\.d/.*:[0-9]+:.*",
        // This Rust port carries the denylist patterns (which literally contain
        // the legacy group names) and replaces the bash gate; self-allowlist it
        // exactly as the bash gate self-allowlisted `legacy-group-name-denylist.sh`.
        r"packages/nixling-contract-tests/tests/policy_modules\.rs:[0-9]+:.*",
    ];
    Regex::new(&format!("^({})$", allowlist_patterns.join("|"))).expect("valid allowlist regex")
}

// ---------------------------------------------------------------------------
// Migrated from tests/vm-submodule-cutover-eval.sh.
//
// Asserts no production consumer in `nixos-modules/` reads
// `config.microvm.vms.${name}.config.config.*` directly — every consumer routes
// through the nixling-owned helpers `nl.vmRunner` / `nl.vmToplevel` /
// `nl.vmDeclaredRunner` in `nixos-modules/lib.nix`. `lib.nix`, `host.nix`, and
// `vm-submodule.nix` are the substrate-side authors and are EXEMPT.
// ---------------------------------------------------------------------------
#[test]
fn vm_submodule_cutover() {
    let pattern = Regex::new(r"config\.microvm\.vms\.\$\{[^}]*\}\.config\.config")
        .expect("valid cutover regex");
    let exempt: BTreeSet<&str> = [
        "nixos-modules/lib.nix",
        "nixos-modules/host.nix",
        "nixos-modules/vm-submodule.nix",
    ]
    .into_iter()
    .collect();

    let mut violations: Vec<String> = Vec::new();
    for rel in git_listed_files(&["nixos-modules"]) {
        if exempt.contains(rel.as_str()) {
            continue;
        }
        let Some(content) = read_repo_file_opt(&rel) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if pattern.is_match(line) {
                violations.push(format!("{rel}:{}:{line}", idx + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "vm-submodule-cutover: production consumers must route through \
         nl.vmRunner/vmToplevel/vmDeclaredRunner, found direct \
         config.microvm.vms.${{...}}.config.config reads:\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/static-rust-dependency-direction.sh.
//
// The Rust workspace dependency graph flows one way: ipc/core are leaves;
// host depends on core+ipc; the binaries (nixling, nixlingd) and the privileged
// broker (nixling-priv-broker, a sibling workspace) sit above. The broker must
// NOT depend on nixlingd/nixling; the CLI/daemon must NOT depend on the broker.
// This is a pure static parse of the `Cargo.toml` files. It also asserts the
// CLI and daemon actually import `nixling_ipc` from their source trees.
// ---------------------------------------------------------------------------
#[test]
fn static_rust_dependency_direction() {
    // (crate, allowed in-workspace deps) — verbatim port of the bash WANT map.
    let want: &[(&str, &[&str])] = &[
        ("nixling-core", &[]),
        ("nixling-ipc", &["nixling-core"]),
        ("nixling-host", &["nixling-core", "nixling-ipc"]),
        (
            "xtask",
            &["nixling-core", "nixling-ipc", "nixling", "nixlingd"],
        ),
        ("nixling", &["nixling-core", "nixling-ipc"]),
        ("nixlingd", &["nixling-core", "nixling-host", "nixling-ipc"]),
        (
            "nixling-priv-broker",
            &["nixling-core", "nixling-host", "nixling-ipc"],
        ),
    ];
    let internal_crate = Regex::new(
        r"^(nixling-core|nixling-host|nixling-ipc|nixling-priv-broker|nixling|nixlingd|xtask)$",
    )
    .expect("valid internal-crate regex");

    let mut violations: Vec<String> = Vec::new();
    for (crate_name, allowed) in want {
        let toml_rel = format!("packages/{crate_name}/Cargo.toml");
        let Some(toml) = read_repo_file_opt(&toml_rel) else {
            // Mirror the bash gate's per-crate SKIP when a Cargo.toml is absent.
            continue;
        };
        let allowed_set: BTreeSet<&str> = allowed.iter().copied().collect();
        for dep in internal_deps(&toml) {
            if internal_crate.is_match(&dep) && !allowed_set.contains(dep.as_str()) {
                let expected = if allowed.is_empty() {
                    "<none>".to_string()
                } else {
                    allowed.join(" ")
                };
                violations.push(format!(
                    "{crate_name} depends on {dep} (not in allowed set: {expected})"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "static-rust-dependency-direction: disallowed in-workspace dep edge(s):\n{}",
        violations.join("\n")
    );

    // The CLI and daemon must reach the broker only over IPC — assert each
    // actually imports `nixling_ipc` from its own source tree.
    let use_ipc = Regex::new(r"use[[:space:]]+nixling_ipc::").expect("valid use-import regex");
    for crate_name in ["nixling", "nixlingd"] {
        let src_root = format!("packages/{crate_name}/src");
        let imports_ipc = git_listed_files(&[&src_root]).into_iter().any(|rel| {
            read_repo_file_opt(&rel)
                .map(|content| use_ipc.is_match(&content))
                .unwrap_or(false)
        });
        assert!(
            imports_ipc,
            "static-rust-dependency-direction: {crate_name} does not import nixling_ipc \
             from its source tree"
        );
    }
}

#[test]
fn cli_output_contracts_live_in_ipc() {
    let cli = read_repo_file_opt("packages/nixling/src/lib.rs").expect("read nixling lib.rs");
    let ipc = read_repo_file_opt("packages/nixling-ipc/src/cli_output.rs")
        .expect("read nixling-ipc cli_output.rs");
    let xtask = read_repo_file_opt("packages/xtask/src/main.rs").expect("read xtask main.rs");

    for type_name in MIGRATED_CLI_OUTPUT_TYPES {
        assert!(
            !cli_defines_type(&cli, type_name),
            "{type_name} must live in nixling-ipc::cli_output, not packages/nixling/src/lib.rs"
        );
        assert!(
            !xtask_imports_nixling_type(&xtask, type_name),
            "xtask must import {type_name} from nixling_ipc::cli_output, not the nixling presentation crate"
        );
    }

    assert!(
        xtask.contains("cli_output::"),
        "gen-cli-schemas must import CLI output schemas from nixling_ipc::cli_output"
    );

    for type_name in STRICT_CLI_OUTPUT_OBJECT_TYPES {
        assert!(
            struct_has_deny_unknown_fields(&ipc, type_name),
            "{type_name} must retain #[serde(... deny_unknown_fields ...)] after relocation"
        );
    }
}

const MIGRATED_CLI_OUTPUT_TYPES: &[&str] = &[
    "ListOutputV2",
    "ListItemOutputV2",
    "VmExecCreateOutputV1",
    "VmExecListOutputV1",
    "VmExecListEntryOutputV1",
    "VmExecStatusOutputV1",
    "VmExecLogsOutputV1",
    "VmExecKillOutputV1",
    "ShellListOutputV1",
    "ShellListSessionOutputV1",
    "ShellDetachOutputV1",
    "ShellKillOutputV1",
    "VmDisplayListOutputV1",
    "VmDisplaySessionOutputV1",
    "VmDisplayCloseOutputV1",
    "RealmListOutputV1",
    "RealmInspectOutputV1",
    "OpInspectOutputV1",
    "OpInspectTraceOutputV1",
    "OpInspectLocalOutputV1",
    "OpInspectRealmOutputV1",
    "OpInspectDegradedOutputV1",
    "RealmPolicyOutputV1",
    "StatusOutputV2",
    "StatusInventoryOutputV2",
    "ApiReadyStatusV1",
    "ApiReadyErrorV1",
    "ApiReadySimple",
    "StatusVmOutputV2",
    "LivePoolIntegrityOutputV1",
    "StatusServicesOutputV2",
    "StatusServicesOutputV3",
    "RunnerParityOutputV2",
    "StatusBridgeCheckOutputV2",
    "AuditOutputV2",
    "AuditVirtiofsdOutputV2",
    "AuditSshOutputV2",
    "AuditBridgeIsolationOutputV2",
    "AuditSidecarsOutputV2",
    "AuditUsbipEnvOutputV2",
    "HostCheckOutputV2",
    "HostCheckSummaryV2",
    "HostCheckFindingV2",
    "HostCheckSeverityV2",
    "AuthStatusOutputV2",
    "AuthRoleV2",
    "AuthSocketStatusV2",
    "AuthDeniedSubcommandV2",
    "StoreVerifyOutputV2",
];

const STRICT_CLI_OUTPUT_OBJECT_TYPES: &[&str] = &[
    "ListItemOutputV2",
    "VmExecCreateOutputV1",
    "VmExecListOutputV1",
    "VmExecListEntryOutputV1",
    "VmExecStatusOutputV1",
    "VmExecLogsOutputV1",
    "VmExecKillOutputV1",
    "ShellListOutputV1",
    "ShellListSessionOutputV1",
    "ShellDetachOutputV1",
    "ShellKillOutputV1",
    "VmDisplayListOutputV1",
    "VmDisplaySessionOutputV1",
    "VmDisplayCloseOutputV1",
    "RealmListOutputV1",
    "OpInspectOutputV1",
    "OpInspectTraceOutputV1",
    "OpInspectLocalOutputV1",
    "OpInspectRealmOutputV1",
    "OpInspectDegradedOutputV1",
    "RealmPolicyOutputV1",
    "StatusInventoryOutputV2",
    "ApiReadyErrorV1",
    "StatusVmOutputV2",
    "LivePoolIntegrityOutputV1",
    "StatusServicesOutputV2",
    "StatusServicesOutputV3",
    "RunnerParityOutputV2",
    "StatusBridgeCheckOutputV2",
    "AuditOutputV2",
    "AuditVirtiofsdOutputV2",
    "AuditSshOutputV2",
    "AuditBridgeIsolationOutputV2",
    "AuditSidecarsOutputV2",
    "AuditUsbipEnvOutputV2",
    "HostCheckOutputV2",
    "HostCheckSummaryV2",
    "HostCheckFindingV2",
    "AuthStatusOutputV2",
    "AuthSocketStatusV2",
    "AuthDeniedSubcommandV2",
    "StoreVerifyOutputV2",
];

fn cli_defines_type(src: &str, type_name: &str) -> bool {
    src.lines().any(|line| {
        line_defines_pub_type(line, "struct", type_name)
            || line_defines_pub_type(line, "enum", type_name)
    })
}

fn line_defines_pub_type(line: &str, kind: &str, type_name: &str) -> bool {
    let prefix = format!("pub {kind} ");
    let Some(rest) = line.trim_start().strip_prefix(&prefix) else {
        return false;
    };
    let ident = rest
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .next()
        .unwrap_or_default();
    ident == type_name
}

fn xtask_imports_nixling_type(src: &str, type_name: &str) -> bool {
    src.contains(&format!("nixling::{type_name}"))
        || nixling_use_blocks(src).any(|block| {
            block
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .any(|token| token == type_name)
        })
}

fn nixling_use_blocks(src: &str) -> impl Iterator<Item = &str> {
    src.match_indices("use nixling::").filter_map(|(start, _)| {
        let tail = &src[start..];
        tail.find(';').map(|end| &tail[..=end])
    })
}

fn struct_has_deny_unknown_fields(src: &str, type_name: &str) -> bool {
    let lines = src.lines().collect::<Vec<_>>();
    let Some(struct_line) = lines
        .iter()
        .position(|line| line_defines_pub_type(line, "struct", type_name))
    else {
        return false;
    };
    lines[..struct_line]
        .iter()
        .rev()
        .take_while(|line| {
            let line = line.trim_start();
            line.starts_with("#[") || line.starts_with("///") || line.starts_with("//")
        })
        .any(|line| line.contains("deny_unknown_fields"))
}

/// Faithful port of the bash gate's `internal_deps()` awk parser: collect the
/// first whitespace-delimited token of every entry under a `[dependencies]`,
/// `[dev-dependencies]`, `[build-dependencies]`, or
/// `[target.*.dependencies]` table, stripping at the first whitespace or `=`.
fn internal_deps(toml: &str) -> BTreeSet<String> {
    let dep_section =
        Regex::new(r"^\[(dependencies|dev-dependencies|build-dependencies)\]").unwrap();
    let target_dep_section = Regex::new(r"^\[target\..*\.dependencies\]").unwrap();
    let other_section = Regex::new(r"^\[").unwrap();
    let dep_entry = Regex::new(r"^[a-zA-Z0-9_-]+").unwrap();

    let mut in_deps = false;
    let mut deps: BTreeSet<String> = BTreeSet::new();
    for line in toml.lines() {
        if dep_section.is_match(line) || target_dep_section.is_match(line) {
            in_deps = true;
            continue;
        }
        if other_section.is_match(line) {
            in_deps = false;
            continue;
        }
        if in_deps && dep_entry.is_match(line) {
            let mut name = line.split_whitespace().next().unwrap_or("");
            if let Some(eq) = name.find('=') {
                name = &name[..eq];
            }
            if !name.is_empty() {
                deps.insert(name.to_string());
            }
        }
    }
    deps
}
