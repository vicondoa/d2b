use std::{env, fs, path::PathBuf};

use d2b_contract_tests::read_repo_file;
use d2b_core::{bundle::Bundle, realm_controller_config::RealmControllersJson};
use serde_json::Value;

fn realm_core_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/d2b-realm-core.json",
    ))
    .expect("d2b-realm-core schema parses")
}

fn definition<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema
        .get("definitions")
        .and_then(Value::as_object)
        .and_then(|definitions| definitions.get(name))
        .unwrap_or_else(|| panic!("schema is missing definition {name}"))
}

fn read_fixture_json<T: serde::de::DeserializeOwned>(dir: &std::path::Path, name: &str) -> T {
    let path = dir.join(name);
    let json = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture at {}: {err}", path.display()));
    serde_json::from_str(&json).unwrap_or_else(|err| {
        panic!(
            "failed to parse fixture {name} as {}: {err}",
            std::any::type_name::<T>()
        )
    })
}

#[test]
fn identity_lifecycle_dtos_are_in_generated_schema_contract() {
    let schema = realm_core_schema();
    let root_refs = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .map(Value::to_string)
        .unwrap_or_else(|| schema.to_string());

    for dto in [
        "RealmIdentityRef",
        "ControllerGenerationCredentialRef",
        "KeyRotationId",
        "RevocationListId",
        "RecoveryProcedureId",
        "RealmIdentityFingerprint",
        "RealmIdentityMetadata",
        "ControllerGenerationMetadata",
        "ParentTrustAnchor",
        "ChildKeyPin",
        "EnrollmentRecord",
        "KeyRotationPlan",
        "KeyRotationEvent",
        "RevocationRecord",
        "RevocationList",
        "SessionTeardownDirective",
        "RecoveryProcedure",
        "IdentityAuditEventMetadata",
        "RealmIdentityConfigJson",
        "RealmIdentityConfigEntry",
        "RealmIdentityConfigInvariants",
        "RealmIdentityConfigRuntimeState",
    ] {
        definition(&schema, dto);
        assert!(
            root_refs.contains(&format!("#/definitions/{dto}")),
            "d2b-realm-core root schema must expose {dto}"
        );
    }
}

#[test]
fn realm_identity_artifact_is_wired_as_private_non_secret_bundle_source() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./realm-identity-config-json.nix"),
        "default.nix must import realm-identity-config-json.nix"
    );

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    assert!(
        bundle_artifacts.contains("realmIdentityJson"),
        "bundle-artifacts.nix must declare realmIdentityJson metadata"
    );

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "realmIdentityPath = \"/etc/d2b/realm-identity.json\";",
        "key = \"/etc/d2b/realm-identity.json\";",
    ] {
        assert!(
            bundle_nix.contains(needle),
            "bundle.nix missing realm-identity wiring: {needle}"
        );
    }

    let identity_nix = read_repo_file("nixos-modules/realm-identity-config-json.nix");
    for needle in [
        "installFileName = \"realm-identity.json\";",
        "classification = \"contractPrivateNonSecret\";",
        "sensitivity = \"nonSecret\";",
        "runtimeState = \"metadata-only\";",
        "noSecretMaterial = true;",
        "preservesRuntimeBehavior = true;",
    ] {
        assert!(
            identity_nix.contains(needle),
            "realm-identity emitter missing contract marker: {needle}"
        );
    }
}

#[test]
fn identity_lifecycle_schema_excludes_material_fields() {
    let schema = realm_core_schema().to_string();

    for forbidden in [
        "privateKey",
        "publicKeyPem",
        "credentialMaterial",
        "providerToken",
        "relayCredential",
        "signatureBytes",
        "sessionSecret",
    ] {
        assert!(
            !schema.contains(forbidden),
            "schema must not expose secret/key material field {forbidden}"
        );
    }
}

#[test]
fn rendered_realm_identity_and_controller_contracts_are_integrity_pinned_when_fixture_available() {
    let Some(dir) = env::var_os("D2B_FIXTURES").map(PathBuf::from) else {
        eprintln!("  (skipping rendered realm-identity contract check; D2B_FIXTURES unset)");
        return;
    };

    let identity: Value = read_fixture_json(&dir, "realm-identity.json");
    let controllers: RealmControllersJson = read_fixture_json(&dir, "realm-controllers.json");
    let bundle: Bundle = read_fixture_json(&dir, "bundle.json");

    assert_eq!(identity["schemaVersion"], "v2");
    assert_eq!(identity["runtimeState"], "metadata-only");
    assert_eq!(identity["invariants"]["metadataOnly"], true);
    assert_eq!(identity["invariants"]["noSecretMaterial"], true);
    assert_eq!(identity["invariants"]["preservesRuntimeBehavior"], true);

    let rendered = identity.to_string();
    for forbidden in [
        "privateKey",
        "publicKeyPem",
        "credentialMaterial",
        "providerToken",
        "relayCredential",
        "signatureBytes",
        "sessionSecret",
        "SharedAccessKey",
        "BEGIN PRIVATE KEY",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "rendered realm-identity fixture must not expose secret/key material {forbidden}"
        );
    }

    assert_eq!(
        bundle.realm_identity_path.as_deref(),
        Some("/etc/d2b/realm-identity.json")
    );
    assert_eq!(
        bundle.realm_controllers_path.as_deref(),
        Some("/etc/d2b/realm-controllers.json")
    );
    assert_eq!(bundle.bundle_version, 13);
    assert_eq!(bundle.schema_version, "v2");
    assert_eq!(controllers.schema_version, "v2");
    assert_eq!(
        controllers.runtime_state,
        d2b_core::realm_controller_config::RealmControllerRuntimeState::MetadataOnly
    );
    let hashes = bundle
        .artifact_hashes
        .as_ref()
        .expect("bundle fixture must carry artifact hashes");
    for path in [
        "/etc/d2b/realm-identity.json",
        "/etc/d2b/realm-controllers.json",
    ] {
        assert!(
            hashes.contains_key(path),
            "bundle v13 must integrity-pin private realm artifact {path}"
        );
    }
}
