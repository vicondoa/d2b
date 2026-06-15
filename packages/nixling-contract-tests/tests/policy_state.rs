//! Group-rename / state-dir / store-sync policy-lint gates (the "H-group"),
//! migrated from the `tests/*-eval.sh` bash gates. Each test reads the real
//! repo files (via the `nixling_contract_tests::read_repo_file` helper) and
//! asserts a structural/source invariant — all are grep-only gates over
//! `nixos-modules/**` (no Nix eval/build). This crate runs only from
//! `tests/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access — and
//! shelling out to `git` for the gitignore-respecting file enumeration the
//! bash gates got from `grep -R` — is sound here.
//!
//! Migrated gates:
//!   * tests/group-rename-semantic-eval.sh         -> group_rename_semantic
//!   * tests/group-migration-fresh-install-eval.sh -> group_migration_fresh_install
//!   * tests/state-dir-acl-eval.sh                 -> state_dir_acl
//!   * tests/store-sync-export-eval.sh             -> store_sync_export
//!
//! NOTE: the retired launcher group-name literals (`nixling-` + `launcher{,s}`)
//! are assembled from fragments throughout this file so that the contract
//! crate's `legacy_group_name_denylist` gate (which scans `packages/**` but
//! does NOT self-allowlist this file) cannot trip on them.

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

/// Read a repo-relative file, returning `None` when absent or not valid UTF-8
/// (binary files are skipped, mirroring `grep -I`).
fn read_repo_file_opt(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join(rel)).ok()
}

/// Enumerate repo-relative tracked + untracked-non-ignored files under the
/// given pathspecs via `git ls-files`. The original `group-rename-semantic`
/// gate used `grep -R` over `nixos-modules`; `git ls-files` mirrors that while
/// skipping build artifacts under `target/` (the helper crate's dev cache).
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

/// `grep -Eq "$pattern" "$file"`: whether any line of `haystack` matches
/// `pattern`. Applied per line so POSIX `[[:space:]]` (which includes newline
/// in the `regex` crate) cannot accidentally span lines, matching grep.
fn line_matches(haystack: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).unwrap_or_else(|e| panic!("invalid regex /{pattern}/: {e}"));
    haystack.lines().any(|l| re.is_match(l))
}

