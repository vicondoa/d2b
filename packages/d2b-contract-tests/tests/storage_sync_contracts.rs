use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::{read_repo_file, repo_root};
use d2b_core::{
    bundle::Bundle,
    processes::ProcessesJson,
    realm_controller_config::{RealmControllerRuntimeState, RealmControllersJson},
    storage::{
        ActorKind, PrincipalKind, RepairPolicy, SensitivityClass, StorageInvariant, StorageJson,
        StoragePathKind,
    },
    sync::{LockKind, SyncJson},
};
use regex::Regex;

#[test]
fn storage_and_sync_emitters_are_wired_into_private_bundle() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./bundle-artifacts.nix"),
        "default.nix must import the canonical bundle artifact composer"
    );

    let artifacts_nix = read_repo_file("nixos-modules/bundle-artifacts.nix");
    for needle in [
        "realmStorageRows = import ./realm-storage-rows.nix",
        "paths = realmStorageRows.paths;",
        "locks = realmStorageRows.locks;",
        "installFileName = \"storage.json\";",
        "installFileName = \"sync.json\";",
    ] {
        assert!(
            artifacts_nix.contains(needle),
            "bundle-artifacts.nix missing realm storage/sync wiring: {needle}"
        );
    }

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "storagePath = \"/etc/d2b/storage.json\";",
        "syncPath = \"/etc/d2b/sync.json\";",
        "key = \"/etc/d2b/storage.json\";",
        "key = \"/etc/d2b/sync.json\";",
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
fn legacy_storage_emitters_delegate_to_realm_rows_without_vm_repair() {
    for (path, projection) in [
        (
            "nixos-modules/storage-json.nix",
            "paths = realmStorageRows.paths;",
        ),
        (
            "nixos-modules/sync-json.nix",
            "locks = realmStorageRows.locks;",
        ),
    ] {
        let source = read_repo_file(path);
        for needle in [
            "realmStorageRows = import ./realm-storage-rows.nix",
            projection,
            "schemaVersion = \"v2\";",
        ] {
            assert!(source.contains(needle), "{path} missing `{needle}`");
        }
        for forbidden in [
            "cfg.vms",
            "cfg.envs",
            "scope = \"vm:",
            "scope = \"env:",
            "/run/d2b/vms/",
            "cfg.store.stateDir",
            "nix-activation",
            "actor \"nix-module\" \"tmpfiles\"",
        ] {
            assert!(
                !source.contains(forbidden),
                "{path} retains legacy VM/env storage repair: {forbidden}"
            );
        }
    }
}

#[test]
fn allocator_artifact_is_wired_into_private_bundle() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./allocator-json.nix"),
        "default.nix must import allocator-json.nix"
    );

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    assert!(
        bundle_artifacts.contains("allocatorJson"),
        "bundle-artifacts.nix must declare allocatorJson metadata"
    );

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "allocatorPath = \"/etc/d2b/allocator.json\";",
        "key = \"/etc/d2b/allocator.json\";",
    ] {
        assert!(
            bundle_nix.contains(needle),
            "bundle.nix missing allocator wiring: {needle}"
        );
    }

    let allocator_nix = read_repo_file("nixos-modules/allocator-json.nix");
    assert!(allocator_nix.contains("runtimeState = \"metadata-only\";"));
    assert!(allocator_nix.contains("spawnsService = false;"));
    assert!(allocator_nix.contains("classification = \"contractPrivateNonSecret\";"));
    assert!(allocator_nix.contains("sensitivity = \"nonSecret\";"));

    let bundle_doc = read_repo_file("docs/reference/manifest-bundle.md");
    assert!(bundle_doc.contains("`allocator.json`"));
}

