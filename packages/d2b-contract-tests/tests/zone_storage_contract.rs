use d2b_contract_tests::read_repo_file;
use d2b_contracts::v3::storage::{
    ZoneStoreDescriptorPublicationRequirement, ZoneStoreFilesystemRequirement,
    ZoneStoreFsyncRequirement, ZoneStoreLockingRequirement, ZoneStoreReplacementDetection,
    ZoneStoreReplacementPublicationRequirement, ZoneStoreStorageRow,
};
use serde_json::{Value, json};

fn canonical_row() -> Value {
    json!({
        "zoneStoreId": "zone-store-local-root",
        "storageOwnerPrincipal": "d2b-zonert",
        "parentDirectoryId": "zone-store-parent-local-root",
        "ownership": {
            "owner": "d2b-zonert",
            "group": "d2b-zonert",
            "mode": "0640",
            "linkCount": 1
        },
        "filesystem": "regular-file-anchored-fd-relative-no-follow",
        "locking": "ofd-close-on-exec",
        "marker": {
            "identityMarkerId": "zone-store-marker-local-root"
        },
        "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
        "fsync": "database-and-parent-directory",
        "publication": {
            "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
            "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
        }
    })
}

#[test]
fn source_row_rejects_paths_missing_invariants_and_unknown_fields() {
    let canonical: ZoneStoreStorageRow =
        serde_json::from_value(canonical_row()).expect("canonical row");
    assert_eq!(serde_json::to_value(canonical).unwrap(), canonical_row());

    for field in ["zoneStoreId", "parentDirectoryId", "identityMarkerId"] {
        let mut candidate = canonical_row();
        if field == "identityMarkerId" {
            candidate["marker"][field] = json!("/var/lib/d2b/zones/local-root/store.redb");
        } else {
            candidate[field] = json!("/var/lib/d2b/zones/local-root/store.redb");
        }
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "host path in {field} must be rejected"
        );
    }

    for field in [
        "zoneStoreId",
        "storageOwnerPrincipal",
        "parentDirectoryId",
        "ownership",
        "filesystem",
        "locking",
        "marker",
        "replacementDetection",
        "fsync",
        "publication",
    ] {
        let mut candidate = canonical_row();
        candidate.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "missing required invariant {field} must be rejected"
        );
    }

    for field in ["owner", "group", "mode", "linkCount"] {
        let mut candidate = canonical_row();
        candidate["ownership"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "missing ownership invariant {field} must be rejected"
        );
    }

    let mut missing_marker_id = canonical_row();
    missing_marker_id["marker"]
        .as_object_mut()
        .unwrap()
        .remove("identityMarkerId");
    assert!(serde_json::from_value::<ZoneStoreStorageRow>(missing_marker_id).is_err());

    for field in ["descriptor", "replacement"] {
        let mut candidate = canonical_row();
        candidate["publication"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "missing publication invariant {field} must be rejected"
        );
    }

    let mut unknown = canonical_row();
    unknown["hostPath"] = json!("/var/lib/d2b/zones/local-root/store.redb");
    assert!(serde_json::from_value::<ZoneStoreStorageRow>(unknown).is_err());
}

#[test]
fn generated_schema_is_closed_required_and_path_free() {
    let schema: Value = serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v3/zone-storage.json",
    ))
    .expect("zone-storage schema parses");

    let root = schema.as_object().expect("root schema object");
    assert_eq!(root.get("additionalProperties"), Some(&json!(false)));
    let required = root["required"].as_array().expect("root required array");
    for field in [
        "zoneStoreId",
        "storageOwnerPrincipal",
        "parentDirectoryId",
        "ownership",
        "filesystem",
        "locking",
        "marker",
        "replacementDetection",
        "fsync",
        "publication",
    ] {
        assert!(required.contains(&json!(field)), "schema requires {field}");
    }

    let schema_text = schema.to_string();
    for forbidden in ["hostPath", "pathTemplate", "absolutePath"] {
        assert!(
            !schema_text.contains(forbidden),
            "schema must not expose path field {forbidden}"
        );
    }
    for definition in [
        "ZoneStoreOwnershipInvariant",
        "ZoneStoreMarkerInvariant",
        "ZoneStorePublicationInvariant",
    ] {
        assert_eq!(
            schema["definitions"][definition]["additionalProperties"],
            json!(false),
            "{definition} must be closed"
        );
    }
}

