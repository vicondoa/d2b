//! Policy/source-lint gates over the `nixos-modules/` tree and the Rust
//! workspace dependency graph (the "H-group"), migrated from the
//! `tests/*.sh` bash gates. Each test reads the real repo files and asserts a
//! structural/source invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access - and
//! shelling out to `git` for the gitignore-respecting file enumeration that the
//! bash gates got from `rg` - is sound here.
//!
//! Migrated gates:
//!   * tests/legacy-group-name-denylist.sh    -> legacy_group_name_denylist
//!   * tests/vm-submodule-cutover-eval.sh      -> vm_submodule_cutover
//!   * tests/static-rust-dependency-direction.sh -> static_rust_dependency_direction

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use d2b_contract_tests::{repo_files, repo_root};
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
    repo_files(roots)
}

// ---------------------------------------------------------------------------
// Migrated from tests/legacy-group-name-denylist.sh.
//
// Asserts no live references to the legacy `d2b-launcher{,s}` group names
// remain in source under `nixos-modules`, `packages`, and `tests`. The
// allowlist is matched against full `path:lineno:content` lines (anchored
// `^...$`), NOT as a substring - ported verbatim from the bash gate, with one
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
        "legacy d2b-launcher{{,s}} references found:\n{}",
        violations.join("\n")
    );
}

/// Negative coverage migrated from the retired
/// `tests/legacy-group-name-denylist-self-test.sh`: a forbidden
/// `d2b-launcher` reference in a non-allowlisted source path must be
/// flagged, while an allowlisted (migration-tombstone) reference must not.
#[test]
fn legacy_group_name_denylist_rejects_forbidden_line() {
    let search = legacy_group_search();
    let allowlist = legacy_group_allowlist();

    let forbidden = "packages/forbidden.rs:1:const BAD: &str = \"d2b-launcher\";";
    assert!(
        search.is_match(forbidden),
        "search must match the forbidden line"
    );
    assert!(
        !allowlist.is_match(forbidden),
        "a forbidden d2b-launcher reference in a non-allowlisted path must be flagged"
    );

    let allowed = "nixos-modules/host-users.nix:42:    d2b-launcher = { };";
    assert!(
        allowlist.is_match(allowed),
        "the host-users.nix migration-tombstone line must stay allowlisted"
    );
}

/// The line-matching search for legacy group names (`d2b-launcher{,s}`).
fn legacy_group_search() -> Regex {
    Regex::new(r"d2b-launcher(s)?").expect("valid search regex")
}

/// Full-line (`^...$`-anchored, matched against `path:lineno:content`) allowlist
/// of permitted legacy-group-name references - a verbatim port of the bash
/// gate's `allowlist=(...)` array.
fn legacy_group_allowlist() -> Regex {
    let allowlist_patterns = [
        r"nixos-modules/host-activation\.nix:[0-9]+:[[:space:]]*(legacyLauncherGid|legacyLaunchersGid|getent group|for legacy_name in d2b-launcher d2b-launchers; do).*",
        r"nixos-modules/host-activation-helper/.*",
        r"packages/d2b-host-activation-helper/.*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*# d2b-launcher\{,s\} → d2b rename\. No module references the[[:space:]]*",
        r"nixos-modules/host-users\.nix:[0-9]+:[[:space:]]*d2b-launcher = \{ \};[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# DEPRECATED v1\.2: kept as migration tombstone for the[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*# d2b-launcher\{,s\} → d2b rename\. No module references the[[:space:]]*",
        r"nixos-modules/host-daemon\.nix:[0-9]+:[[:space:]]*users\.groups\.d2b-launchers = \{ \};[[:space:]]*",
        r"packages/d2b-core/src/privileges\.rs:[0-9]+:.*d2b-launcher.*",
        r"packages/d2b-contracts/src/broker_wire\.rs:[0-9]+:.*d2b-launcher.*",
        r"packages/d2b-priv-broker/src/bootstrap\.rs:[0-9]+:.*d2b-launcher.*",
        r"nixos-modules/privileges-json\.nix:[0-9]+:.*d2b-launcher.*",
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
        r"packages/d2b-contract-tests/tests/policy_modules\.rs:[0-9]+:.*",
    ];
    Regex::new(&format!("^({})$", allowlist_patterns.join("|"))).expect("valid allowlist regex")
}