#[test]
fn realm_controller_artifact_is_wired_into_private_bundle() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./realm-controller-config-json.nix"),
        "default.nix must import realm-controller-config-json.nix"
    );

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    assert!(
        bundle_artifacts.contains("realmControllersJson"),
        "bundle-artifacts.nix must declare realmControllersJson metadata"
    );
    assert!(
        !bundle_artifacts.contains("d2bRealmBundleArtifactAcls"),
        "realm private artifacts must not add a parallel activation ACL hook"
    );

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "realmControllersPath = \"/etc/d2b/realm-controllers.json\";",
        "key = \"/etc/d2b/realm-controllers.json\";",
    ] {
        assert!(
            bundle_nix.contains(needle),
            "bundle.nix missing realm-controller wiring: {needle}"
        );
    }

    let controller_nix = read_repo_file("nixos-modules/realm-controller-config-json.nix");
    let storage_rows = read_repo_file("nixos-modules/realm-storage-rows.nix");
    assert!(controller_nix.contains("installFileName = \"realm-controllers.json\";"));
    assert!(controller_nix.contains("classification = \"contractPrivateNonSecret\";"));
    assert!(controller_nix.contains("sensitivity = \"nonSecret\";"));
    assert!(controller_nix.contains("runtimeState = \"metadata-only\";"));
    assert!(controller_nix.contains("preservesDirectUnixSocketSemantics = true;"));
    assert!(storage_rows.contains("[ \"controller\" \"providers\" \"storage\" \"sync\" ]"));
    assert!(storage_rows.contains("path = \"${configRoot}/${file}.json\";"));
    assert!(storage_rows.contains("owner = brokerUser args.realmId;"));
}

#[test]
fn rendered_allocator_contract_shape_and_private_bundle_path_when_fixture_available() {
    let Some(dir) = env::var_os("D2B_FIXTURES").map(PathBuf::from) else {
        eprintln!("  (skipping rendered allocator contract check; D2B_FIXTURES unset)");
        return;
    };

    let allocator: serde_json::Value = read_json(&dir, "allocator.json");
    let bundle: Bundle = read_json(&dir, "bundle.json");

    assert_eq!(allocator["schemaVersion"], "v2");
    assert_eq!(allocator["allocator"]["runtimeState"], "metadata-only");
    assert_eq!(allocator["allocator"]["runtime"]["spawnsService"], false);
    assert_eq!(allocator["allocator"]["runtime"]["socketActivated"], false);
    assert!(allocator["allocator"]["runtime"]["serviceName"].is_null());
    assert_eq!(
        allocator["allocator"]["rootSocket"],
        "/run/d2b/allocator/local-root.sock"
    );
    assert_eq!(
        allocator["allocator"]["leaseLedger"],
        "/var/lib/d2b/allocator/leases.jsonl"
    );
    assert_eq!(allocator["invariants"]["noRuntimeAllocatorService"], true);
    assert_eq!(
        allocator["invariants"]["preservesEnvRuntimeSourceOfTruth"],
        true
    );
    assert_eq!(allocator["invariants"]["privateMetadataOnly"], true);

    let mut resource_ids = BTreeSet::new();
    for request in allocator["resourceRequests"]
        .as_array()
        .expect("allocator resource requests")
    {
        let resource_id = request["resourceId"]
            .as_str()
            .expect("allocator resource id");
        assert!(
            resource_ids.insert(resource_id),
            "allocator resource id rendered more than once: {resource_id}"
        );
        assert!(
            request["realmPath"]
                .as_str()
                .is_some_and(|realm| !realm.is_empty()),
            "allocator resource requests must remain rooted in a realm path"
        );
    }

    assert_eq!(
        bundle.allocator_path.as_deref(),
        Some("/etc/d2b/allocator.json")
    );
    assert!(
        bundle
            .artifact_hashes
            .as_ref()
            .is_some_and(|hashes| hashes.contains_key("/etc/d2b/allocator.json")),
        "bundle integrity table must cover allocator.json"
    );
}