#[test]
fn value_sets_are_narrowed_to_attested_singletons() {
    for (value, expected) in [
        (
            serde_json::to_value(
                ZoneStoreFilesystemRequirement::RegularFileAnchoredFdRelativeNoFollow,
            )
            .unwrap(),
            json!("regular-file-anchored-fd-relative-no-follow"),
        ),
        (
            serde_json::to_value(ZoneStoreLockingRequirement::OfdCloseOnExec).unwrap(),
            json!("ofd-close-on-exec"),
        ),
        (
            serde_json::to_value(
                ZoneStoreReplacementDetection::FailClosedOnMissingReplacedOrIdentityMismatch,
            )
            .unwrap(),
            json!("fail-closed-on-missing-replaced-or-identity-mismatch"),
        ),
        (
            serde_json::to_value(ZoneStoreFsyncRequirement::DatabaseAndParentDirectory).unwrap(),
            json!("database-and-parent-directory"),
        ),
        (
            serde_json::to_value(
                ZoneStoreDescriptorPublicationRequirement::OwnedDescriptorCloseOnExecVerifiedBeforeConcurrency,
            )
            .unwrap(),
            json!("owned-descriptor-close-on-exec-verified-before-concurrency"),
        ),
        (
            serde_json::to_value(
                ZoneStoreReplacementPublicationRequirement::AtomicRenameRetainPriorQuarantineAmbiguity,
            )
            .unwrap(),
            json!("atomic-rename-retain-prior-quarantine-ambiguity"),
        ),
    ] {
        assert_eq!(value, expected);
    }

    for (field, unknown) in [
        ("filesystem", "unknown-filesystem"),
        ("locking", "unknown-locking"),
        ("replacementDetection", "unknown-replacement"),
        ("fsync", "unknown-fsync"),
    ] {
        let mut candidate = canonical_row();
        candidate[field] = json!(unknown);
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "unknown {field} value must be rejected"
        );
    }

    for (field, unknown) in [
        ("descriptor", "unknown-descriptor-publication"),
        ("replacement", "unknown-replacement-publication"),
    ] {
        let mut candidate = canonical_row();
        candidate["publication"][field] = json!(unknown);
        assert!(
            serde_json::from_value::<ZoneStoreStorageRow>(candidate).is_err(),
            "unknown publication {field} value must be rejected"
        );
    }
}

#[test]
fn rendered_nix_contract_matches_the_rust_row_and_bundle_wiring() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(default_nix.contains("./zone-storage-json.nix"));

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    assert!(bundle_artifacts.contains("extraArtifacts = lib.mkOption"));

    let emitter = read_repo_file("nixos-modules/zone-storage-json.nix");
    assert!(emitter.contains("d2b._bundle.extraArtifacts = zoneStorageArtifacts;"));
    for (field, rendered_value) in [
        ("zoneStoreId", "zone-store-${zoneName}"),
        ("storageOwnerPrincipal", "d2b-zonert"),
        ("parentDirectoryId", "zone-store-parent-${zoneName}"),
        ("owner", "d2b-zonert"),
        ("group", "d2b-zonert"),
        ("mode", "0640"),
        ("filesystem", "regular-file-anchored-fd-relative-no-follow"),
        ("locking", "ofd-close-on-exec"),
        ("identityMarkerId", "zone-store-marker-${zoneName}"),
        (
            "replacementDetection",
            "fail-closed-on-missing-replaced-or-identity-mismatch",
        ),
        ("fsync", "database-and-parent-directory"),
        (
            "descriptor",
            "owned-descriptor-close-on-exec-verified-before-concurrency",
        ),
        (
            "replacement",
            "atomic-rename-retain-prior-quarantine-ambiguity",
        ),
    ] {
        assert!(
            emitter.contains(&format!("{field} = \"{rendered_value}\";")),
            "Nix emitter must render {field}"
        );
    }
    assert!(emitter.contains("linkCount = 1;"));
    for forbidden in ["hostPath", "pathTemplate", "/var/lib/d2b"] {
        assert!(
            !emitter.contains(forbidden),
            "Nix emitter must not carry {forbidden}"
        );
    }
}