// ---------------------------------------------------------------------------
// Migrated from tests/vm-submodule-cutover-eval.sh.
//
// Asserts no production consumer in `nixos-modules/` reads
// `config.microvm.vms.${name}.config.config.*` directly - every consumer routes
// through the d2b-owned helpers `d2bLib.vmRunner` / `d2bLib.vmToplevel` /
// `d2bLib.vmDeclaredRunner` in `nixos-modules/lib.nix`. `lib.nix`, `host.nix`, and
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
         d2bLib.vmRunner/vmToplevel/vmDeclaredRunner, found direct \
         config.microvm.vms.${{...}}.config.config reads:\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/static-rust-dependency-direction.sh.
//
// The Rust workspace dependency graph flows one way: contracts/core are leaves;
// host depends on core+contracts; the binaries (d2b, d2bd) and the
// privileged broker (d2b-priv-broker, a sibling workspace) sit above. The
// broker must NOT depend on d2bd/d2b; the CLI/daemon must NOT depend on
// the broker.
// This is a pure static parse of the `Cargo.toml` files. It also asserts the
// CLI and daemon actually import `d2b_contracts` from their source trees.
// ---------------------------------------------------------------------------
#[test]
fn static_rust_dependency_direction() {
    // (crate, allowed in-workspace deps) - verbatim port of the bash WANT map.
    let want: &[(&str, &[&str])] = &[
        ("d2b-core", &[]),
        ("d2b-contracts", &["d2b-core"]),
        ("d2b-host", &["d2b-core", "d2b-contracts"]),
        ("xtask", &["d2b-core", "d2b-contracts", "d2b", "d2bd"]),
        ("d2b", &["d2b-core", "d2b-contracts"]),
        ("d2bd", &["d2b-core", "d2b-host", "d2b-contracts"]),
        (
            "d2b-priv-broker",
            &["d2b-core", "d2b-host", "d2b-contracts"],
        ),
    ];
    let internal_crate =
        Regex::new(r"^(d2b-core|d2b-host|d2b-contracts|d2b-priv-broker|d2b|d2bd|xtask)$")
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

    // The CLI and daemon must reach the broker only over IPC - assert each
    // actually imports `d2b_contracts` from its own source tree.
    let use_contracts =
        Regex::new(r"use[[:space:]]+d2b_contracts::").expect("valid use-import regex");
    for crate_name in ["d2b", "d2bd"] {
        let src_root = format!("packages/{crate_name}/src");
        let imports_contracts = git_listed_files(&[&src_root]).into_iter().any(|rel| {
            read_repo_file_opt(&rel)
                .map(|content| use_contracts.is_match(&content))
                .unwrap_or(false)
        });
        assert!(
            imports_contracts,
            "static-rust-dependency-direction: {crate_name} does not import d2b_contracts \
             from its source tree"
        );
    }
}

#[test]
fn authority_capability_is_not_downstream_mintable() {
    let core = read_repo_file_opt("packages/d2b-core-controller/src/authority_persistence.rs")
        .expect("read core authority persistence");
    let capability_block = core
        .split_once("pub struct AuthorityOperationCapability")
        .and_then(|(_, tail)| tail.split_once("/// Typed persistence port"))
        .map(|(block, _)| block)
        .expect("authority capability block");
    let public_constructor =
        Regex::new(r"\bpub(?:\([^)]*\))?\s+fn\s+new\s*\(").expect("constructor regex");
    assert!(
        !public_constructor.is_match(capability_block),
        "AuthorityOperationCapability must not expose a public constructor"
    );
    assert!(
        !capability_block.contains("Default"),
        "AuthorityOperationCapability must not implement Default"
    );
    assert!(
        !capability_block.contains("Deserialize"),
        "AuthorityOperationCapability must not be deserializable"
    );
    let adapter = read_repo_file_opt("packages/d2bd/src/authority_persistence.rs")
        .expect("read d2bd adapter");
    assert!(
        adapter.contains("PreparedAuthorityOperation") && adapter.contains("AuthorityRecoveryData"),
        "d2bd must return non-authorizing prepared/recovery data"
    );
    assert!(
        !adapter.contains("AuthorityOperationCapability::new"),
        "d2bd must not mint core capabilities directly"
    );
}