#[test]
fn rendered_realm_controller_contract_shape_when_fixture_available() {
    let Some(dir) = env::var_os("D2B_FIXTURES").map(PathBuf::from) else {
        eprintln!("  (skipping rendered realm-controller contract check; D2B_FIXTURES unset)");
        return;
    };

    let controllers: RealmControllersJson = read_json(&dir, "realm-controllers.json");
    let bundle: Bundle = read_json(&dir, "bundle.json");
    let storage: StorageJson = read_json(&dir, "storage.json");

    assert_eq!(controllers.schema_version, "v2");
    assert_eq!(
        controllers.runtime_state,
        RealmControllerRuntimeState::MetadataOnly
    );
    assert!(controllers.invariants.metadata_only);
    let any_materialized = controllers.controllers.iter().any(|controller| {
        controller.daemon.materialized_service
            || controller.broker.materialized_service
            || controller.broker.materialized_socket
    });
    assert_eq!(
        controllers.invariants.no_systemd_units_materialized,
        !any_materialized
    );
    assert!(controllers.invariants.preserves_global_daemon_behavior);
    assert!(
        controllers
            .invariants
            .preserves_direct_unix_socket_semantics
    );
    for controller in &controllers.controllers {
        assert!(
            controller.daemon.user.as_str().starts_with("d2bd-r-"),
            "realm daemon principal must be deterministic and realm-scoped"
        );
        assert!(
            controller.broker.user.as_str().starts_with("d2bbr-r-"),
            "realm broker principal must be distinct and realm-scoped"
        );
        assert!(
            controller
                .daemon
                .public_socket_group
                .as_str()
                .starts_with("d2b-r-"),
            "realm public group must be realm-scoped"
        );
        assert_eq!(
            controller.allocator.config_path.as_str(),
            "/etc/d2b/allocator.json"
        );
        assert_eq!(
            controller.allocator.root_socket.as_str(),
            "/run/d2b/allocator/local-root.sock"
        );
        let realm_id = controller.realm_id.as_str();
        assert_eq!(
            controller.daemon.config_path.as_str(),
            format!("/etc/d2b/r/{realm_id}/controller.json")
        );
        for file in ["controller", "providers", "storage", "sync"] {
            let expected_id = format!("path:realm-config-{file}:{realm_id}");
            let expected_path = format!("/etc/d2b/r/{realm_id}/{file}.json");
            let row = storage
                .paths
                .iter()
                .find(|path| path.id.as_str() == expected_id)
                .unwrap_or_else(|| panic!("storage.json missing {expected_id}"));
            assert_eq!(row.path_template.as_str(), expected_path);
            assert_eq!(row.kind, StoragePathKind::RegularFile);
            assert_eq!(row.mode, "0640");
            assert_eq!(row.owner.kind, PrincipalKind::User);
            assert_eq!(row.owner.value, controller.broker.user);
            assert_eq!(row.creator.kind, ActorKind::Broker);
            assert_eq!(row.creator.value, controller.broker.user);
            assert_eq!(row.repair_policy, RepairPolicy::BrokerReconcile);
            assert!(!row.recursive);
        }
    }

    assert_eq!(
        bundle.realm_controllers_path.as_deref(),
        Some("/etc/d2b/realm-controllers.json")
    );
    assert!(
        bundle
            .artifact_hashes
            .as_ref()
            .is_some_and(|hashes| hashes.contains_key("/etc/d2b/realm-controllers.json")),
        "bundle integrity table must cover realm-controllers.json"
    );
}

