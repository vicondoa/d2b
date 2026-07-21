//! Shared CLI-contract integration-test harness.
//!
//! Most CLI-contract cases drive the `d2b` binary against static fixtures
//! and need nothing here. A handful of cases (audit / host-check daemon-backed
//! paths) must talk to a real, KVM-free `d2bd` over `AF_UNIX` +
//! `SO_PEERCRED`. This module spawns such a daemon in `--once` mode with a
//! synthetic config authorizing the real connecting test uid.
//!
//! The d2bd binary path is delivered out-of-band via
//! `D2B_TEST_D2BD_BIN` (the gated rust-workspace-checks.sh step builds
//! `-p d2bd` and exports it). `d2b` does NOT depend on `d2bd`
//! (the static-rust-dependency-direction policy forbids that edge), so daemon
//! cases SKIP cleanly when the env var is unset (e.g. the plain
//! `cargo test --workspace` pass).

#![allow(dead_code)]

use std::path::Path;

fn sha256_digest(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn write_bundle_with_hash(bundle_path: &Path, mut bundle: serde_json::Value) {
    use std::os::unix::fs::PermissionsExt;

    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundleHash");
    let mut canonical_bundle = bundle.clone();
    canonical_bundle
        .as_object_mut()
        .expect("canonical bundle object")
        .insert("artifactHashes".to_owned(), serde_json::Value::Null);
    let canonical = serde_json::to_vec(&canonical_bundle).expect("encode canonical bundle");
    bundle.as_object_mut().expect("bundle object").insert(
        "bundleHash".to_owned(),
        serde_json::Value::String(sha256_digest(&canonical)),
    );
    std::fs::write(
        bundle_path,
        serde_json::to_vec_pretty(&bundle).expect("encode hermetic bundle"),
    )
    .expect("write hermetic bundle");
    std::fs::set_permissions(bundle_path, std::fs::Permissions::from_mode(0o640))
        .expect("chmod hermetic bundle");
}

pub fn refresh_bundle_integrity(destination: &Path, changed_artifacts: &[&str]) {
    let bundle_path = destination.join("bundle.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bundle_path).expect("read bundle"))
            .expect("decode bundle");
    let artifact_hashes = bundle
        .as_object_mut()
        .expect("bundle object")
        .get_mut("artifactHashes")
        .and_then(serde_json::Value::as_object_mut)
        .expect("bundle artifact hashes");
    for artifact in changed_artifacts {
        let bytes =
            std::fs::read(destination.join(artifact)).expect("read changed bundle artifact");
        artifact_hashes.insert(
            (*artifact).to_owned(),
            serde_json::Value::String(sha256_digest(&bytes)),
        );
    }
    write_bundle_with_hash(&bundle_path, bundle);
}

pub fn build_hermetic_bundle_tree(fixtures: &Path, destination: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(destination.join("closures")).expect("mk fixture closures");
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o750))
        .expect("chmod fixture directory");
    std::fs::set_permissions(
        destination.join("closures"),
        std::fs::Permissions::from_mode(0o750),
    )
    .expect("chmod fixture closures");
    for entry in std::fs::read_dir(fixtures).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        if entry.file_type().expect("fixture type").is_file() {
            let bytes = std::fs::read(entry.path()).expect("read fixture");
            let path = destination.join(entry.file_name());
            std::fs::write(&path, bytes).expect("write fixture");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
                .expect("chmod fixture");
        }
    }
    for entry in std::fs::read_dir(fixtures.join("closures")).expect("read fixture closures") {
        let entry = entry.expect("fixture closure");
        let bytes = std::fs::read(entry.path()).expect("read fixture closure");
        let path = destination.join("closures").join(entry.file_name());
        std::fs::write(&path, bytes).expect("write fixture closure");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
            .expect("chmod fixture closure");
    }

    let provider_registry_path = destination.join("provider-registry-v2.json");
    let mut provider_registry: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&provider_registry_path).expect("read provider registry"),
    )
    .expect("decode provider registry");
    let provider_registry_object = provider_registry
        .as_object_mut()
        .expect("provider registry object");
    provider_registry_object.insert(
        "configurationFingerprint".to_owned(),
        serde_json::Value::String("0".repeat(64)),
    );
    provider_registry_object.insert("providers".to_owned(), serde_json::Value::Array(Vec::new()));
    let provider_registry_bytes =
        serde_json::to_vec(&provider_registry).expect("encode empty provider registry");
    std::fs::write(&provider_registry_path, &provider_registry_bytes)
        .expect("write empty provider registry");
    std::fs::set_permissions(
        &provider_registry_path,
        std::fs::Permissions::from_mode(0o640),
    )
    .expect("chmod provider registry");
    let provider_registry_digest = sha256_digest(&provider_registry_bytes);

    let bundle_path = destination.join("bundle.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bundle_path).expect("read copied bundle"))
            .expect("decode copied bundle");
    let object = bundle.as_object_mut().expect("bundle object");
    for field in [
        "hostPath",
        "privilegesPath",
        "processesPath",
        "providerRegistryV2Path",
        "publicManifestPath",
        "realmControllersPath",
        "realmIdentityPath",
        "realmWorkloadsLauncherV2Path",
        "storagePath",
        "syncPath",
    ] {
        let name = if field == "publicManifestPath" {
            "manifest.json".to_owned()
        } else {
            object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .and_then(|path| Path::new(path).file_name())
                .expect("bundle artifact filename")
                .to_string_lossy()
                .into_owned()
        };
        object.insert(field.to_owned(), serde_json::Value::String(name));
    }
    // The rendered fixture's allocator.json carries per-workload tap resource
    // requests (`AllocatorResourceSourceKind::realm-workload-network`) and its
    // unsafe-local-workloads.json declares a realm-native (non `unsafe-local`)
    // configured workload — both outside this CLI-contract suite's scope: no
    // `d2b` command under test here reads either artifact, and the
    // fixture-contract layer already owns validating their Nix<->Rust schema
    // shape (packages/d2b-contract-tests). Drop the references so the
    // hermetic sanity check below only re-validates the artifacts this
    // suite's fixtures actually depend on.
    for unused_field in ["allocatorPath", "unsafeLocalWorkloadsPath"] {
        object.insert(unused_field.to_owned(), serde_json::Value::Null);
    }
    let artifact_hashes = object
        .get("artifactHashes")
        .and_then(serde_json::Value::as_object)
        .expect("bundle artifact hashes")
        .iter()
        .map(|(path, digest)| {
            let key = if path.ends_with("/vms.json") {
                "manifest.json".to_owned()
            } else if Path::new(path).is_absolute() {
                Path::new(path)
                    .file_name()
                    .expect("artifact filename")
                    .to_string_lossy()
                    .into_owned()
            } else {
                path.clone()
            };
            let digest = if key == "provider-registry-v2.json" {
                serde_json::Value::String(provider_registry_digest.clone())
            } else {
                digest.clone()
            };
            (key, digest)
        })
        .collect();
    object.insert(
        "artifactHashes".to_owned(),
        serde_json::Value::Object(artifact_hashes),
    );
    write_bundle_with_hash(&bundle_path, bundle);
    d2b_core::bundle_resolver::BundleResolver::load_with_policy(
        &destination.join("bundle.json"),
        &d2b_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
    )
    .expect("validate hermetic bundle");
}