#[test]
fn providers_and_controllers_use_closed_effect_ports() {
    let crates = provider_controller_crates();
    let forbidden_internal = [
        "d2b-priv-broker",
        "d2bd",
        "d2b-resource-store",
        "d2b-resource-api",
    ];
    let allowed_internal = [
        "d2b-audit",
        "d2b-contracts",
        "d2b-controller-toolkit",
        "d2b-core",
        "d2b-core-controller",
        "d2b-host-argv",
        "d2b-process",
        "d2b-process-conformance",
        "d2b-provider",
        "d2b-provider-system-core",
        "d2b-provider-system-minijail",
        "d2b-provider-system-systemd",
        "d2b-provider-toolkit",
        "d2b-realm-codec-protobuf",
        "d2b-realm-core",
        "d2b-realm-provider",
        "d2b-realm-transport",
        "d2b-session",
        "d2b-telemetry",
        "d2b-resource-store-redb",
    ];
    let mut violations = Vec::new();
    for (crate_name, manifest_dir, src_root) in crates {
        let manifest = read_repo_file_opt(&format!("{manifest_dir}/Cargo.toml"))
            .unwrap_or_else(|| panic!("read {crate_name} manifest"));
        for dependency in internal_deps(&manifest) {
            if forbidden_internal.contains(&dependency.as_str())
                || (dependency == "d2b-resource-store-redb" && crate_name != "d2b-core-controller")
            {
                violations.push(format!(
                    "{crate_name}: direct dependency {dependency} bypasses the effect port"
                ));
            }
            if dependency.starts_with("d2b-")
                && !forbidden_internal.contains(&dependency.as_str())
                && !allowed_internal.contains(&dependency.as_str())
            {
                violations.push(format!(
                    "{crate_name}: direct dependency {dependency} is outside the Provider/controller allowlist"
                ));
            }
        }
        for rel in git_listed_files(&[&src_root]) {
            if !rel.ends_with(".rs") {
                continue;
            }
            if rel == "packages/d2b-provider-observability-otel/src/emitter_socket.rs"
                || rel == "packages/d2b-provider-supervisor/src/broker.rs"
                || rel == "packages/d2b-provider-supervisor/src/systemd.rs"
                || rel == "packages/d2b-provider-supervisor/src/lib.rs"
            {
                continue;
            }
            let Some(content) = read_repo_file_opt(&rel) else {
                continue;
            };
            let Ok(file) = syn::parse_file(&content) else {
                violations.push(format!("{rel}: Rust source did not parse"));
                continue;
            };
            let mut visitor = HostEffectVisitor::default();
            syn::visit::Visit::visit_file(&mut visitor, &file);
            for effect in visitor.forbidden {
                violations.push(format!("{rel}: prohibited host effect API {effect}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "provider/controller code must route host effects through typed ports:\n{}",
        violations.join("\n")
    );

    assert!(
        read_repo_file_opt("packages/d2b-provider-device-tpm/src/lib.rs")
            .expect("read device-tpm lib.rs")
            .contains("TpmEffectPort"),
        "device-tpm must expose its closed effect port"
    );
    let volume = read_repo_file_opt("packages/d2b-provider-volume-local/src/lib.rs")
        .expect("read volume-local lib.rs");
    assert!(
        volume.contains("VolumeLayoutEffectPort") && volume.contains("VolumeSourceEffectPort"),
        "volume-local must expose typed layout/source effect ports"
    );
    let runtime =
        read_repo_file_opt("packages/d2bd/src/resource_runtime.rs").expect("read resource runtime");
    let runtime_file = syn::parse_file(&runtime).expect("resource runtime parses");
    let mut runtime_visitor = AuthorityBoundaryVisitor::default();
    syn::visit::Visit::visit_file(&mut runtime_visitor, &runtime_file);
    assert!(
        runtime_visitor.saw_durable_reservation,
        "production Zone runtime must call AuthorityReservation::reserve_durable"
    );
}

fn provider_controller_crates() -> Vec<(String, String, String)> {
    let cargo = std::env::var_os("CARGO")
        .map(std::path::PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repo_root().join(path)
            }
        })
        .unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(repo_root())
        .args([
            "metadata",
            "--manifest-path",
            "Cargo.toml",
            "--format-version",
            "1",
            "--no-deps",
        ])
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata JSON");
    let mut crates = Vec::new();
    for package in metadata["packages"].as_array().expect("metadata packages") {
        let name = package["name"].as_str().expect("package name");
        if name != "d2b-core-controller"
            && (!name.starts_with("d2b-provider-") || name == "d2b-provider-toolkit")
        {
            continue;
        }
        let manifest = package["manifest_path"].as_str().expect("manifest path");
        let manifest = manifest
            .strip_prefix(&format!("{}/", repo_root().display()))
            .unwrap_or(manifest)
            .to_owned();
        let manifest_dir = manifest
            .strip_suffix("/Cargo.toml")
            .unwrap_or(manifest.as_str())
            .to_owned();
        crates.push((
            name.to_owned(),
            manifest_dir.clone(),
            format!("{manifest_dir}/src"),
        ));
    }
    crates.sort();
    assert!(
        crates
            .iter()
            .any(|(name, _, _)| name == "d2b-core-controller"),
        "Cargo metadata must include d2b-core-controller"
    );
    assert!(
        crates
            .iter()
            .any(|(name, _, _)| name == "d2b-provider-device-tpm"),
        "Cargo metadata must include device-tpm Provider"
    );
    crates
}

#[derive(Default)]
struct HostEffectVisitor {
    forbidden: Vec<String>,
    aliases: BTreeMap<String, String>,
}

impl<'ast> syn::visit::Visit<'ast> for HostEffectVisitor {
    fn visit_file(&mut self, file: &'ast syn::File) {
        for item in &file.items {
            if let syn::Item::Use(item) = item {
                collect_aliases(&item.tree, &mut Vec::new(), &mut self.aliases);
            }
        }
        syn::visit::visit_file(self, file);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        collect_aliases(&item.tree, &mut Vec::new(), &mut self.aliases);
        collect_use_tree(&item.tree, &mut Vec::new(), &mut self.forbidden);
        syn::visit::visit_item_use(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let joined = segments.join("::");
        let resolved = self
            .aliases
            .get(segments.first().map(String::as_str).unwrap_or_default())
            .map(|alias| {
                if segments.len() == 1 {
                    alias.clone()
                } else {
                    format!("{alias}::{}", segments[1..].join("::"))
                }
            })
            .unwrap_or_else(|| joined.clone());
        let forbidden = [
            "std::fs",
            "std::net",
            "std::path",
            "std::process::Command",
            "std::process::Stdio",
            "std::os::unix::net",
            "tokio::net",
            "systemd",
            "d2b_priv_broker",
        ];
        if (forbidden
            .iter()
            .any(|prefix| resolved == *prefix || resolved.starts_with(&format!("{prefix}::")))
            || segments
                .iter()
                .any(|segment| matches!(segment.as_str(), "TcpStream" | "UdpSocket")))
            && !self.forbidden.contains(&resolved)
        {
            self.forbidden.push(resolved);
        }

        syn::visit::visit_path(self, path);
    }
}

fn collect_aliases(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    aliases: &mut BTreeMap<String, String>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_aliases(&path.tree, prefix, aliases);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut full_parts = prefix.clone();
            full_parts.push(name.ident.to_string());
            let full = full_parts.join("::");
            aliases.insert(name.ident.to_string(), full);
        }
        syn::UseTree::Rename(rename) => {
            let mut full_parts = prefix.clone();
            full_parts.push(rename.ident.to_string());
            let full = full_parts.join("::");
            aliases.insert(rename.rename.to_string(), full);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_aliases(item, prefix, aliases);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn collect_use_tree(tree: &syn::UseTree, prefix: &mut Vec<String>, forbidden: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, forbidden);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            record_forbidden_use(path, forbidden);
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            record_forbidden_use(path, forbidden);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix, forbidden);
            }
        }
        syn::UseTree::Glob(_) => record_forbidden_use(prefix.clone(), forbidden),
    }
}

