//! Larger source/doc-lint policy gates (the "H-group"), migrated from the
//! `tests/*-eval.sh` bash gates. Each test reads the real repo files (via the
//! `d2b_contract_tests` repo-file helpers and a gitignore-respecting
//! `git ls-files` enumeration) and asserts a structural / source / doc
//! invariant. This crate runs only from `tests/tools/rust-workspace-checks.sh`
//! against the real checkout (it is excluded from the hermetic Nix sandbox
//! workspace build), so repo-file access - and shelling out to `git` for the
//! gitignore-respecting file enumeration that the bash gates got from `rg` - is
//! sound here.
//!
//! Migrated gates:
//!   * tests/no-bash-exec-eval.sh  -> no_bash_exec_check + no_bash_exec_fixture_coverage
//!   * tests/host-prep-dag-eval.sh -> host_prep_dag_module_surface +
//!     host_prep_dag_d2b_host_reexport + host_prep_dag_broker_wire_scaffolds +
//!     host_prep_dag_d2bd_wiring + host_prep_dag_documentation
//!
//! Scan-root note (no-bash-exec): the bash gate's `check` mode scans
//! `packages/` while excluding any path with a `target/`, `tests/`, or `.git/`
//! directory component. This Rust port file lives under
//! `packages/d2b-contract-tests/tests/`, i.e. under a `tests/` component,
//! so it is excluded from its own scan - the forbidden-pattern regex string it
//! carries can never flag itself.
//!
//! syn-ast-walk note (no-bash-exec): the bash gate's third mode handed off to
//! the standalone `tests/tools/no-bash-ast-walker/` cargo tool (a `syn`-based
//! AST walker), falling back to `check` mode whenever that tool was absent.
//! Porting the AST walker into this crate would require adding `syn` as a
//! dev-dependency, which is out of scope for this migration; the standalone
//! walker tool remains in place. The per-line regex `check` mode ported here is
//! a strict superset of the walker's literal detection at the source-line level
//! (it matches the same `Command::new("…bash"|"…sh")` literals and more), so the
//! ADR 0017 regression invariant is preserved.

use std::collections::BTreeSet;

use d2b_contract_tests::{read_repo_file, repo_files, repo_path_exists, repo_root};
use regex::Regex;

/// Read a repo-relative file, returning `None` when the path is absent or not
/// valid UTF-8 (binary files are skipped, mirroring `rg`/`grep -I`).
fn read_repo_file_opt(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join(rel)).ok()
}

/// Enumerate repo-relative tracked + untracked-non-ignored files under the
/// given pathspecs via `git ls-files`. This mirrors `rg`'s default behaviour
/// (respects `.gitignore`, so build artifacts under `target/` and Nix `result`
/// symlinks are excluded) that the original bash gates relied on.
fn git_listed_files(roots: &[&str]) -> Vec<String> {
    repo_files(roots)
}

/// Whether `rel` lives under an excluded directory component, mirroring the
/// bash gate's `rg -g '!**/target/**' -g '!**/tests/**' -g '!**/.git/**'`
/// globs. A `**/X/**` glob excludes paths where `X` is a *directory* component
/// (something follows it), so we test every component except the final
/// (filename) one.
fn is_excluded_dir(rel: &str) -> bool {
    let mut components: Vec<&str> = rel.split('/').collect();
    components.pop(); // drop the filename component
    components
        .iter()
        .any(|c| matches!(*c, "target" | "tests" | ".git"))
}

