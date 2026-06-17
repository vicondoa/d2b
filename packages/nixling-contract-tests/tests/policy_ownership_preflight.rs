//! Per-VM store-ownership source-lint guardrails, migrated from the source-grep
//! tail of `tests/per-vm-state-ownership-eval.sh`.
//!
//! The eval-time assertions over `nixling.daemon.perVmStateOwnershipMatrix`
//! live in the nix-unit corpus (`tests/unit/nix/cases/per-vm-state-ownership.nix`).
//! The bash gate ALSO carried source-level regression guards — that
//! `nixos-modules/store.nix` and the daemon's
//! `packages/nixlingd/src/ownership_preflight.rs` never re-introduce the legacy
//! `root:kvm` ownership / `2775` group-writable store mode. Those are
//! source-greps, not eval-time values, so they belong in the Rust policy layer
//! with the other `policy_*.rs` lints (this crate reads the real checkout via
//! `tests/tools/rust-workspace-checks.sh`, which is excluded from the hermetic Nix
//! sandbox).
//!
//! Faithful port of the bash gate's `grep`s:
//!   * store.nix keeps the canonical `chown nixlingd:users` / `chmod 0755`
//!     META_DIR fix-ups, and contains NO `chown root:kvm` / `chmod 2775`.
//!   * no nixos-modules file enforces `store store-meta` at mode `2775`.
//!   * ownership_preflight.rs declares NO `mode: 0o2775` on a `store*` path
//!     (within the 6-line window after each `path: "store` line, mirroring the
//!     bash `grep -A5`).

use nixling_contract_tests::{read_repo_file, repo_path_exists, repo_root};

const STORE_NIX: &str = "nixos-modules/store.nix";
const OWNERSHIP_PREFLIGHT_RS: &str = "packages/nixlingd/src/ownership_preflight.rs";

fn store_nix() -> String {
    assert!(
        repo_path_exists(STORE_NIX),
        "expected {STORE_NIX} to exist in the checkout",
    );
    read_repo_file(STORE_NIX)
}

/// Recursively collect every regular file under `nixos-modules/`.
fn nixos_module_files() -> Vec<std::path::PathBuf> {
    let root = repo_root().join("nixos-modules");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// store.nix keeps the canonical META_DIR ownership fix-up.
// ---------------------------------------------------------------------------
#[test]
fn store_sync_chowns_meta_dirs_to_nixlingd_users() {
    let content = store_nix();
    let needle = r#"find "$META_DIR" -type d -exec chown nixlingd:users {} +"#;
    assert!(
        content.lines().any(|l| l.contains(needle)),
        "{STORE_NIX} must keep the canonical META_DIR chown fix-up: `{needle}`",
    );
}

#[test]
fn store_sync_chmods_meta_dirs_0755() {
    let content = store_nix();
    let needle = r#"find "$META_DIR" -type d -exec chmod 0755 {} +"#;
    assert!(
        content.lines().any(|l| l.contains(needle)),
        "{STORE_NIX} must keep the canonical META_DIR chmod 0755 fix-up: `{needle}`",
    );
}

// ---------------------------------------------------------------------------
// store.nix must NOT carry the legacy root:kvm / 2775 store enforcement.
// ---------------------------------------------------------------------------
#[test]
fn store_sync_has_no_legacy_root_kvm_ownership() {
    let content = store_nix();
    assert!(
        !content.lines().any(|l| l.contains("chown root:kvm")),
        "{STORE_NIX} still contains legacy `chown root:kvm` store ownership fix-up",
    );
}

#[test]
fn store_sync_has_no_2775_store_mode() {
    let content = store_nix();
    assert!(
        !content.lines().any(|l| l.contains("chmod 2775")),
        "{STORE_NIX} must not grant group-write (`chmod 2775`) on store/store-meta directories",
    );
}

// ---------------------------------------------------------------------------
// No nixos-modules file enforces `store store-meta` at mode 2775.
// ---------------------------------------------------------------------------
#[test]
fn nixos_modules_have_no_store_store_meta_2775_enforcement() {
    let mut offenders = Vec::new();
    for path in nixos_module_files() {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            if line.contains("store store-meta")
                && (line.contains("--mode 2775") || line.contains("chmod 2775"))
            {
                offenders.push(format!("{}: {}", path.display(), line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "nixos-modules still contain store/store-meta 2775 enforcement:\n{}",
        offenders.join("\n"),
    );
}

// ---------------------------------------------------------------------------
// The daemon canonical ownership preflight declares no store* path at 2775.
//
// Mirrors the bash gate's `grep -n 'path: "store' ownership_preflight.rs -A5 |
// grep -Fq 'mode: 0o2775'`: scan a 6-line window (the matching line plus the
// next five) after each `path: "store` declaration.
// ---------------------------------------------------------------------------
#[test]
fn daemon_ownership_preflight_has_no_store_2775_mode() {
    assert!(
        repo_path_exists(OWNERSHIP_PREFLIGHT_RS),
        "expected {OWNERSHIP_PREFLIGHT_RS} to exist in the checkout",
    );
    let content = read_repo_file(OWNERSHIP_PREFLIGHT_RS);
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains(r#"path: "store"#) {
            let end = (i + 6).min(lines.len());
            for window_line in &lines[i..end] {
                assert!(
                    !window_line.contains("mode: 0o2775"),
                    "daemon canonical ownership preflight ({OWNERSHIP_PREFLIGHT_RS}) still \
                     expects store/store-meta 2775 (found `mode: 0o2775` within 6 lines of a \
                     `path: \"store` declaration at line {})",
                    i + 1,
                );
            }
        }
    }
}