fn record_forbidden_use(path: Vec<String>, forbidden: &mut Vec<String>) {
    let joined = path.join("::");
    if [
        "std::fs",
        "std::net",
        "std::path",
        "std::process::Command",
        "std::process::Stdio",
        "std::os::unix::net",
        "tokio::net",
        "systemd",
        "d2b_priv_broker",
    ]
    .iter()
    .any(|prefix| joined == *prefix || joined.starts_with(&format!("{prefix}::")))
    {
        forbidden.push(joined);
    }
}

#[derive(Default)]
struct AuthorityBoundaryVisitor {
    saw_durable_reservation: bool,
}

impl<'ast> syn::visit::Visit<'ast> for AuthorityBoundaryVisitor {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "reserve_durable" {
            self.saw_durable_reservation = true;
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        let path = expression
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if path.windows(2).any(|pair| {
            pair == ["AuthorityReservation", "reserve_durable"]
                || pair == ["ExternalNicReservation", "reserve_durable"]
        }) {
            self.saw_durable_reservation = true;
        }
        syn::visit::visit_expr_path(self, expression);
    }
}

#[test]
fn host_effect_ast_policy_ignores_comments_and_strings_but_catches_aliases() {
    let source = r#"
        // std::fs::remove_file("/host")
        const TEXT: &str = "tokio::net::TcpStream";
        use std::fs as host_fs;
        fn mutate() { let _ = host_fs::remove_file("owned"); }
    "#;
    let file = syn::parse_file(source).expect("fixture parses");
    let mut visitor = HostEffectVisitor::default();
    syn::visit::Visit::visit_file(&mut visitor, &file);
    assert!(
        visitor
            .forbidden
            .iter()
            .any(|path| path.starts_with("std::fs"))
    );
    assert!(
        !visitor.forbidden.is_empty() && visitor.forbidden.len() <= 2,
        "AST visitor should report the aliased import and call once each"
    );
}