#[test]
fn storage_and_sync_schemas_are_committed_and_closed() {
    let storage_schema = read_repo_file("docs/reference/schemas/v2/storage.json");
    let sync_schema = read_repo_file("docs/reference/schemas/v2/sync.json");
    let allocator_schema = read_repo_file("docs/reference/schemas/v2/allocator.json");
    let realm_controller_schema =
        read_repo_file("docs/reference/schemas/v2/realm-controllers.json");
    for (name, schema) in [
        ("storage.json", storage_schema.as_str()),
        ("sync.json", sync_schema.as_str()),
        ("allocator.json", allocator_schema.as_str()),
        ("realm-controllers.json", realm_controller_schema.as_str()),
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
    assert!(allocator_schema.contains("\"metadata-only\""));
    assert!(allocator_schema.contains("\"namespace-boundary\""));
    assert!(realm_controller_schema.contains("\"local-root-metadata\""));
    assert!(realm_controller_schema.contains("\"preservesDirectUnixSocketSemantics\""));
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

    assert!(reference.contains("/var/lib/d2b/daemon-state/storage-lifecycle-report.json"));
    assert!(reference.contains("./schemas/v2/storage-lifecycle-report.json"));
    assert!(xtask.contains("\"storage-lifecycle-report.json\""));
}

#[test]
fn rendered_storage_contract_covers_process_writable_paths_when_fixture_available() {
    let fixture_dirs: Vec<_> = ["D2B_FIXTURES", "D2B_FIXTURES_FULL"]
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|dir| (name, PathBuf::from(dir))))
        .collect();
    if fixture_dirs.is_empty() {
        eprintln!("  (skipping rendered storage/sync contract check; D2B_FIXTURES unset)");
        return;
    }

    for (fixture_name, dir) in fixture_dirs {
        let storage: StorageJson = read_json(&dir, "storage.json");
        let sync: SyncJson = read_json(&dir, "sync.json");
        let processes: ProcessesJson = read_json(&dir, "processes.json");
        let bundle: Bundle = read_json(&dir, "bundle.json");

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
                for writable in &node.profile.mount_policy.writable_paths {
                    assert!(
                        storage_paths.contains(writable.path.as_str()),
                        "{fixture_name} storage.json missing writable path for {}:{} -> {}",
                        dag.vm,
                        node.id.0,
                        writable.path
                    );
                }
            }
        }

        assert!(
            storage.paths.iter().all(|path| {
                matches!(
                    path.repair_policy,
                    RepairPolicy::BrokerReconcile | RepairPolicy::BrokerFailClosed
                ) && path.creator.kind == ActorKind::Broker
                    && !path.recursive
            }),
            "{fixture_name} realm/workload storage rows must be non-recursive and broker-owned"
        );
        assert!(
            storage.paths.iter().all(|path| {
                !path.scope.as_str().starts_with("vm:")
                    && !path.scope.as_str().starts_with("env:")
                    && !path.path_template.as_str().starts_with("/run/d2b/vms/")
                    && !path.path_template.as_str().starts_with("/var/lib/d2b/vms/")
            }),
            "{fixture_name} storage.json must not retain legacy VM/env scopes or paths"
        );
        assert!(
            storage
                .paths
                .iter()
                .all(|path| { !path.path_template.as_str().starts_with("/run/udev/") }),
            "{fixture_name} storage.json must not claim broker-owned storage authority over foreign /run/udev state"
        );
        assert!(
            sync.locks
                .iter()
                .all(|lock| lock.kind == LockKind::Ofd && lock.cloexec_required),
            "{fixture_name} every realm/workload lock must be an O_CLOEXEC OFD lock"
        );

        let store_live_rows: Vec<_> = storage
            .paths
            .iter()
            .filter(|path| path.id.as_str().ends_with("/store-view-live"))
            .collect();
        assert!(
            !store_live_rows.is_empty(),
            "{fixture_name} storage.json must include workload store-view live rows"
        );
        for path in store_live_rows {
            assert_ne!(path.path_template.as_str(), "/nix/store");
            for invariant in [
                StorageInvariant::SameFilesystem,
                StorageInvariant::HardlinkFarmNoRecursion,
                StorageInvariant::NoRecursiveMutation,
            ] {
                assert!(
                    path.invariants.contains(&invariant),
                    "{fixture_name} {} missing hardlink invariant {invariant:?}",
                    path.id
                );
            }
        }

        let guest_session_credentials: Vec<_> = storage
            .paths
            .iter()
            .filter(|path| {
                path.id
                    .as_str()
                    .starts_with("path:workload-guest-session-credential:")
            })
            .collect();
        assert!(
            !guest_session_credentials.is_empty(),
            "{fixture_name} storage.json must preserve guest-session credential rows"
        );
        for path in guest_session_credentials {
            assert_eq!(path.kind, StoragePathKind::RegularFile);
            assert_eq!(path.mode, "0440");
            assert_eq!(path.owner.kind, PrincipalKind::User);
            assert_eq!(path.owner.value.as_str(), "root");
            assert_eq!(path.group.kind, PrincipalKind::Group);
            assert!(
                path.group.value.as_str().starts_with("d2b-gctlfs-"),
                "{fixture_name} guest-session credential group must be workload-scoped"
            );
            assert_eq!(path.creator.kind, ActorKind::Broker);
            assert_eq!(path.repair_policy, RepairPolicy::BrokerFailClosed);
            assert!(!path.recursive);
        }

        // realm-observability-rows.nix's path rows (config/state/secret-source/
        // runtime/store-sync projection) must be re-emitted through the same
        // canonical, broker-owned storage.json authority as every other
        // workload path, not left dangling for workload-process-rows.nix's
        // `resourceRefs.observability` ids to reference nothing.
        let observability_paths: Vec<_> = storage
            .paths
            .iter()
            .filter(|path| path.id.as_str().starts_with("path:observability-"))
            .collect();
        if !observability_paths.is_empty() {
            assert_eq!(
                bundle.observability_secrets_path.as_deref(),
                Some("/etc/d2b/observability-secrets.json"),
                "{fixture_name} bundle must expose observability secret metadata"
            );
            assert!(
                bundle.artifact_hashes.as_ref().is_some_and(|hashes| {
                    hashes.contains_key("/etc/d2b/observability-secrets.json")
                }),
                "{fixture_name} bundle must integrity-pin observability secret metadata"
            );
            let expected_prefixes = [
                "path:observability-config:",
                "path:observability-runtime:",
                "path:observability-secrets:",
                "path:observability-state:",
                "path:observability-store-sync-projection:",
            ];
            assert_eq!(
                observability_paths.len(),
                expected_prefixes.len(),
                "{fixture_name} observability storage rows must register exactly the canonical path set"
            );
            for prefix in expected_prefixes {
                assert!(
                    observability_paths
                        .iter()
                        .any(|path| path.id.as_str().starts_with(prefix)),
                    "{fixture_name} storage.json missing observability path {prefix}"
                );
            }
            for path in &observability_paths {
                assert_eq!(
                    path.creator.kind,
                    ActorKind::Broker,
                    "{fixture_name} observability storage rows must be broker-created"
                );
                assert_eq!(path.owner.kind, PrincipalKind::User);
                assert!(
                    path.owner.value.as_str().starts_with("d2bbr-r-"),
                    "{fixture_name} observability storage rows must be broker-owned"
                );
                assert!(!path.recursive);
                assert!(path.no_follow);
                assert!(
                    matches!(
                        path.repair_policy,
                        RepairPolicy::BrokerReconcile | RepairPolicy::BrokerFailClosed
                    ),
                    "{fixture_name} observability storage rows must use a broker repair policy"
                );
            }
        }
    }
}

