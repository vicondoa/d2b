//! Daemon startup storage/restart contract checks.
//!
//! ADR 0034 makes storage, process restart, and lock ownership explicit
//! generated contracts. This module is the daemon-side startup gate that
//! checks those contracts before any broad cleanup or adoption logic grows
//! around them. It is intentionally read-only: it never mutates storage,
//! never opens lock files, and never treats persisted PID values as
//! authority.
//!
//! The check operates on the v3 zone-native storage view: every authoritative
//! Zone must carry its integrity-pinned `ZoneStoreStorageRow`. The v2
//! host-wide `storage.json`/`sync.json` contracts and bundle-version gating
//! are gone, so the gate validates per-Zone rows and nothing else.

use d2b_core::bundle_resolver::BundleResolver;
pub use d2b_core::storage_lifecycle::{
    StorageContractValidationReason, StorageLifecycleIssue, StorageLifecycleReport,
};

/// Persisted storage lifecycle report schema.
pub const STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION: &str = "v2";

/// Report used when the trusted bundle cannot be loaded.
pub fn bundle_resolver_unavailable_report() -> StorageLifecycleReport {
    StorageLifecycleReport {
        schema_version: STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION.to_owned(),
        storage_contract_present: false,
        sync_contract_present: false,
        path_count: 0,
        restart_policy_count: 0,
        lock_count: 0,
        issues: vec![StorageLifecycleIssue::BundleResolverUnavailable],
    }
}

