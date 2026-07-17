//! Source-policy guardrails for realm/workload store-view ownership.

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};

const STORE_NIX: &str = "nixos-modules/store.nix";
const STORAGE_ROWS_NIX: &str = "nixos-modules/realm-storage-rows.nix";
const OWNERSHIP_PREFLIGHT_RS: &str = "packages/d2bd/src/ownership_preflight.rs";

fn source(path: &str) -> String {
    assert!(repo_path_exists(path), "expected {path} to exist");
    read_repo_file(path)
}

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

#[test]
fn store_view_rows_are_broker_owned() {
    let store = source(STORE_NIX);
    for needle in [
        "realmStorageRows = import ./realm-storage-rows.nix",
        r#"lib.hasSuffix "/store-view-live" row.id"#,
        r#"row.creator.kind == "broker""#,
        r#"row.repairPolicy == "broker-reconcile""#,
    ] {
        assert!(
            store.contains(needle),
            "{STORE_NIX} missing realm storage-row integration: {needle}"
        );
    }

    let rows = source(STORAGE_ROWS_NIX);
    for needle in [
        "creator = brokerActor realmId;",
        "writers ? [ (brokerActor realmId) ]",
        "repairPolicy ? \"broker-reconcile\"",
        "recursive = false;",
        "id = (normalized \"workload-store-view-live\").resourceId;",
    ] {
        assert!(
            rows.contains(needle),
            "{STORAGE_ROWS_NIX} missing broker-owned store-view policy: {needle}"
        );
    }
}

#[test]
fn store_view_rows_require_hardlink_invariants() {
    let store = source(STORE_NIX);
    for needle in [
        "hasInvariant \"same-filesystem\" row",
        "hasInvariant \"hardlink-farm-no-recursion\" row",
        "hasInvariant \"no-recursive-mutation\" row",
        "row.creator.kind == \"broker\"",
        "row.repairPolicy == \"broker-reconcile\"",
        "row.recursive == false",
        "hard_linkFarmRoot != \"/nix/store\"",
    ] {
        assert!(
            store.contains(needle),
            "{STORE_NIX} missing store-view hardlink guard: {needle}"
        );
    }
}

#[test]
fn store_module_has_no_activation_repair() {
    let store = source(STORE_NIX);
    for forbidden in [
        "system.activationScripts",
        "systemd.tmpfiles.rules",
        "META_DIR",
        "find ",
        "chown ",
        "chmod ",
        "setfacl ",
    ] {
        assert!(
            !store.contains(forbidden),
            "{STORE_NIX} must not repair broker-owned store trees: {forbidden}"
        );
    }
}

#[test]
fn store_sync_has_no_legacy_root_kvm_or_2775_ownership() {
    let store = source(STORE_NIX);
    for forbidden in ["chown root:kvm", "chmod 2775", "mode = \"2775\""] {
        assert!(
            !store.contains(forbidden),
            "{STORE_NIX} contains legacy store ownership policy: {forbidden}"
        );
    }
}

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

#[test]
fn daemon_ownership_preflight_has_no_store_2775_mode() {
    let content = source(OWNERSHIP_PREFLIGHT_RS);
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains(r#"path: "store"#) {
            let end = (i + 6).min(lines.len());
            assert!(
                lines[i..end]
                    .iter()
                    .all(|window_line| !window_line.contains("mode: 0o2775")),
                "{OWNERSHIP_PREFLIGHT_RS} expects store mode 0o2775 near line {}",
                i + 1
            );
        }
    }
}