/// Every rendered lock in `sync.json` must carry a non-null `resourceId`
/// that pairs, without invention, to a real generated `storage.json` row
/// which the shared `d2b-state` runtime bridge
/// (`LockSet::acquire_from_generated`) will actually open. This is a
/// schema/JSON-level contract test (this crate does not depend on
/// `d2b-state`): it proves the *generated fixtures* carry every field the
/// runtime bridge requires, not that the bridge itself behaves correctly
/// (that is covered by `d2b-state`'s own unit tests in `lock.rs`/`path.rs`).
#[test]
fn rendered_sync_locks_pair_exactly_with_real_storage_rows_when_fixture_available() {
    let fixture_dirs: Vec<_> = ["D2B_FIXTURES", "D2B_FIXTURES_FULL"]
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|dir| (name, PathBuf::from(dir))))
        .collect();
    if fixture_dirs.is_empty() {
        eprintln!("  (skipping rendered sync/storage pairing check; D2B_FIXTURES unset)");
        return;
    }

    for (fixture_name, dir) in fixture_dirs {
        let storage: StorageJson = read_json(&dir, "storage.json");
        let sync: SyncJson = read_json(&dir, "sync.json");

        let rows_by_id: BTreeMap<&str, _> = storage
            .paths
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect();

        assert!(
            !sync.locks.is_empty(),
            "{fixture_name} sync.json must render at least one lock"
        );

        // Every lock must carry a resourceId, and it must resolve to a real
        // regular-file storage row that the runtime bridge can actually
        // open/acquire against — never a dangling or absent id.
        for lock in &sync.locks {
            let resource_id = lock.resource_id.as_ref().unwrap_or_else(|| {
                panic!(
                    "{fixture_name} lock {} has no resourceId; the shared runtime bridge cannot \
                     acquire it without inventing a resource identity",
                    lock.id.as_str()
                )
            });
            let row = rows_by_id.get(resource_id.as_str()).unwrap_or_else(|| {
                panic!(
                    "{fixture_name} lock {} resourceId {} has no matching storage.json row",
                    lock.id.as_str(),
                    resource_id.as_str()
                )
            });
            assert_eq!(
                row.kind,
                StoragePathKind::RegularFile,
                "{fixture_name} lock {} resourceId {} must pair with a regular-file row",
                lock.id.as_str(),
                resource_id.as_str()
            );
            assert!(
                row.no_follow,
                "{fixture_name} lock-file row {} must be noFollow",
                row.id.as_str()
            );
            assert!(
                !row.recursive,
                "{fixture_name} lock-file row {} must not be recursive",
                row.id.as_str()
            );
            if let Some(path_template) = &lock.path_template {
                assert_eq!(
                    path_template.as_str(),
                    row.path_template.as_str(),
                    "{fixture_name} lock {} pathTemplate must match its paired storage row",
                    lock.id.as_str()
                );
            }
            assert_eq!(
                lock.scope.as_str(),
                row.scope.as_str(),
                "{fixture_name} lock {} scope must match its paired storage row's scope",
                lock.id.as_str()
            );
            // Every generated lock is CLOEXEC OFD, close-on-exec inheritance,
            // and fail-fast with no fd-passing mechanism today: assert the
            // exact uniform policy so a silent drift is caught, not papered
            // over by a permissive adapter.
            assert_eq!(lock.kind, LockKind::Ofd);
            assert!(lock.cloexec_required);
        }

        // Global total order: `SyncJson::global_order_rank` must assign a
        // strict bijection onto `0..locks.len()` — no duplicate or missing
        // rank, proving the total order is well-defined across every
        // rendered lock without any fabricated `acquire_after` edge.
        let mut ranks: Vec<usize> = sync
            .locks
            .iter()
            .map(|lock| {
                sync.global_order_rank(&lock.id).unwrap_or_else(|| {
                    panic!(
                        "{fixture_name} lock {} missing from its own global order",
                        lock.id.as_str()
                    )
                })
            })
            .collect();
        ranks.sort_unstable();
        let expected: Vec<usize> = (0..sync.locks.len()).collect();
        assert_eq!(
            ranks, expected,
            "{fixture_name} global_order_rank must be a strict bijection over every rendered lock"
        );

        // The per-workload `keys.lock` row (task requirement: every OFD lock
        // file that currently lacks a storage row, especially workload keys)
        // must exist, be broker-owned/secret-adjacent/mode-0600, and pair
        // with exactly one lock whose owner/allowedHolders match the row's
        // broker owner.
        let keys_lock_rows: Vec<_> = storage
            .paths
            .iter()
            .filter(|row| row.id.as_str().starts_with("path:workload-keys-lock:"))
            .collect();
        assert!(
            !keys_lock_rows.is_empty(),
            "{fixture_name} storage.json must include a workload keys.lock row"
        );
        for row in &keys_lock_rows {
            assert_eq!(row.kind, StoragePathKind::RegularFile);
            assert_eq!(row.mode, "0600");
            assert_eq!(row.sensitivity, SensitivityClass::SecretAdjacent);
            assert_eq!(row.creator.kind, ActorKind::Broker);
            assert_eq!(row.owner.kind, PrincipalKind::User);
            assert!(row.owner.value.as_str().starts_with("d2bbr-r-"));
            assert!(
                matches!(
                    row.repair_policy,
                    RepairPolicy::BrokerReconcile | RepairPolicy::BrokerFailClosed
                ),
                "{fixture_name} keys.lock row {} must use a broker repair policy",
                row.id.as_str()
            );
            assert!(row.invariants.contains(&StorageInvariant::NoSymlink));
            assert!(row.invariants.contains(&StorageInvariant::NoMagicLink));

            let paired_lock = sync
                .locks
                .iter()
                .find(|lock| {
                    lock.resource_id.as_ref().map(|id| id.as_str()) == Some(row.id.as_str())
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{fixture_name} keys.lock row {} has no paired lock in sync.json",
                        row.id.as_str()
                    )
                });
            assert_eq!(paired_lock.owner_process.kind, ActorKind::Broker);
            assert_eq!(
                paired_lock.owner_process.value.as_str(),
                row.owner.value.as_str()
            );
            assert_eq!(
                paired_lock.release_authority.value.as_str(),
                row.owner.value.as_str(),
                "{fixture_name} keys.lock owner/release authority must be symmetric"
            );
        }
    }
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