/// Run a read-only startup check over the bundle's v3 zone-native storage
/// contract. Output is bounded to ids and closed reason kinds; it never
/// includes raw storage paths.
///
/// Every authoritative Zone (one that has a verified resource bundle) must
/// carry its own `ZoneStoreStorageRow`; a Zone without its row, or an
/// index with no Zones at all, is reported as a missing storage contract.
/// The v2 sync/lock and bundle-version gates are removed: there is no
/// daemon-wide `sync.json` contract any more, and the diagnostic does not
/// depend on a bundle format version.
pub fn run_startup_contract_check(resolver: &BundleResolver) -> StorageLifecycleReport {
    let mut issues = Vec::new();

    let zones = match resolver.zone_resource_bundle_zones() {
        Ok(zones) => zones,
        Err(_) => {
            issues.push(StorageLifecycleIssue::MissingStorageContract);
            return StorageLifecycleReport {
                schema_version: STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION.to_owned(),
                storage_contract_present: false,
                sync_contract_present: false,
                path_count: 0,
                restart_policy_count: 0,
                lock_count: 0,
                issues,
            };
        }
    };

    let mut path_count = 0usize;
    for zone in &zones {
        if resolver.zone_storage_row(zone.as_str()).is_some() {
            path_count += 1;
        }
    }
    if zones.is_empty() || path_count < zones.len() {
        issues.push(StorageLifecycleIssue::MissingStorageContract);
    }

    StorageLifecycleReport {
        schema_version: STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION.to_owned(),
        storage_contract_present: !zones.is_empty() && issues.is_empty(),
        sync_contract_present: false,
        path_count,
        restart_policy_count: 0,
        lock_count: 0,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    use d2b_core::bundle_resolver::{BundleResolver, BundleVerifyPolicy};

    use super::*;

    /// An on-disk v3 zone-native bundle whose resolver is built through the
    /// production loader, so `zone_storage_rows` is populated from the
    /// declared artifacts.
    struct ZoneNativeFixture {
        _root: PathBuf,
        resolver: BundleResolver,
    }

    impl Drop for ZoneNativeFixture {
        // Remove-On-Drop teardown must run synchronously at the end of the
        // plain #[test] body; the crate has no async runtime.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self._root);
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest as _;
        let raw: [u8; 32] = sha2::Sha256::digest(bytes).into();
        let hex: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("sha256:{hex}")
    }

    // Fixture writer drives blocking fs synchronously from plain #[test] bodies;
    // the crate has no async runtime.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write_artifact(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(path, bytes).expect("write fixture artifact");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
            .expect("chmod fixture artifact to 0640");
    }

    /// A minimal verified Zone resource bundle (`zones/<zone>/resource-bundle.json`)
    /// with no resources and the content hash the resolver verifies.
    ///
    /// The content hash is the canonical digest of the empty resource array:
    /// `framed_canonical_digest("d2b:v3:resource-bundle", canonical("[]"))`.
    fn resource_bundle_bytes(zone: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 3,
            "bundleVersion": 1,
            "zone": zone,
            "contentHash": "sha256:854fc6c314b185ac9f842231e368fc75650729f669e15d0f1e60141ea334cb5e",
            "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
            "schemaFingerprints": {},
            "providerSchemaDigests": {},
            "resources": [],
            "generatedAt": "1970-01-01T00:00:00.000Z"
        }))
        .expect("zone resource bundle serializes")
    }

    /// A closed per-Zone storage row (`<zone>/storage.json`).
    fn storage_row_bytes(zone: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "identity": {
                "zoneUid": "123e4567-e89b-42d3-a456-426614174000",
                "storeUid": "223e4567-e89b-42d3-a456-426614174001",
                "storeEpoch": 1
            },
            "zoneStoreId": format!("zone-store-{zone}"),
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": format!("zone-store-parent-{zone}"),
            "ownership": {
                "owner": "d2b-zonert", "group": "d2b-zonert",
                "mode": "0640", "linkCount": 1
            },
            "auxiliaryDirectories": {
                "audit": {
                    "directoryId": format!("zone-store-audit-{zone}"),
                    "owner": "d2bd", "group": "d2bd", "mode": "0700",
                    "repairOwner": "privileged-broker"
                },
                "telemetry": {
                    "directoryId": format!("zone-store-telemetry-{zone}"),
                    "owner": "d2bd", "group": "d2bd", "mode": "0700",
                    "repairOwner": "privileged-broker"
                }
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": format!("zone-store-marker-{zone}")
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        }))
        .expect("zone storage row serializes")
    }

    /// The sealed zone topology index the loader verifies against the Zone set.
    ///
    /// The parent-map digest pins the single-root topology `{"dev": null}`
    /// (`framed_canonical_digest("d2b:v3:parent-topology", canonical(parent_map))`).
    fn topology_index_bytes(zone: &str) -> Vec<u8> {
        let parent_map = BTreeMap::from([(zone.to_owned(), None::<String>)]);
        let parent_map_value =
            serde_json::to_value(&parent_map).expect("parent map serializes to value");
        let mut zones = serde_json::Map::new();
        zones.insert(zone.to_owned(), serde_json::Value::Object(Default::default()));
        let mut generation_by_zone = serde_json::Map::new();
        generation_by_zone.insert(
            zone.to_owned(),
            serde_json::Value::String(format!("sha256:{}", "a".repeat(64))),
        );
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": "v1",
            "zones": zones,
            "topology": {
                "sealed": true,
                "parentMap": parent_map_value,
                "parentMapDigest": "sha256:a6236cbb470f08d01b75b0282cf349f0521a49a620661c7279bca7b828c46ca1",
                "generationByZone": generation_by_zone
            },
            "executionIndex": {},
            "networkIndex": {},
            "closureIndex": {}
        }))
        .expect("topology index serializes")
    }

    /// The v3 `bundle.json` index; `bundleHash` is the canonical hash with
    /// `artifactHashes` nullified, matching the production verifier.
    fn bundle_index_bytes(
        zone: &str,
        hashes: &BTreeMap<String, String>,
    ) -> Vec<u8> {
        let mut value = serde_json::json!({
            "artifactHashes": hashes,
            "bundleVersion": 1,
            "schemaVersion": "v3",
            "privilegesPath": "privileges.json",
            "zones": [{
                "zone": zone,
                "path": format!("zones/{zone}/resource-bundle.json")
            }],
            "generation": {
                "generator": "test",
                "sourceRevision": null,
                "generatedAt": null
            }
        });
        let preimage = {
            let mut with_nulled_hashes = value.clone();
            with_nulled_hashes["artifactHashes"] = serde_json::Value::Null;
            serde_json::to_vec(&with_nulled_hashes).expect("bundle hash preimage serializes")
        };
        value["bundleHash"] = serde_json::Value::String(sha256_hex(&preimage));
        serde_json::to_vec(&value).expect("bundle index serializes")
    }

    /// Build a verifiable one-Zone ("dev") v3 bundle, optionally shipping the
    /// Zone's `storage.json` row, and load it through the production loader.
    // Fixture builder drives blocking fs synchronously from plain #[test] bodies;
    // the crate has no async runtime.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn zone_native_fixture(with_storage_row: bool) -> ZoneNativeFixture {
        let zone = "dev";
        let root = std::env::temp_dir().join(format!(
            "d2b-storage-lifecycle-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos(),
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");

        let resource_bytes = resource_bundle_bytes(zone);
        write_artifact(&root.join(format!("zones/{zone}/resource-bundle.json")), &resource_bytes);
        let index_bytes = topology_index_bytes(zone);
        write_artifact(&root.join("index.json"), &index_bytes);

        let mut hashes = BTreeMap::from([
            (
                format!("zones/{zone}/resource-bundle.json"),
                sha256_hex(&resource_bytes),
            ),
            ("index.json".to_owned(), sha256_hex(&index_bytes)),
        ]);
        if with_storage_row {
            let row_bytes = storage_row_bytes(zone);
            hashes.insert(
                format!("zones/{zone}/storage.json"),
                sha256_hex(&row_bytes),
            );
            write_artifact(
                &root.join(format!("zones/{zone}/storage.json")),
                &row_bytes,
            );
        }
        let bundle_bytes = bundle_index_bytes(zone, &hashes);
        write_artifact(&root.join("bundle.json"), &bundle_bytes);

        let resolver = BundleResolver::load_with_policy(
            &root.join("bundle.json"),
            &BundleVerifyPolicy::for_tests(),
        )
        .expect("zone-native fixture bundle loads");
        ZoneNativeFixture {
            _root: root,
            resolver,
        }
    }

    #[test]
    fn reports_clean_when_every_zone_carries_its_storage_row() {
        let fixture = zone_native_fixture(true);
        let report = run_startup_contract_check(&fixture.resolver);
        assert!(!report.is_degraded(), "{report:?}");
        assert!(report.storage_contract_present);
        assert!(!report.sync_contract_present);
        assert_eq!(report.path_count, 1);
        assert_eq!(report.restart_policy_count, 0);
        assert_eq!(report.lock_count, 0);
    }

    #[test]
    fn reports_missing_storage_row_for_a_zone_without_one() {
        let fixture = zone_native_fixture(false);
        let report = run_startup_contract_check(&fixture.resolver);
        assert!(report.is_degraded(), "{report:?}");
        assert!(
            report
                .issues
                .contains(&StorageLifecycleIssue::MissingStorageContract),
            "{report:?}"
        );
        assert!(!report.storage_contract_present);
        assert_eq!(report.path_count, 0);
    }

    #[test]
    fn startup_report_carries_diagnostics_without_pidfd_authority_or_raw_paths() {
        let fixture = zone_native_fixture(true);
        let report = run_startup_contract_check(&fixture.resolver);
        let serialized = serde_json::to_string(&report).expect("serialize report");
        assert!(!serialized.contains("pidfd"), "{serialized}");
        assert!(!serialized.contains("pidFd"), "{serialized}");
        assert!(!serialized.contains("/run/d2b"), "{serialized}");
        assert!(!serialized.contains("path:run-root"), "{serialized}");
    }

    #[test]
    fn bundle_resolver_unavailable_report_is_degraded_and_schema_current() {
        let report = bundle_resolver_unavailable_report();
        assert_eq!(
            report.schema_version,
            STORAGE_LIFECYCLE_REPORT_SCHEMA_VERSION
        );
        assert!(report.is_degraded());
        assert_eq!(
            report.issues,
            vec![StorageLifecycleIssue::BundleResolverUnavailable]
        );
    }
}