// ---------------------------------------------------------------------------
// Migrated from tests/group-rename-semantic-eval.sh.
//
// Asserts the canonical `nixling` lifecycle-group rename semantics: the group
// is declared, launcher users join it, the daemon's publicSocketGroup is
// `nixling`, the state-dir ACL launcher group is `nixling`, the ACL helper
// renders the group traversal grant, and the retired legacy group names survive
// only as empty migration tombstones (never in any `extraGroups` list).
// ---------------------------------------------------------------------------
#[test]
fn group_rename_semantic() {
    let host_users = read_repo_file("nixos-modules/host-users.nix");
    let host_daemon = read_repo_file("nixos-modules/host-daemon.nix");
    let host_activation = read_repo_file("nixos-modules/host-activation.nix");
    let acl_helper = read_repo_file("nixos-modules/host-activation.d/state-dir-acl.sh");

    // Legacy group-name literals are assembled from fragments so this test file
    // does not itself trip the legacy-group-name denylist gate.
    let legacy = concat!("nixling-", "launcher"); // singular tombstone group name
    let legacy_plural = concat!("nixling-", "launchers"); // plural tombstone group name

    require_match(
        &host_users,
        r"nixling = \{ \};",
        "users.groups.nixling declaration missing",
    );
    require_match(
        &host_users,
        r#"extraGroups = \[ "nixling" \];"#,
        "launcherUsers are not added to nixling",
    );
    require_match(
        &host_daemon,
        r#"publicSocketGroup = "nixling";"#,
        "daemon publicSocketGroup is not nixling",
    );
    require_match(
        &host_activation,
        r"LAUNCHER_GROUP=nixling",
        "state-dir ACL launcher group is not nixling",
    );
    require_match(
        &acl_helper,
        r"g:\$LAUNCHER_GROUP:--x",
        "ACL helper does not render g:<launcher-group>:",
    );
    require_match(
        &host_users,
        &[legacy, r" = \{ \};"].concat(),
        "singular legacy tombstone missing",
    );
    require_match(
        &host_daemon,
        &[legacy_plural, r" = \{ \};"].concat(),
        "plural legacy tombstone missing",
    );

    // Negative: the legacy group must not appear in any `extraGroups` list
    // under nixos-modules.
    let neg = [r#"extraGroups = \[[^]]*""#, legacy, r#"(s)?""#].concat();
    let neg_re = Regex::new(&neg).expect("valid extraGroups denylist regex");
    let mut violations: Vec<String> = Vec::new();
    for rel in git_listed_files(&["nixos-modules"]) {
        let Some(content) = read_repo_file_opt(&rel) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if neg_re.is_match(line) {
                violations.push(format!("{rel}:{}:{line}", idx + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "group-rename-semantic: legacy group still appears in extraGroups:\n{}",
        violations.join("\n")
    );
}

/// `require()` from the bash gate: assert a line of `haystack` matches
/// `pattern`, failing with the gate's descriptive message.
fn require_match(haystack: &str, pattern: &str, msg: &str) {
    assert!(
        line_matches(haystack, pattern),
        "group-rename-semantic: FAIL — {msg} (no line matched /{pattern}/)"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/group-migration-fresh-install-eval.sh.
//
// Phase B fresh-install gate: the legacy-gid migration activation block must be
// present, guarded by root-existence checks, route through the fd-safe
// `chgrp-by-numeric-gid` helper (which uses O_DIRECTORY|O_NOFOLLOW,
// F_OFD_SETLK, AT_SYMLINK_NOFOLLOW), and must NOT use a raw path-based `chgrp`
// fallback inside the migration block.
// ---------------------------------------------------------------------------
#[test]
fn group_migration_fresh_install() {
    let activation_rel = "nixos-modules/host-activation.nix";
    let helper_rel = "nixos-modules/host-activation-helper/src/main.rs";
    assert!(
        repo_path_exists(activation_rel) && repo_path_exists(helper_rel),
        "group-migration-fresh-install: FAIL — migration module/helper missing"
    );
    let activation = read_repo_file(activation_rel);
    let helper = read_repo_file(helper_rel);

    for needle in [
        "system.activationScripts.nixlingGroupMigration",
        r#"lib.stringAfter [ "users" ]"#,
        r#"[ -e "$root" ] || continue"#,
        "chgrp-by-numeric-gid",
        "--skip-while-lock-held /run/nixling/daemon.lock",
        r#"lib.stringAfter [ "users" "nixlingGroupMigration" ]"#,
    ] {
        assert!(
            activation.contains(needle),
            "group-migration-fresh-install: FAIL — missing {needle} in {activation_rel}"
        );
    }
    for needle in [
        "O_DIRECTORY | libc::O_NOFOLLOW",
        "libc::F_OFD_SETLK",
        "libc::AT_SYMLINK_NOFOLLOW",
    ] {
        assert!(
            helper.contains(needle),
            "group-migration-fresh-install: FAIL — missing {needle} in {helper_rel}"
        );
    }

    // awk port: extract the nixlingGroupMigration block (from the line declaring
    // it through the first `    '';` close, inclusive) and assert no raw
    // `chgrp` (a whole-word `chgrp`, not `chgrp-by-numeric-gid`) appears.
    let raw_chgrp = Regex::new(r"(^|[[:space:]])chgrp([[:space:]]|$)").unwrap();
    let block_end = Regex::new(r"^    '';").unwrap();
    let mut in_block = false;
    let mut raw_chgrp_line: Option<String> = None;
    for line in activation.lines() {
        if line.contains("system.activationScripts.nixlingGroupMigration") {
            in_block = true;
        }
        if in_block {
            if raw_chgrp.is_match(line) {
                raw_chgrp_line = Some(line.to_string());
                break;
            }
            if block_end.is_match(line) {
                break;
            }
        }
    }
    assert!(
        raw_chgrp_line.is_none(),
        "group-migration-fresh-install: FAIL — raw chgrp found in migration module: {:?}",
        raw_chgrp_line
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/state-dir-acl-eval.sh.
//
// v1.1 invariant gate over `/var/lib/nixling` state-dir posture:
//   (a) declared `0750 root nixlingd` in host-daemon.nix tmpfiles (NOT 0755);
//   (b) the `nixlingStateDirAcl` activation script grants per-user/per-group
//       `--x` traversal (incl. `kvm` as a GROUP and the `nixling` lifecycle
//       group, the latter possibly via the sourced state-dir-acl.sh helper);
//   (c) no `setfacl -d -m` default ACL is applied at the state-dir root.
// ---------------------------------------------------------------------------
#[test]
fn state_dir_acl() {
    let daemon = read_repo_file("nixos-modules/host-daemon.nix");
    let activation = read_repo_file("nixos-modules/host-activation.nix");
    let acl_helper = read_repo_file("nixos-modules/host-activation.d/state-dir-acl.sh");

    let mut failures: Vec<String> = Vec::new();

    // (a) 0750 declaration present.
    if !line_matches(&daemon, r#""d /var/lib/nixling 0750 root nixlingd"#) {
        failures.push("/var/lib/nixling not declared `0750 root nixlingd`".into());
    }
    // (a) 0755 workaround absent (in either module).
    if line_matches(&daemon, r#""d /var/lib/nixling 0755"#)
        || line_matches(&activation, r#""d /var/lib/nixling 0755"#)
    {
        failures.push("found `0755 /var/lib/nixling` workaround".into());
    }
    // (b) nixlingStateDirAcl activation script present.
    if !activation.contains("nixlingStateDirAcl") {
        failures.push("nixlingStateDirAcl activation script missing".into());
    }
    // (b) setfacl `u:<user>:--x` traversal grant on the state dir.
    if !line_matches(
        &activation,
        r#"setfacl.*"u:[^"]+:--x".*(\$state_dir|var/lib/nixling)"#,
    ) {
        failures.push("no `setfacl -m \"u:<user>:--x\" <state-dir>` invocation found".into());
    }
    // (b) `kvm` is a GROUP not a USER: enforce a `g:kvm:--x` grant.
    if !line_matches(&activation, r#"setfacl.*"g:kvm:--x""#) {
        failures.push("`setfacl -m \"g:kvm:--x\"` grant missing (kvm is a Linux group)".into());
    }
    // (b) `nixling` traversal grant: either a direct `g:nixling(-<legacy>)?:--x`
    // setfacl in the activation module, or the activation module sources the
    // state-dir-acl.sh helper which renders `g:$LAUNCHER_GROUP:--x`.
    let direct_pat = [r#"setfacl.*"g:nixling(-"#, "launcher", r#")?:--x""#].concat();
    let via_helper = activation.contains("host-activation.d/state-dir-acl.sh")
        && acl_helper.contains(r#""g:$LAUNCHER_GROUP:--x""#);
    if !(line_matches(&activation, &direct_pat) || via_helper) {
        failures
            .push("`setfacl -m \"g:nixling:--x\" /var/lib/nixling` traversal grant missing".into());
    }
    // (c) NO `setfacl -d -m` default ACL on the state-dir root (it would widen
    // per-VM subdir surface). Comment lines are excluded, matching the bash
    // `grep -v '^[[:space:]]*#'` filter.
    let default_acl = Regex::new(
        r#"setfacl[[:space:]]+-d[[:space:]]+-m[[:space:]]+"[^"]+".*(\$state_dir|/var/lib/nixling[^/])"#,
    )
    .unwrap();
    let comment = Regex::new(r"^[[:space:]]*#").unwrap();
    for line in activation.lines() {
        if default_acl.is_match(line) && !comment.is_match(line) {
            failures.push(format!(
                "found `setfacl -d -m` default ACL on /var/lib/nixling root: {line}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "state-dir-acl: FAIL —\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/store-sync-export-eval.sh.
//
// Static gate for the StoreSync-only observability export wiring on the HOST
// side (`nixos-modules/components/observability/host.nix`): the host collector
// reads `<stateDir>/observability/store-sync/store-sync-*.jsonl` via a
// dedicated filelog receiver + pipeline forwarded over OTLP, never touches the
// unified broker audit log or privileged daemon socket, promotes only the host
// singleton resource attrs plus `source=store-sync-audit` (never target_vm /
// target_env), and grants the collector focused ACLs on the export dir only.
// ---------------------------------------------------------------------------
#[test]
fn store_sync_export() {
    let host_rel = "nixos-modules/components/observability/host.nix";
    assert!(
        repo_path_exists(host_rel),
        "store-sync-export: FAIL — missing required file: {host_rel}"
    );
    let host = read_repo_file(host_rel);
    // Comment-free view so a `#`-comment can neither satisfy a positive check
    // nor trip a negative one (port of the bash `code_only()` helper).
    let code = code_only(&host);

    let wants: &[(&str, &str)] = &[
        (
            "export dir resolves under <stateDir>/observability/store-sync",
            r#"storeSyncExportDir[[:space:]]*=.*/observability/store-sync""#,
        ),
        (
            "export glob targets store-sync-*.jsonl rotation shape",
            r#"storeSyncExportGlob[[:space:]]*=.*/store-sync-\*\.jsonl""#,
        ),
        (
            "host collector uses a filelog receiver for StoreSync export",
            r#""filelog/store_sync_audit"[[:space:]]*="#,
        ),
        (
            "filelog receiver includes the StoreSync export glob",
            r#"include[[:space:]]*=[[:space:]]*\[[[:space:]]*storeSyncExportGlob[[:space:]]*\]"#,
        ),
        (
            "filelog receiver parses JSON log bodies",
            r#"type[[:space:]]*=[[:space:]]*"json_parser""#,
        ),
        (
            "StoreSync logs have a dedicated OTel pipeline",
            r#"pipelines\."logs/store_sync_audit"[[:space:]]*="#,
        ),
        (
            "StoreSync logs forward to the existing OTLP exporter",
            r#"exporters[[:space:]]*=[[:space:]]*\[[[:space:]]*"otlp"[[:space:]]*\]"#,
        ),
        (
            "StoreSync resource marks vm.name as host",
            r#"key[[:space:]]*=[[:space:]]*"vm.name";[[:space:]]*value[[:space:]]*=[[:space:]]*"host""#,
        ),
        (
            "StoreSync resource marks vm.env as host",
            r#"key[[:space:]]*=[[:space:]]*"vm.env";[[:space:]]*value[[:space:]]*=[[:space:]]*"host""#,
        ),
        (
            "StoreSync resource marks vm.role as host",
            r#"key[[:space:]]*=[[:space:]]*"vm.role";[[:space:]]*value[[:space:]]*=[[:space:]]*"host""#,
        ),
        (
            "StoreSync resource marks service.name as nixling-store-sync",
            r#"key[[:space:]]*=[[:space:]]*"service.name";[[:space:]]*value[[:space:]]*=[[:space:]]*"nixling-store-sync""#,
        ),
        (
            "StoreSync resource marks source as store-sync-audit",
            r#"key[[:space:]]*=[[:space:]]*"source";[[:space:]]*value[[:space:]]*=[[:space:]]*"store-sync-audit""#,
        ),
        (
            "collector gets traverse (--x) on the state dir",
            r#"setfacl -m "u:nixling-host-otel-collector:--x" "\$state_dir""#,
        ),
        (
            "collector gets traverse (--x) on the observability dir",
            r#"setfacl -m "u:nixling-host-otel-collector:--x" "\$obs_dir""#,
        ),
        (
            "collector gets read+traverse (r-x) on the export dir",
            r#"setfacl -m "u:nixling-host-otel-collector:r-x" "\$export_dir""#,
        ),
        (
            "rotated export files inherit collector read via a default ACL",
            r#"setfacl -d -m "u:nixling-host-otel-collector:r--" "\$export_dir""#,
        ),
    ];

    let denies: &[(&str, &str)] = &[
        (
            "host collector never references the broker audit log path",
            r#"audit/broker"#,
        ),
        ("host collector never globs broker-*.jsonl", r#"broker-\*"#),
        (
            "host collector never references the privileged daemon socket",
            r#"priv\.sock"#,
        ),
        (
            "target_vm is NOT promoted to a resource attribute",
            r#"key[[:space:]]*=[[:space:]]*"target_vm""#,
        ),
        (
            "target_env is NOT promoted to a resource attribute",
            r#"key[[:space:]]*=[[:space:]]*"target_env""#,
        ),
        (
            "no collector ACL is granted on any audit path",
            r#"setfacl.*nixling-host-otel-collector.*audit"#,
        ),
    ];

    let mut failures: Vec<String> = Vec::new();
    for (label, pat) in wants {
        if !line_matches(&code, pat) {
            failures.push(format!("want FAIL: {label} (missing /{pat}/)"));
        }
    }
    for (label, pat) in denies {
        if line_matches(&code, pat) {
            failures.push(format!("deny FAIL: {label} (found /{pat}/)"));
        }
    }

    assert!(
        failures.is_empty(),
        "store-sync-export: FAIL —\n{}",
        failures.join("\n")
    );
}

/// Port of the bash `code_only()` helper: drop full-line `#` comments and strip
/// trailing ` #...` inline comments, so a comment can neither satisfy a `want`
/// nor trip a `deny`. Mirrors `sed -e 's/[[:space:]]#.*$//' -e '/^[[:space:]]*#/d'`
/// (substitution first, then full-comment-line deletion).
fn code_only(content: &str) -> String {
    let trailing = Regex::new(r"[[:space:]]#.*$").unwrap();
    let full_comment = Regex::new(r"^[[:space:]]*#").unwrap();
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let stripped = trailing.replace(line, "").into_owned();
        if full_comment.is_match(&stripped) {
            continue;
        }
        out.push(stripped);
    }
    out.join("\n")
}