#[test]
fn cli_output_contracts_live_in_contract_crate() {
    let cli = read_repo_file_opt("packages/d2b/src/lib.rs").expect("read d2b lib.rs");
    let ipc = read_repo_file_opt("packages/d2b-contracts/src/cli_output.rs")
        .expect("read d2b-contracts cli_output.rs");
    let xtask = read_repo_file_opt("packages/xtask/src/main.rs").expect("read xtask main.rs");

    for type_name in MIGRATED_CLI_OUTPUT_TYPES {
        assert!(
            !cli_defines_type(&cli, type_name),
            "{type_name} must live in d2b-contracts::cli_output, not packages/d2b/src/lib.rs"
        );
        assert!(
            !xtask_imports_d2b_type(&xtask, type_name),
            "xtask must import {type_name} from d2b_contracts::cli_output, not the d2b presentation crate"
        );
    }

    assert!(
        xtask.contains("cli_output::"),
        "gen-cli-schemas must import CLI output schemas from d2b_contracts::cli_output"
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

fn xtask_imports_d2b_type(src: &str, type_name: &str) -> bool {
    src.contains(&format!("d2b::{type_name}"))
        || d2b_use_blocks(src).any(|block| {
            block
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .any(|token| token == type_name)
        })
}

fn d2b_use_blocks(src: &str) -> impl Iterator<Item = &str> {
    src.match_indices("use d2b::").filter_map(|(start, _)| {
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
    let dep_entry = Regex::new(r"^([a-zA-Z0-9_-]+)\s*=").unwrap();
    let package_entry = Regex::new(r#"package\s*=\s*"([a-zA-Z0-9_-]+)""#).unwrap();

    let mut in_deps = false;
    let mut deps: BTreeSet<String> = BTreeSet::new();
    let mut pending_alias: Option<String> = None;
    let mut pending_package: Option<String> = None;
    let finish = |deps: &mut BTreeSet<String>,
                  pending_alias: &mut Option<String>,
                  pending_package: &mut Option<String>| {
        if let Some(alias) = pending_alias.take() {
            deps.insert(pending_package.take().unwrap_or(alias));
        }
    };
    for line in toml.lines() {
        if dep_section.is_match(line) || target_dep_section.is_match(line) {
            finish(&mut deps, &mut pending_alias, &mut pending_package);
            in_deps = true;
            continue;
        }
        if other_section.is_match(line) {
            finish(&mut deps, &mut pending_alias, &mut pending_package);
            in_deps = false;
            continue;
        }
        if !in_deps {
            continue;
        }
        let trimmed = line.trim();
        if let Some(captures) = dep_entry.captures(trimmed) {
            finish(&mut deps, &mut pending_alias, &mut pending_package);
            let alias = captures[1].to_owned();
            if let Some(package) = package_entry.captures(trimmed) {
                deps.insert(package[1].to_owned());
            } else if trimmed.contains('{') {
                pending_alias = Some(alias);
            } else {
                deps.insert(alias);
            }
        } else if pending_alias.is_some() {
            if let Some(package) = package_entry.captures(trimmed) {
                pending_package = Some(package[1].to_owned());
            }
            if trimmed == "}" || trimmed.ends_with("},") {
                finish(&mut deps, &mut pending_alias, &mut pending_package);
            }
        }
    }
    finish(&mut deps, &mut pending_alias, &mut pending_package);
    deps
}
