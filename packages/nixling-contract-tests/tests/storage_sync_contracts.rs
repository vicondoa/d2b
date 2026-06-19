use std::{collections::BTreeSet, env, fs, path::PathBuf};

use nixling_contract_tests::read_repo_file;
use nixling_core::{
    processes::ProcessesJson,
    storage::{StorageJson, StoragePathKind},
    sync::{LockKind, SyncJson},
};

#[test]
fn storage_and_sync_emitters_are_wired_into_private_bundle() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./storage-json.nix"),
        "default.nix must import storage-json.nix"
    );
    assert!(
        default_nix.contains("./sync-json.nix"),
        "default.nix must import sync-json.nix"
    );

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "storagePath = \"/etc/nixling/storage.json\";",
        "syncPath = \"/etc/nixling/sync.json\";",
        "key = \"/etc/nixling/storage.json\";",
        "key = \"/etc/nixling/sync.json\";",
    ] {
        assert!(
            bundle_nix.contains(needle),
            "bundle.nix missing storage/sync wiring: {needle}"
        );
    }

    let bundle_doc = read_repo_file("docs/reference/manifest-bundle.md");
    assert!(bundle_doc.contains("`storage.json`"));
    assert!(bundle_doc.contains("`sync.json`"));
}

#[test]
fn storage_and_sync_schemas_are_committed_and_closed() {
    let storage_schema = read_repo_file("docs/reference/schemas/v2/storage.json");
    let sync_schema = read_repo_file("docs/reference/schemas/v2/sync.json");
    for (name, schema) in [
        ("storage.json", storage_schema.as_str()),
        ("sync.json", sync_schema.as_str()),
    ] {
        assert!(
            schema.contains("\"additionalProperties\": false"),
            "{name} must deny unknown fields"
        );
    }
    assert!(storage_schema.contains("\"adoption-quarantined\""));
    assert!(storage_schema.contains("\"tamper-evident-segmented\""));
    assert!(sync_schema.contains("\"ofd\""));
    assert!(sync_schema.contains("\"scm-rights\""));
    assert!(sync_schema.contains("\"explicit-fd-mapping\""));
}

#[test]
fn storage_lifecycle_report_schema_and_reference_are_committed() {
    let schema = read_repo_file("docs/reference/schemas/v2/storage-lifecycle-report.json");
    let reference = read_repo_file("docs/reference/storage-lifecycle-report.md");
    let xtask = read_repo_file("packages/xtask/src/main.rs");
    let schema: serde_json::Value =
        serde_json::from_str(&schema).expect("storage lifecycle report schema parses as JSON");

    assert_eq!(schema["title"], "StorageLifecycleReport");
    assert!(schema["properties"].get("schemaVersion").is_some());

    let issue_variants = schema["definitions"]["StorageLifecycleIssue"]["oneOf"]
        .as_array()
        .expect("StorageLifecycleIssue oneOf variants");
    let legacy_variant = issue_variants
        .iter()
        .find(|variant| {
            variant["properties"]["kind"]["enum"]
                .as_array()
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| value == "legacy-bundle-contracts-unavailable")
                })
        })
        .expect("legacy bundle issue variant in schema");
    assert!(legacy_variant["properties"].get("bundleVersion").is_some());
    assert!(legacy_variant["properties"].get("bundle_version").is_none());

    for kind in ["missing-restart-policy", "adoptable-missing-cgroup-leaf"] {
        let role_variant = issue_variants
            .iter()
            .find(|variant| {
                variant["properties"]["kind"]["enum"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| value == kind))
            })
            .unwrap_or_else(|| panic!("{kind} issue variant in schema"));
        assert!(role_variant["properties"].get("roleId").is_some());
        assert!(role_variant["properties"].get("role_id").is_none());
    }

    assert!(reference.contains("/var/lib/nixling/daemon-state/storage-lifecycle-report.json"));
    assert!(reference.contains("./schemas/v2/storage-lifecycle-report.json"));
    assert!(xtask.contains("\"storage-lifecycle-report.json\""));
}

#[test]
fn rendered_storage_contract_covers_process_writable_paths_when_fixture_available() {
    let Some(dir) = env::var_os("NL_FIXTURES").map(PathBuf::from) else {
        eprintln!("  (skipping rendered storage/sync contract check; NL_FIXTURES unset)");
        return;
    };
    let storage: StorageJson = read_json(&dir, "storage.json");
    let sync: SyncJson = read_json(&dir, "sync.json");
    let processes: ProcessesJson = read_json(&dir, "processes.json");

    storage
        .validate_unique_ids()
        .expect("rendered storage contract must have unique ids");
    sync.validate_lock_order()
        .expect("rendered sync contract must have valid lock order");

    let storage_paths: BTreeSet<&str> = storage
        .paths
        .iter()
        .map(|path| path.path_template.as_str())
        .collect();
    for dag in &processes.vms {
        for node in &dag.nodes {
            let restart = storage.restart_policies.iter().find(|policy| {
                policy.vm.as_str() == dag.vm && policy.role_id.as_str() == node.id.0
            });
            assert!(
                restart.is_some(),
                "missing restart policy for {}:{}",
                dag.vm,
                node.id.0
            );
            for writable in &node.profile.mount_policy.writable_paths {
                assert!(
                    storage_paths.contains(writable.path.as_str()),
                    "storage.json missing writable path for {}:{} -> {}",
                    dag.vm,
                    node.id.0,
                    writable.path
                );
            }
        }
    }

    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.path_template.as_str() == "/etc/nixling/storage.json"),
        "storage.json must describe itself as a private bundle artifact"
    );
    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.path_template.as_str() == "/etc/nixling/sync.json"),
        "storage.json must describe sync.json as a private bundle artifact"
    );
    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.kind == StoragePathKind::UnixSocket),
        "storage.json should include role/readiness Unix socket paths"
    );
    assert!(
        sync.locks
            .iter()
            .any(|lock| lock.kind == LockKind::Ofd && lock.cloexec_required),
        "sync.json must include at least one O_CLOEXEC OFD lock"
    );
}

fn read_json<T: serde::de::DeserializeOwned>(dir: &std::path::Path, name: &str) -> T {
    let path = dir.join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!(
            "failed to parse {} as {}: {err}",
            path.display(),
            std::any::type_name::<T>()
        )
    })
}