#[test]
fn broker_storage_and_sync_requests_stay_opaque_only() {
    let broker_wire = read_repo_file("packages/d2b-contracts/src/broker_wire.rs");
    let request_re =
        Regex::new(r"(?s)pub struct (\w+Request) \{(?P<body>.*?)\n\}").expect("request regex");
    let field_re =
        Regex::new(r"(?m)^\s*(?:pub(?:\([^)]+\))?\s+)?([A-Za-z0-9_]+)\s*:").expect("field regex");
    let forbidden_fields = BTreeSet::from([
        "acl",
        "cleanup",
        "cleanup_policy",
        "cmd",
        "command",
        "fd_passing_policy",
        "group",
        "mode",
        "owner",
        "path",
        "path_template",
        "repair_policy",
    ]);

    let allowed = BTreeSet::from([
        ("OpenCgroupDirRequest", "path_class"),
        ("PrepareDirRequest", "path_class"),
        ("RunActivationRequest", "mode"),
    ]);
    let mut violations = Vec::new();
    for cap in request_re.captures_iter(&broker_wire) {
        let request = cap.get(1).expect("request name").as_str();
        let body = cap.name("body").expect("request body").as_str();
        for cap in field_re.captures_iter(body) {
            let field = &cap[1];
            let normalized = field.to_ascii_lowercase();
            if forbidden_fields.contains(normalized.as_str())
                && !allowed.contains(&(request, field))
            {
                violations.push(format!("{request}.{field}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "broker IPC storage/sync policy violation: mutating broker requests must carry opaque \
         ids/classes only, not caller-supplied raw storage paths, owners, ACLs, cleanup commands, \
         or lock policy. Add/extend storage.json or sync.json rows and resolve them in the broker \
         instead. Violations: {violations:?}"
    );
}

#[test]
fn tmpfiles_do_not_create_realm_or_workload_storage_leaves() {
    let tmpfiles_paths = literal_d2b_tmpfiles_paths();
    assert!(
        !tmpfiles_paths.is_empty(),
        "policy-paths: static tmpfiles inventory found no literal d2b tmpfiles paths"
    );

    let forbidden: Vec<_> = tmpfiles_paths
        .iter()
        .filter(|path| {
            path.starts_with("/etc/d2b/r/")
                || path.starts_with("/run/d2b/r/")
                || path.starts_with("/var/lib/d2b/r/")
                || path.starts_with("/var/cache/d2b/r/")
                || path.contains("/w/")
        })
        .cloned()
        .collect();

    assert!(
        forbidden.is_empty(),
        "policy-paths: tmpfiles must stop at fixed local-root anchors and endpoint state; \
         realm/workload leaves are broker-created from opaque storage ids: {forbidden:?}"
    );
}

#[test]
fn host_mutation_sources_are_registered_with_storage_or_sync_policy() {
    let discovered = host_mutation_sources();
    let registered = registered_host_mutation_sources();
    let unregistered: Vec<_> = discovered
        .iter()
        .filter(|path| !registered.contains_key(path.as_str()))
        .cloned()
        .collect();

    assert!(
        unregistered.is_empty(),
        "policy-paths: host-mutable path/lock mutation contexts must be registered with a \
         storage.json/sync.json contract row and one repair owner. This scan matches mutation \
         contexts (tmpfiles, activation snippets, mkdir/chmod/chown/setfacl, fs::write, \
         File::create/OpenOptions, create_dir*) near d2b host paths/locks; docs/ and tests/ \
         are excluded so prose/fixtures do not satisfy or fail the gate. Add contract coverage \
         before adding new mutation sources. Unregistered sources: {unregistered:?}"
    );

    let stale: Vec<_> = registered
        .keys()
        .filter(|path| !discovered.contains(**path))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "policy-paths: registered host mutation sources must remain live scan matches, so the \
         storage/sync ownership allowlist cannot accumulate stale entries. Stale registrations: \
         {stale:?}"
    );

    assert!(
        read_repo_file("AGENTS.md").contains("single repair owner"),
        "AGENTS.md must document the durable single repair owner rule for host-mutable paths/locks"
    );
}

fn literal_d2b_tmpfiles_paths() -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let rule_re = Regex::new(
        r#"(?m)^\s*"?[a-zA-Z][!+=~^-]*\s+((?:/var/lib|/var/cache|/run|/etc)/d2b(?:/[^ "'\t\n]*)?)"#,
    )
    .expect("tmpfiles path regex");
    for path in collect_repo_files("nixos-modules", "nix") {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("policy-paths: cannot read {}: {err}", path.display()));
        if !text.contains("tmpfiles.rules") {
            continue;
        }
        let code = stripped_code_text(&path, &text);
        for cap in rule_re.captures_iter(&code) {
            paths.insert(cap[1].trim_end_matches('/').to_owned());
        }
    }
    paths
}

fn host_mutation_sources() -> BTreeSet<String> {
    let mutation_re = Regex::new(
        r"(fs::(?:write|copy|rename|remove_(?:file|dir|dir_all))|write_evidence|std::os::unix::fs::symlink|symlink|hard_link|File::create|OpenOptions::new|create_dir(?:_all)?|set_permissions|chmod|chown|setfacl|systemd\.tmpfiles\.rules|tmpfiles\.rules|activationScripts|install\s+-[dm]|mkdir\s+-p)",
    )
    .expect("mutation context regex");
    let surface_re = Regex::new(
        r"(/var/lib/d2b(?:/|\b)|/var/cache/d2b(?:/|\b)|/run/d2b(?:/|\b)|/etc/d2b(?:/|\b)|cfg\.site\.stateDir|cfg\.store\.stateDir|evidence_dir|\.lock|locks/)",
    )
    .expect("surface regex");

    let mut found = BTreeSet::new();
    for rel in [
        ("nixos-modules", "nix"),
        ("packages/d2b/src", "rs"),
        ("packages/d2b-priv-broker/src", "rs"),
        ("packages/d2bd/src", "rs"),
        ("packages/d2b-host/src", "rs"),
        ("packages/d2b-host-activation-helper/src", "rs"),
    ] {
        for path in collect_repo_files(rel.0, rel.1) {
            let rel_path = path
                .strip_prefix(repo_root())
                .expect("repo-relative path")
                .to_string_lossy()
                .into_owned();
            if rel_path.starts_with("nixos-modules/options-")
                || rel_path == "nixos-modules/processes-json.nix"
                || rel_path == "nixos-modules/storage-json.nix"
                || rel_path == "nixos-modules/sync-json.nix"
            {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_else(|err| {
                panic!(
                    "policy-paths: cannot read mutation source {}: {err}",
                    path.display()
                )
            });
            let lines: Vec<String> = text
                .lines()
                .map(|line| stripped_code_line(&path, line).to_owned())
                .collect();
            for (idx, line) in lines.iter().enumerate() {
                if !mutation_re.is_match(line) {
                    continue;
                }
                let start = idx.saturating_sub(4);
                let end = (idx + 5).min(lines.len());
                let window = lines[start..end].join("\n");
                if surface_re.is_match(&window) {
                    found.insert(rel_path.clone());
                    break;
                }
            }
        }
    }
    found
}

fn stripped_code_text(path: &Path, text: &str) -> String {
    text.lines()
        .map(|line| stripped_code_line(path, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stripped_code_line<'a>(path: &Path, line: &'a str) -> &'a str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("nix") => line.split('#').next().unwrap_or(line),
        Some("rs") => line.split("//").next().unwrap_or(line),
        _ => line,
    }
}

fn registered_host_mutation_sources() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "nixos-modules/bundle.nix",
            "storage paths:private bundle artifacts",
        ),
        (
            "nixos-modules/components/observability/guest.nix",
            "storage root:path:run-root",
        ),
        (
            "nixos-modules/components/observability/host.nix",
            "storage root:path:run-root",
        ),
        (
            "nixos-modules/guest-control.nix",
            "storage paths:nodeWritablePaths/readinessSocketPaths",
        ),
        (
            "nixos-modules/host-activation.nix",
            "storage roots:path:state-root,path:run-root",
        ),
        (
            "nixos-modules/host-broker.nix",
            "storage root:path:state-root",
        ),
        (
            "nixos-modules/host-daemon.nix",
            "fixed local-root runtime/state/config anchors",
        ),
        (
            "nixos-modules/user-services.nix",
            "storage root:path:run-root",
        ),
        ("nixos-modules/store.nix", "storage root:path:state-root"),
        (
            "packages/d2b/src/host_validate.rs",
            "storage paths:validation evidence root/records",
        ),
        (
            "packages/d2b/src/lib.rs",
            "storage paths:validation evidence root/records via host validate dispatch",
        ),
        (
            "packages/d2b-host-activation-helper/src/main.rs",
            "storage root:path:state-root",
        ),
        (
            "packages/d2b-host/src/cgroup.rs",
            "sync lock:cgroup-delegation",
        ),
        (
            "packages/d2b-host/src/ownership_matrix.rs",
            "storage root:path:state-root",
        ),
        (
            "packages/d2b-priv-broker/src/audit.rs",
            "storage root:path:state-root audit log subtree",
        ),
        (
            "packages/d2b-priv-broker/src/live_handlers.rs",
            "broker resolves storage/sync opaque ids",
        ),
        (
            "packages/d2b-priv-broker/src/ops/exec_reconcile.rs",
            "storage roots:path:etc-root,path:state-root",
        ),
        (
            "packages/d2b-priv-broker/src/ops/media.rs",
            "storage paths:qemu-media registry/runtime index",
        ),
        (
            "packages/d2b-priv-broker/src/ops/store_sync.rs",
            "storage paths:store-view hardlink farm",
        ),
        (
            "packages/d2b-priv-broker/src/runtime.rs",
            "storage root:path:run-root",
        ),
        (
            "packages/d2bd/src/audio_dispatch.rs",
            "storage paths:audio-state + sync lock:audio-<vm>",
        ),
        ("packages/d2bd/src/lib.rs", "storage root:path:run-root"),
        (
            "packages/d2bd/src/typed_error.rs",
            "storage degraded-state reports",
        ),
    ])
}

fn collect_repo_files(rel_dir: &str, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = repo_root().join(rel_dir);
    collect_repo_files_inner(&root, extension, &mut out);
    out.sort();
    out
}

fn collect_repo_files_inner(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-paths: cannot read {}: {err}", dir.display()))
    {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if matches!(name, "target" | "tests" | "fixtures" | "docs") {
                continue;
            }
            collect_repo_files_inner(&path, extension, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}