#[test]
fn nix_package_source_filters_are_path_segment_based() {
    let files = ["nixos-modules/rust-host-tools.nix"];

    for rel in files {
        let content = read_repo_file(rel);
        assert!(
            !content.contains("hasInfix \"target\" rel"),
            "{rel}: package source filters must not substring-match `target`; \
             that excludes legitimate files like \
             packages/d2b-realm-core/src/target.rs"
        );
        assert!(
            content.contains("d2bLib.cleanRustPackagesSource ../."),
            "{rel}: package source filters must use the centralized segment-based helper"
        );
    }

    let helper = read_repo_file("nixos-modules/lib.nix");
    assert!(
        helper.contains(r#"type == "directory" && baseNameOf path == "target""#),
        "nixos-modules/lib.nix: cleanRustPackagesSource must exclude only a directory segment named `target`"
    );

    let gateway_vm = read_repo_file("nixos-modules/gateway-vm.nix");
    assert!(
        !gateway_vm.contains("cleanSourceWith") && !gateway_vm.contains("filterSource"),
        "nixos-modules/gateway-vm.nix: gateway VMs must consume _hostToolPackages, \
         not define an ad-hoc Rust source filter"
    );
}

#[test]
fn host_sccache_is_a_global_opt_in_daemon_contract() {
    let options = read_repo_file("nixos-modules/options-host-sccache.nix");
    let module = read_repo_file("nixos-modules/host-sccache.nix");
    let tools = read_repo_file("nixos-modules/rust-host-tools.nix");
    let makefile = read_repo_file("Makefile");

    assert!(
        options.contains("options.d2b.site.hostSccache.enable")
            && options.contains("default = false"),
        "host sccache must remain disabled by default"
    );
    assert!(
        module.contains("extra-sandbox-paths")
            && module.contains("\"build-users-group\"")
            && module.contains("cacheDir = \"/var/cache/d2b-sccache\"")
            && module.contains("2770 root ${buildUsersGroup} -")
            && module.contains("hostPlatform.isLinux")
            && module.contains("builtins.hasAttr buildUsersGroup config.users.groups"),
        "host sccache must provision the fixed multi-user daemon cache with eval-time posture checks"
    );
    assert!(
        !module.contains("systemd.services"),
        "host sccache must not add a service"
    );
    assert!(
        tools.contains("SCCACHE_CACHE_SIZE")
            && tools.contains("10G")
            && tools.contains("configured cache")
            && !tools.contains("owner-private")
            && tools.contains("exec \"$@\""),
        "host-tool sccache wrapper must be bounded, visible when configured, and rustc-fallback safe"
    );
    assert!(
        makefile.contains("D2B_HOST_SCCACHE")
            && makefile.contains("/etc/nix/nix.conf")
            && makefile.contains("root:nixbld")
            && makefile.contains("mode 2770")
            && makefile.contains("D2B_HOST_VM_CHECK")
            && makefile.contains("unknown vmCheck")
            && !makefile.contains("--option extra-sandbox-paths")
            && !makefile.contains("XDG_CACHE_HOME")
            && !makefile.contains("SCCACHE_DIR:-"),
        "host integration must use the global daemon mount and its fail-closed preflight"
    );
}

#[test]
fn host_sccache_policy_is_in_the_bazel_policy_suite() {
    let policy_build = read_repo_file("bazel/checks/policy/BUILD.bazel");

    assert!(
        policy_build
            .matches("//packages/d2b-contract-tests:policy_source")
            .count()
            >= 2,
        "Bazel policy suite must run the host sccache source contract"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/no-bash-exec-eval.sh (v1.1 / ADR 0017: "the Rust CLI
// never executes bash").
//
//   * mode_check            -> no_bash_exec_check
//   * mode_fixture_coverage -> no_bash_exec_fixture_coverage
//
// The forbidden-pattern regex and the allow-list mechanism are ported verbatim.
// ---------------------------------------------------------------------------

/// Repo-relative path of the exempt-paths allow-list fixture.
const EXEMPT_PATHS_FIXTURE: &str = "tests/fixtures/no-bash-exec-exempt-paths.json";

/// The bash-exec forbidden pattern, ported verbatim from the gate's
/// `BASH_EXEC_PATTERN`. Covers:
///   Command::new("bash")
///   Command::new("/bin/sh") / Command::new("/bin/bash")
///   Command::new("/usr/bin/env" ...) with a bash/sh follow-up arg
/// matched per source line.
fn bash_exec_pattern() -> Regex {
    Regex::new(r#"Command::new\("(/bin/|/usr/bin/)?(env(\s+|\s*"\s*,\s*"\s*))?(ba)?sh""#)
        .expect("valid bash-exec regex")
}

/// Read the allow-list (`exempt_paths` array) from the JSON fixture. Returns the
/// set of repo-root-relative paths the gate permits to carry a bash-exec site.
fn exempt_paths() -> BTreeSet<String> {
    assert!(
        repo_path_exists(EXEMPT_PATHS_FIXTURE),
        "no-bash-exec-eval: missing exempt-paths fixture {EXEMPT_PATHS_FIXTURE}"
    );
    let raw = read_repo_file(EXEMPT_PATHS_FIXTURE);
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!("no-bash-exec-eval: {EXEMPT_PATHS_FIXTURE} is not JSON: {err}")
    });
    parsed
        .get("exempt_paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn no_bash_exec_check() {
    let pattern = bash_exec_pattern();
    let exempt = exempt_paths();

    let mut violations: Vec<String> = Vec::new();
    for rel in git_listed_files(&["packages"]) {
        if is_excluded_dir(&rel) {
            continue;
        }
        let Some(content) = read_repo_file_opt(&rel) else {
            // Skip binary / unreadable files, mirroring `rg`/`grep -I`.
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if !pattern.is_match(line) {
                continue;
            }
            // The allow-list is matched against the exact repo-relative path
            // (the bash gate's `grep -Fxq -- "${file#$ROOT/}"`).
            if exempt.contains(&rel) {
                continue;
            }
            violations.push(format!("{rel}:{}:{line}", idx + 1));
        }
    }

    assert!(
        violations.is_empty(),
        "no-bash-exec-eval[check]: found bash exec sites not in allow-list \
         (ADR 0017 - the Rust CLI must never invoke bash; allow-list additions \
         require dev-tool dependency governance):\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_bash_exec_fixture_coverage() {
    let exempt = exempt_paths();
    let mut stale: Vec<String> = Vec::new();
    for path in &exempt {
        if !repo_path_exists(path) {
            stale.push(path.clone());
        }
    }
    assert!(
        stale.is_empty(),
        "no-bash-exec-eval[fixture-coverage]: stale allow-list entries (files missing) - \
         remove them from {EXEMPT_PATHS_FIXTURE}:\n{}",
        stale.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-prep-dag-eval.sh.
//
// Asserts the host-prep DAG module + broker wire scaffolds + daemon wiring +
// docs carry the documented step set, public API, ordering edges, and
// broker-op mapping. Static gate - no nixpkgs eval required.
// ---------------------------------------------------------------------------

#[test]
fn broker_never_shells_out_to_vm_switch_to_configuration() {
    let files = [
        "packages/d2b-priv-broker/src/live_handlers.rs",
        "packages/d2b-priv-broker/src/runtime.rs",
        "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
    ];
    for rel in files {
        let content = read_repo_file(rel);
        let production = content
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(content.as_str());
        assert!(
            !production.contains("exec \"$dst/bin/switch-to-configuration\"")
                && !production.contains("switch-to-configuration\" \"$mode\""),
            "{rel}: production broker code must not execute VM switch-to-configuration"
        );
        let lines: Vec<_> = production.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            if !line.contains("switch-to-configuration") {
                continue;
            }
            let start = idx.saturating_sub(12);
            let end = usize::min(lines.len(), idx + 13);
            let window = lines[start..end].join("\n");
            assert!(
                !window.contains("Command::new")
                    && !window.contains("/bin/sh")
                    && !window.contains("unshare")
                    && !window.contains("mount --bind"),
                "{rel}: switch-to-configuration may only be returned as a guest path, not executed from host context near line {}",
                idx + 1
            );
        }
    }
}

/// The ten canonical host-prep step kinds, in the order the bash gate lists
/// them.
const HOST_PREP_STEP_KINDS: &[&str] = &[
    "BringUpTapInterface",
    "PreOpenVhostNetFd",
    "SeedDnsmasqLease",
    "BindMountFromHardlinkFarm",
    "ApplyNftablesRules",
    "OwnershipMatrixCheck",
    "SshHostKeyPreflight",
    "ApplyNmUnmanaged",
    "ApplySysctl",
    "SetBridgePortFlags",
];

const HOST_PREP_DAG_MOD: &str = "packages/d2b-host/src/host_prep_dag.rs";

/// Mirror `grep -B <before> KIND_LINE | grep -q DEP`: returns true iff `dep`
/// appears (as a substring) on any line within the window of `before` preceding
/// lines through each line containing `kind_line` (inclusive). The bash gate
/// pipes every such window together and greps the combined output, so a match
/// in any window passes.
fn dep_edge_within_window(content: &str, kind_line: &str, before: usize, dep: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if line.contains(kind_line) {
            let start = idx.saturating_sub(before);
            if lines[start..=idx].iter().any(|l| l.contains(dep)) {
                return true;
            }
        }
    }
    false
}

#[test]
fn host_prep_dag_module_surface() {
    assert!(
        repo_path_exists(HOST_PREP_DAG_MOD),
        "host-prep-dag-eval: missing {HOST_PREP_DAG_MOD}"
    );
    let module = read_repo_file(HOST_PREP_DAG_MOD);

    // ==> host-prep DAG module surface: every step kind variant declared in
    // the typed enum. Per-line: `^\s+KIND,?$` or `^\s+KIND\b`.
    for kind in HOST_PREP_STEP_KINDS {
        let re = Regex::new(&format!(r"^\s+{k},?$|^\s+{k}\b", k = regex::escape(kind)))
            .expect("valid step-kind regex");
        assert!(
            module.lines().any(|line| re.is_match(line)),
            "host_prep_dag.rs missing HostPrepStepKind::{kind}"
        );
    }

    // ==> host-prep DAG ordering edges (presence in the builder).
    assert!(
        module.contains("kind: HostPrepStepKind::ApplyNftablesRules"),
        "host_prep_dag.rs missing ApplyNftablesRules step kind in builder"
    );
    assert!(
        module.contains("id(HostPrepStepKind::ApplyNmUnmanaged)"),
        "host_prep_dag.rs missing ApplyNmUnmanaged dep edge in ApplyNftablesRules"
    );
    assert!(
        module.contains("kind: HostPrepStepKind::ApplySysctl"),
        "host_prep_dag.rs missing ApplySysctl step kind in builder"
    );
    assert!(
        module.contains("kind: HostPrepStepKind::SetBridgePortFlags"),
        "host_prep_dag.rs missing SetBridgePortFlags step kind in builder"
    );

    // ==> Public API.
    for sym in [
        "pub struct HostPrepStep",
        "pub struct HostPrepStepId",
        "pub enum HostPrepStepKind",
        "pub struct BundleStepRef",
        "pub struct HostPrepStepFailed",
        "pub enum CycleError",
        "pub fn build_host_prep_dag",
        "pub fn build_host_prep_dag_for",
        "pub fn topo_sort",
    ] {
        assert!(module.contains(sym), "host_prep_dag.rs missing '{sym}'");
    }

    // ==> source-side ordering edges (depends_on), via the `grep -B 3` window.
    assert!(
        dep_edge_within_window(
            &module,
            "kind: HostPrepStepKind::ApplySysctl",
            3,
            "id(HostPrepStepKind::BringUpTapInterface)",
        ),
        "ApplySysctl missing BringUpTapInterface dep edge"
    );
    assert!(
        dep_edge_within_window(
            &module,
            "kind: HostPrepStepKind::SetBridgePortFlags",
            3,
            "id(HostPrepStepKind::ApplySysctl)",
        ),
        "SetBridgePortFlags missing ApplySysctl dep edge"
    );
    assert!(
        dep_edge_within_window(
            &module,
            "kind: HostPrepStepKind::PreOpenVhostNetFd",
            3,
            "id(HostPrepStepKind::SetBridgePortFlags)",
        ),
        "PreOpenVhostNetFd missing SetBridgePortFlags dep edge"
    );
}

#[test]
fn host_prep_dag_d2b_host_reexport() {
    let lib = read_repo_file("packages/d2b-host/src/lib.rs");
    assert!(
        lib.contains("pub mod host_prep_dag;"),
        "d2b-host lib.rs does not re-export host_prep_dag"
    );
}

#[test]
fn host_prep_dag_broker_wire_scaffolds() {
    let wire_rel = "packages/d2b-contracts/src/broker_wire.rs";
    let runtime_rel = "packages/d2b-priv-broker/src/runtime.rs";
    assert!(
        repo_path_exists(wire_rel),
        "host-prep-dag-eval: missing {wire_rel}"
    );
    assert!(
        repo_path_exists(runtime_rel),
        "host-prep-dag-eval: missing {runtime_rel}"
    );
    let wire = read_repo_file(wire_rel);
    let runtime = read_repo_file(runtime_rel);

    // Both the two live broker arms and the two still-deferred typed
    // Unimplemented stubs must carry the wire scaffolds + dispatch arm.
    for variant in [
        "SeedDnsmasqLease",
        "BindMountFromHardlinkFarm",
        "OwnershipMatrixCheck",
        "SshHostKeyPreflight",
    ] {
        assert!(
            wire.contains(&format!("{variant}({variant}Request)")),
            "broker_wire.rs missing BrokerRequest::{variant}"
        );
        assert!(
            wire.contains(&format!("pub struct {variant}Request")),
            "broker_wire.rs missing {variant}Request struct"
        );
        assert!(
            wire.contains(&format!("Self::{variant}(_) => \"{variant}\"")),
            "broker_wire.rs missing op_name arm for {variant}"
        );
        assert!(
            runtime.contains(&format!("RealBrokerRequest::{variant}")),
            "runtime.rs missing dispatch arm for {variant}"
        );
    }

    // OwnershipMatrixCheck + SshHostKeyPreflight stay typed Unimplemented
    // (expected to remain deferred).
    for variant in ["OwnershipMatrixCheck", "SshHostKeyPreflight"] {
        assert!(
            runtime.contains(&format!("operation: \"{variant}\"")),
            "runtime.rs missing Unimplemented op label for {variant} (expected to stay deferred)"
        );
    }

    // SeedDnsmasqLease + BindMountFromHardlinkFarm are live broker arms that
    // record a typed audit row.
    for variant in ["SeedDnsmasqLease", "BindMountFromHardlinkFarm"] {
        assert!(
            runtime.contains(&format!("\"{variant}\"")),
            "runtime.rs missing op label for live {variant} arm"
        );
        assert!(
            runtime.contains(&format!("OperationFields::{variant}")),
            "runtime.rs missing OperationFields::{variant} audit row for live arm"
        );
    }
}

#[test]
fn host_prep_dag_d2bd_wiring() {
    let lib = read_repo_file("packages/d2bd/src/lib.rs");
    for marker in [
        "build_host_prep_dag",
        "log_host_prep_dag",
        "execute_host_prep_dag",
        "D2B_HOST_PREP_DAG_EXECUTE",
    ] {
        assert!(lib.contains(marker), "d2bd/src/lib.rs missing '{marker}'");
    }
}

#[test]
fn host_prep_dag_documentation() {
    let doc_rel = "docs/reference/host-prep-dag.md";
    assert!(
        repo_path_exists(doc_rel),
        "host-prep-dag-eval: missing {doc_rel}"
    );
    let doc = read_repo_file(doc_rel);

    // ==> documentation: every step kind named.
    for kind in HOST_PREP_STEP_KINDS {
        assert!(doc.contains(kind), "host-prep-dag.md missing {kind}");
    }

    // ==> documentation: ordering phrases (incl. one with a backtick literal).
    for phrase in [
        "BEFORE tap creation",
        "AFTER tap creation",
        "AFTER `ApplySysctl`",
    ] {
        assert!(
            doc.contains(phrase),
            "host-prep-dag.md missing ordering phrase '{phrase}'"
        );
    }

    // Self-reference its own slug.
    //
    // Spec correction: the bash gate asserted the doc *content* contains the
    // substring "host-prep-dag" via `grep -qF "host-prep-dag"`. In the current
    // repo that substring appears in the doc body at exactly one place - a
    // reference to the gate's own `tests/host-prep-dag-eval.sh` filename. That
    // `.sh` is being retired by this migration and the integrator may rewrite
    // that reference, so relying on the `.sh` filename as a content marker is
    // the self-referential doc trap. Instead, the slug is asserted via the
    // doc's canonical path: the file is `docs/reference/host-prep-dag.md`, i.e.
    // the slug IS the filename, which is robust to the `.sh` retirement.
    assert!(
        repo_path_exists("docs/reference/host-prep-dag.md"),
        "host-prep-dag.md missing at its canonical slug path"
    );

    // ==> documentation: Canonical step set completeness. Extract the section
    // between `## Canonical step set` and the next `## ` heading, mirroring the
    // bash gate's awk extraction, and assert every step slug appears in it.
    let canonical_section = canonical_step_set_section(&doc);
    for slug in [
        "ssh-host-key-preflight",
        "ownership-matrix-check",
        "apply-nm-unmanaged",
        "apply-nftables-rules",
        "bring-up-tap-interface",
        "apply-sysctl",
        "set-bridge-port-flags",
        "pre-open-vhost-net-fd",
        "bind-mount-from-hardlink-farm",
        "seed-dnsmasq-lease",
    ] {
        assert!(
            canonical_section.contains(slug),
            "Canonical step set section MISSING {slug} (drift between section and table)"
        );
    }
}

/// Extract the body of the `## Canonical step set` section, mirroring the bash
/// gate's `awk '/^## Canonical step set/{f=1;next} /^## /{if(f){exit}} f{print}'`:
/// start collecting after the `## Canonical step set` heading line, stop at the
/// next `## ` heading.
fn canonical_step_set_section(doc: &str) -> String {
    let mut collecting = false;
    let mut out: Vec<&str> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## Canonical step set") {
            collecting = true;
            continue;
        }
        if line.starts_with("## ") {
            if collecting {
                break;
            }
            continue;
        }
        if collecting {
            out.push(line);
        }
    }
    out.join("\n")
}

const NORMAL_ZONE_RUNTIME_FILES: &[&str] = &[
    "packages/d2b/src/complete.rs",
    "packages/d2b/src/endpoint.rs",
    "packages/d2b/src/exec.rs",
    "packages/d2b/src/guest.rs",
    "packages/d2b/src/provider.rs",
    "packages/d2b/src/resource.rs",
    "packages/d2b/src/share.rs",
    "packages/d2b/src/shell.rs",
    "packages/d2b/src/zone.rs",
    "packages/d2b/src/zone_doctor.rs",
];

const RETIRED_STATE_MARKERS: &[&str] = &[
    "LegacyContext::from_env",
    "load_manifest(",
    "load_bundle_context(",
    "D2B_MANIFEST_PATH",
    "D2B_BUNDLE_PATH",
    "D2B_STATE_ROOT",
];
const RETIRED_STATE_BRIDGE_FILES: &[&str] = &[
    "packages/d2b/src/activation.rs",
    "packages/d2b/src/dispatch.rs",
    "packages/d2b/src/host.rs",
    "packages/d2b/src/legacy.rs",
];

fn retired_state_markers(path: &str, source: &str) -> Vec<String> {
    RETIRED_STATE_MARKERS
        .iter()
        .filter(|marker| source.contains(**marker))
        .map(|marker| format!("{path}: {marker}"))
        .collect()
}

#[test]
fn normal_zone_runtime_does_not_read_retired_state() {
    let violations = NORMAL_ZONE_RUNTIME_FILES
        .iter()
        .flat_map(|path| retired_state_markers(path, &read_repo_file(path)))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "normal Zone runtime imported a retired-state reader:\n{}",
        violations.join("\n")
    );
}

#[test]
fn retired_state_readers_remain_in_the_explicit_legacy_seam() {
    let legacy = read_repo_file("packages/d2b/src/legacy.rs");
    for marker in [
        "pub(crate) fn load_manifest",
        "pub(crate) fn load_bundle_context",
    ] {
        assert!(
            legacy.contains(marker),
            "the explicit legacy seam lost its bounded reader {marker}"
        );
    }
    assert!(
        !retired_state_markers(
            "packages/d2b/src/provider.rs",
            &read_repo_file("packages/d2b/src/provider.rs")
        )
        .iter()
        .any(|violation| violation.contains("LegacyContext")),
        "Provider inspection must not regain a legacy-state access seam"
    );
}

#[test]
fn retired_state_access_has_only_declared_bridge_owners() {
    let owners = git_listed_files(&["packages/d2b/src"])
        .into_iter()
        .filter(|path| path.ends_with(".rs"))
        .filter_map(|path| {
            let source = read_repo_file_opt(&path)?;
            (!retired_state_markers(&path, &source).is_empty()).then_some(path)
        })
        .collect::<BTreeSet<_>>();
    let expected = RETIRED_STATE_BRIDGE_FILES
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        owners, expected,
        "retired-state access escaped its declared bridge owners"
    );
}

#[test]
fn retired_state_policy_rejects_a_new_reader_marker() {
    let violations = retired_state_markers(
        "packages/d2b/src/provider.rs",
        "fn inspect() { let _ = LegacyContext::from_env(); }",
    );
    assert_eq!(
        violations,
        vec!["packages/d2b/src/provider.rs: LegacyContext::from_env".to_owned()]
    );
}
