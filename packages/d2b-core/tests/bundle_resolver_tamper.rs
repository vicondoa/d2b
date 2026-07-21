//! BundleResolver tamper-resistance integration tests.
//!
//! Each test creates a self-contained fake bundle root inside a
//! `tempfile::TempDir` — the real `/etc/d2b` is never touched.
//!
//! The tests use [`BundleVerifyPolicy`] with the **current process's**
//! uid/gid so that files created without `chown` still pass the owner
//! check.  The "owner = nobody" test (`tamper_owner_wrong_uid`) requires
//! `chown` and is skipped automatically when the process is not root.

use d2b_core::bundle_resolver::{BundleResolver, BundleVerifyPolicy};
use d2b_core::error::{BundleError, Error, ManifestError};
use sha2::Digest as _;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

/// Return a [`BundleVerifyPolicy`] whose uid/gid match the running
/// process so test files pass the owner check without `chown`.
fn current_user_policy() -> BundleVerifyPolicy {
    BundleVerifyPolicy {
        required_uid: rustix::process::getuid().as_raw(),
        required_gid: Some(rustix::process::getgid().as_raw()),
        required_mode: 0o640,
    }
}

/// Write `content` to `path` with mode 0o640.
fn write_private(path: &Path, content: &[u8]) {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o640)
        .open(path)
        .expect("create file")
        .write_all(content)
        .expect("write file");
}

/// Compute `"sha256:<hex>"` over `data` — same algorithm as the Rust verifier.
fn sha256_hex(data: &[u8]) -> String {
    let digest: [u8; 32] = sha2::Sha256::digest(data).into();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Build a minimal but fully-parseable bundle JSON (without `bundleHash`).
/// Returns the canonical JSON bytes *without* the hash field.
fn minimal_bundle_json_no_hash() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "bundleVersion": 4,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle json serializes")
}

/// Like `minimal_bundle_json_no_hash` but includes `"artifactHashes": null`
/// so the `bundleHash` computed from these bytes commits to the presence of
/// the `artifactHashes` field (matching the Nix emitter's `dataWithoutHash`).
fn minimal_bundle_json_with_null_artifact_hashes() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "artifactHashes": null,
        "bundleVersion": 4,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle json with null artifact hashes serializes")
}

/// Build bundle JSON with a correct `bundleHash` embedded.
fn bundle_json_with_hash(pre_hash_bytes: &[u8]) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(pre_hash_bytes).expect("pre-hash bundle parses");
    let hash = sha256_hex(pre_hash_bytes);
    value
        .as_object_mut()
        .expect("bundle is object")
        .insert("bundleHash".to_owned(), serde_json::Value::String(hash));
    serde_json::to_vec(&value).expect("bundle with hash serializes")
}

/// Minimal valid host.json bytes.
fn minimal_host_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "site": { "allowUnsafeEastWest": false },
        "environments": [],
        "nftables": {
            "family": "inet",
            "table": "d2b",
            "chains": [],
            "tableHashAfterApply": null,
            "ownershipId": "test"
        },
        "networkManager": {
            "filePath": "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf",
            "matchCriteria": [],
            "reloadBehavior": "atomic-reload",
            "ownership": {
                "owner": "root",
                "group": "root",
                "mode": "0644",
                "driftPolicy": "replace"
            }
        },
        "hostsFile": {
            "startMarker": "# d2b-managed begin",
            "endMarker": "# d2b-managed end",
            "rule": "replace-managed-block"
        },
        "kernelModules": [],
        "fdOwnership": [],
        "cloudHypervisorCapabilities": [],
        "ifNameMappings": [],
        "ch": null,
        "firewallCoexistencePolicy": null
    }))
    .expect("host json serializes")
}

/// Minimal valid processes.json bytes.
fn minimal_processes_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "vms": []
    }))
    .expect("processes json serializes")
}

/// Minimal valid vms.json (ManifestV04) bytes.
fn minimal_vms_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "_manifest": {
            "manifestVersion": 6
        },
        "_observability": {
            "enabled": false,
            "signozUrl": "http://127.0.0.1:8080",
            "signozOtlpGrpcPort": 4317,
            "signozOtlpHttpPort": 4318,
            "obsVsockCid": 0,
            "obsVsockHostSocket": "",
            "vmName": ""
        }
    }))
    .expect("vms json serializes")
}

fn unsafe_local_workloads_json_with_identity(runtime_kind: &str, provider_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "workloads": [{
            "identity": {
                "workloadId": "tools",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "tools.host.d2b",
                "runtimeKind": runtime_kind,
                "providerId": provider_id
            },
            "defaultItemId": "browser",
            "items": [{
                "type": "exec",
                "id": "browser",
                "name": "Browser",
                "icon": {"name": "firefox"},
                "argv": ["firefox"],
                "graphical": true
            }]
        }]
    }))
    .expect("unsafe-local workloads json serializes")
}

/// The real values `nixos-modules/unsafe-local-workloads-json.nix` emits: the
/// same-uid systemd-user runtime implementation id and an opaque, per-realm
/// derived provider id (never the legacy `unsafe-local` placeholder).
fn minimal_unsafe_local_workloads_json() -> Vec<u8> {
    unsafe_local_workloads_json_with_identity("systemd-user", "wrk-tools-systemd-user")
}

/// Write all sibling artifacts the resolver needs into `dir`.
/// `bundle_path` is the bundle.json path that has already been written;
/// the relative references inside it (`host.json`, etc.) are resolved
/// relative to `dir`.
fn write_siblings(dir: &Path, policy: &BundleVerifyPolicy) {
    let host_path = dir.join("host.json");
    let processes_path = dir.join("processes.json");
    let vms_path = dir.join("vms.json");

    write_private(&host_path, &minimal_host_json());
    write_private(&processes_path, &minimal_processes_json());
    // vms.json is read with std::fs::read (public manifest, no policy).
    // Write with world-readable mode so it works regardless of uid.
    fs::write(&vms_path, minimal_vms_json()).expect("write vms.json");

    // Fix modes to match the policy for host.json and processes.json.
    set_mode_to(&host_path, policy.required_mode);
    set_mode_to(&processes_path, policy.required_mode);
}

fn set_mode_to(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set permissions");
}

/// Helper: assert `err` is `BundleTampered` with the given reason slug.
fn assert_tampered(err: &Error, expected_reason: &str) {
    match err {
        Error::Bundle(BundleError::Tampered { reason, .. }) => {
            assert_eq!(
                reason, expected_reason,
                "expected reason={expected_reason:?} but got {reason:?}"
            );
        }
        other => panic!("expected BundleTampered({expected_reason:?}), got {other:?}"),
    }
}

fn allocator_child(role: &str) -> serde_json::Value {
    let prefix = if role == "controller" {
        "ctrl"
    } else {
        "broker"
    };
    let namespace = |kind: &str| {
        serde_json::json!({
            "refId": format!("{prefix}-{kind}-ns"),
            "digest": format!("sha256:{}", "1".repeat(64))
        })
    };
    serde_json::json!({
        "role": role,
        "processId": format!("{prefix}-process-1"),
        "executableRef": format!("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-{prefix}/bin/{prefix}"),
        "executableDigest": format!("sha256:{}", "2".repeat(64)),
        "configRef": format!("{prefix}-config-v2"),
        "configDigest": format!("sha256:{}", "3".repeat(64)),
        "uid": if role == "controller" { 61001 } else { 61002 },
        "gid": if role == "controller" { 61001 } else { 61002 },
        "listenerRef": format!("{prefix}-listener"),
        "bootstrapSessionRef": format!("{prefix}-bootstrap"),
        "cgroupRef": format!("{prefix}-cgroup"),
        "cgroupDigest": format!("sha256:{}", "4".repeat(64)),
        "stateRootRef": format!("{prefix}-state-root"),
        "auditRootRef": format!("{prefix}-audit-root"),
        "namespaces": {
            "user": namespace("user"),
            "mount": namespace("mount"),
            "network": namespace("network"),
            "ipc": namespace("ipc"),
            "pid": namespace("pid"),
            "cgroup": namespace("cgroup")
        },
        "resourceRefs": [format!("{prefix}-resource-a")],
        "leaseRefs": [format!("{prefix}-lease-a")],
        "spawn": {
            "clone3WithPidfd": true,
            "directCgroupPlacement": true,
            "noNewPrivileges": true,
            "emptyInitialCapabilities": true,
            "executableOnlyArgv": true,
            "closedEnvironment": true,
            "inheritedFdAuthorityOnly": true
        }
    })
}

fn allocator_launch_row() -> serde_json::Value {
    let mut row = serde_json::json!({
        "realmId": "work",
        "realmPath": "work",
        "controllerGeneration": "generation-1",
        "controller": allocator_child("controller"),
        "broker": allocator_child("broker")
    });
    let row_digest = sha256_hex(&serde_json::to_vec(&row).expect("serialize launch row"));
    row.as_object_mut().expect("row object").insert(
        "launchRecordDigest".to_owned(),
        serde_json::Value::String(row_digest),
    );
    row
}

fn allocator_artifact(rows: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "allocator": {
            "enabled": true,
            "runtimeState": "metadata-only",
            "rootSocket": "/run/d2b/allocator/local-root.sock",
            "stateDir": "/var/lib/d2b/allocator",
            "leaseLedger": "/var/lib/d2b/allocator/leases.jsonl",
            "auditDir": "/var/lib/d2b/allocator/audit",
            "runtime": {"spawnsService": false, "socketActivated": false}
        },
        "realms": [{
            "realmName": "work",
            "realmId": "work",
            "realmPath": "work",
            "enabled": true,
            "placement": "host-local",
            "hostMutation": false
        }],
        "processLaunch": rows,
        "invariants": {
            "noRuntimeAllocatorService": true,
            "preservesEnvRuntimeSourceOfTruth": true,
            "privateMetadataOnly": true
        }
    }))
    .expect("allocator artifact")
}

fn write_allocator_bundle(dir: &Path, allocator: &[u8]) -> std::path::PathBuf {
    let policy = current_user_policy();
    let host = minimal_host_json();
    let processes = minimal_processes_json();
    let pre_hash = serde_json::to_vec(&serde_json::json!({
        "artifactHashes": null,
        "bundleVersion": 12,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "allocatorPath": "allocator.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle pre-hash");
    let bundle = bundle_json_with_full_hashes(
        &pre_hash,
        serde_json::json!({
            "host.json": sha256_hex(&host),
            "processes.json": sha256_hex(&processes),
            "allocator.json": sha256_hex(allocator)
        }),
    );

    for (name, bytes) in [
        ("bundle.json", bundle.as_slice()),
        ("host.json", host.as_slice()),
        ("processes.json", processes.as_slice()),
        ("allocator.json", allocator),
    ] {
        let path = dir.join(name);
        write_private(&path, bytes);
        set_mode_to(&path, policy.required_mode);
    }
    fs::write(dir.join("vms.json"), minimal_vms_json()).expect("write vms");
    dir.join("bundle.json")
}

// ---------------------------------------------------------------
// Test 1: symlink at bundle.json → BundleTampered { reason: "symlink" }
// ---------------------------------------------------------------
#[test]
fn tamper_symlink() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");
    let target = dir.path().join("real-bundle.json");

    write_private(&target, &minimal_bundle_json_no_hash());
    std::os::unix::fs::symlink(&target, &bundle_path).expect("create symlink");

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("symlink should be rejected");
    assert_tampered(&err, "symlink");
}

// ---------------------------------------------------------------
// Test 2: owner = wrong uid → BundleTampered { reason: "owner" }
//
// Requires root (or CAP_CHOWN) to call fchown; skipped otherwise.
// ---------------------------------------------------------------
#[test]
fn tamper_owner_wrong_uid() {
    if rustix::process::getuid().as_raw() != 0 {
        eprintln!("tamper_owner_wrong_uid: skipping — not root (cannot chown)");
        return;
    }

    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &minimal_bundle_json_no_hash());

    // Change owner to uid=65534 (nobody) using the system chown binary.
    let status = std::process::Command::new("chown")
        .arg("65534")
        .arg(bundle_path.as_os_str())
        .status()
        .expect("chown command ran");
    assert!(status.success(), "chown 65534 failed: {status}");

    // Use a policy that expects uid=0 so the file fails.
    let policy = BundleVerifyPolicy {
        required_uid: 0,
        required_gid: None,
        required_mode: 0o640,
    };
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("wrong-owner file should be rejected");
    assert_tampered(&err, "owner");
}

// ---------------------------------------------------------------
// Test 3: mode 0644 → BundleTampered { reason: "mode" }
// ---------------------------------------------------------------
#[test]
fn tamper_mode_too_permissive() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // Write with 0o644 (world-readable, not 0o640).
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&bundle_path)
        .expect("create")
        .write_all(&minimal_bundle_json_no_hash())
        .expect("write");

    let policy = current_user_policy(); // expects 0o640
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("wrong-mode file should be rejected");
    assert_tampered(&err, "mode");
}

// ---------------------------------------------------------------
// Test 4: corrupted file (hash mismatch) → BundleTampered { reason: "hash" }
// ---------------------------------------------------------------
#[test]
fn tamper_hash_mismatch() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // Compute a valid hash over the pre-hash bytes.
    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    // Rewrite so the bundleHash field value is intact but the other content
    // differs — replace the first occurrence of the bundleVersion value with
    // a different number to ensure the parsed Value changes.
    let mut value: serde_json::Value = serde_json::from_slice(&with_hash).expect("parse with_hash");
    value["bundleVersion"] = serde_json::json!(99);
    let tampered = serde_json::to_vec(&value).expect("re-serialize tampered");
    write_private(&bundle_path, &tampered);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("corrupted file should be rejected");
    // Could be a parse error (invalid JSON) or hash mismatch depending on
    // whether serde_json tolerates trailing whitespace.  In practice
    // serde_json does allow trailing whitespace so we get hash mismatch.
    // Accept both for robustness.
    match &err {
        Error::Bundle(BundleError::Tampered { reason, .. }) if reason == "hash" => {}
        Error::Manifest(_) => {} // parse failure on truly corrupted JSON is also acceptable
        other => panic!("expected hash tamper or parse error, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 4b: binary-garbage corruption → hash mismatch or parse error
// ---------------------------------------------------------------
#[test]
fn tamper_truncated() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    // Truncate to first 10 bytes — definitely unparseable JSON.
    write_private(&bundle_path, &with_hash[..10]);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("truncated file should be rejected");
    match &err {
        Error::Bundle(BundleError::Tampered { reason, .. }) if reason == "hash" => {}
        Error::Manifest(_) => {} // parse failure is also acceptable
        other => panic!("expected tamper or parse error on truncated file, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 5: all-correct file → loads successfully
// ---------------------------------------------------------------
#[test]
fn loads_correct() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();

    // Build bundle.json with correct self-hash.
    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &with_hash);
    set_mode_to(&bundle_path, policy.required_mode);

    // Write host.json, processes.json, vms.json.
    write_siblings(dir.path(), &policy);

    let resolver = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect("all-correct bundle should load without error");

    assert_eq!(resolver.bundle.bundle_version, 4);
    assert_eq!(resolver.bundle.schema_version, "v2");
}

/// Build bundle JSON with both `bundleHash` (computed from `pre_hash_bytes`)
/// and the supplied `artifact_hashes` map.
///
/// `pre_hash_bytes` must already contain `"artifactHashes": null` so the
/// `bundleHash` commits to that field's presence.
fn bundle_json_with_full_hashes(
    pre_hash_bytes: &[u8],
    artifact_hashes: serde_json::Value,
) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(pre_hash_bytes).expect("pre-hash bundle with null hashes parses");
    let hash = sha256_hex(pre_hash_bytes);
    let obj = value.as_object_mut().expect("bundle is object");
    obj.insert("bundleHash".to_owned(), serde_json::Value::String(hash));
    obj.insert("artifactHashes".to_owned(), artifact_hashes);
    serde_json::to_vec(&value).expect("bundle with full hashes serializes")
}

fn unsafe_local_bundle_pre_hash() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "artifactHashes": null,
        "bundleVersion": 11,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "realmWorkloadsLauncherV2Path": "realm-workloads-launcher-v2.json",
        "unsafeLocalWorkloadsPath": "unsafe-local-workloads.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("unsafe-local bundle pre-hash serializes")
}

fn minimal_realm_workloads_launcher_v2_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "runtimeState": "contract-only",
        "workloads": [],
        "invariants": {
            "argvPrivate": true,
            "providerNeutral": true,
            "typedExecutionPosture": true,
            "realmAccentColorOnly": true,
            "noSecretsOrCredentials": true
        }
    }))
    .expect("launcher v2 fixture serializes")
}

fn write_unsafe_local_bundle_with(
    dir: &Path,
    policy: &BundleVerifyPolicy,
    unsafe_local: &[u8],
) -> std::path::PathBuf {
    let host = minimal_host_json();
    let processes = minimal_processes_json();
    let launcher_v2 = minimal_realm_workloads_launcher_v2_json();
    let hashes = serde_json::json!({
        "host.json": sha256_hex(&host),
        "processes.json": sha256_hex(&processes),
        "realm-workloads-launcher-v2.json": sha256_hex(&launcher_v2),
        "unsafe-local-workloads.json": sha256_hex(unsafe_local)
    });
    let bundle = bundle_json_with_full_hashes(&unsafe_local_bundle_pre_hash(), hashes);
    let bundle_path = dir.join("bundle.json");
    let host_path = dir.join("host.json");
    let processes_path = dir.join("processes.json");
    let launcher_v2_path = dir.join("realm-workloads-launcher-v2.json");
    let unsafe_local_path = dir.join("unsafe-local-workloads.json");
    write_private(&bundle_path, &bundle);
    write_private(&host_path, &host);
    write_private(&processes_path, &processes);
    write_private(&launcher_v2_path, &launcher_v2);
    write_private(&unsafe_local_path, unsafe_local);
    fs::write(dir.join("vms.json"), minimal_vms_json()).expect("write vms.json");
    for path in [
        &bundle_path,
        &host_path,
        &processes_path,
        &launcher_v2_path,
        &unsafe_local_path,
    ] {
        set_mode_to(path, policy.required_mode);
    }
    bundle_path
}

fn write_unsafe_local_bundle(dir: &Path, policy: &BundleVerifyPolicy) -> std::path::PathBuf {
    write_unsafe_local_bundle_with(dir, policy, &minimal_unsafe_local_workloads_json())
}

#[test]
fn loads_hashed_unsafe_local_workloads_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();
    let bundle_path = write_unsafe_local_bundle(dir.path(), &policy);
    let resolver =
        BundleResolver::load_with_policy(&bundle_path, &policy).expect("unsafe-local bundle loads");
    assert_eq!(resolver.bundle.bundle_version, 11);
    assert!(resolver.realm_workloads_launcher_v2.is_some());
    assert!(
        resolver
            .find_unsafe_local_workload("tools.host.d2b")
            .is_some()
    );
}

#[test]
fn rejects_tampered_realm_workloads_launcher_v2_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();
    let bundle_path = write_unsafe_local_bundle(dir.path(), &policy);
    write_private(
        &dir.path().join("realm-workloads-launcher-v2.json"),
        br#"{"schemaVersion":"v2","runtimeState":"contract-only","workloads":[],"invariants":{"argvPrivate":false,"providerNeutral":true,"typedExecutionPosture":true,"realmAccentColorOnly":true,"noSecretsOrCredentials":true}}"#,
    );
    let error = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("tampered launcher-v2 artifact rejects");
    assert_tampered(&error, "hash");
}

#[test]
fn rejects_tampered_unsafe_local_workloads_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();
    let bundle_path = write_unsafe_local_bundle(dir.path(), &policy);
    write_private(
        &dir.path().join("unsafe-local-workloads.json"),
        br#"{"schemaVersion":"v2","workloads":[]}"#,
    );
    let error = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("tampered unsafe-local artifact rejects");
    assert_tampered(&error, "hash");
}

/// Regression coverage for the `UnsafeLocalWorkload::validate` /
/// `LocalVmConfiguredWorkload::validate` runtime/provider identity check:
/// a correctly-hashed (non-tampered) artifact that carries the legacy,
/// never-emitted `unsafe-local`/`nixos` placeholder identity must still be
/// rejected as an invalid schema, not silently accepted. Accepting it would
/// mean the resolver's validation can never reject a genuinely malformed
/// artifact — a self-inflicted parsing denial-of-service surface, since a
/// real Nix-emitted bundle (which always uses `systemd-user` /
/// `cloud-hypervisor` / `qemu-media`) would then be indistinguishable from
/// this stale placeholder shape.
#[test]
fn rejects_unsafe_local_workloads_artifact_with_legacy_placeholder_identity() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();
    let legacy = unsafe_local_workloads_json_with_identity("unsafe-local", "unsafe-local");
    let bundle_path = write_unsafe_local_bundle_with(dir.path(), &policy, &legacy);
    let error = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("legacy placeholder runtimeKind/providerId must fail schema validation");
    match error {
        Error::Manifest(ManifestError::ParseError { artifact, .. }) => {
            assert_eq!(artifact, "unsafe-local-workloads.json");
        }
        other => panic!("expected Manifest ParseError, got {other:?}"),
    }
}

/// A `LocalVmConfiguredWorkload` with the legacy `nixos` placeholder
/// `runtimeKind` (the field the Nix emitter never produces; it always emits
/// `cloud-hypervisor` or `qemu-media`) must also fail schema validation
/// rather than parse silently.
#[test]
fn rejects_local_vm_workload_with_legacy_placeholder_runtime_kind() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();
    let legacy = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "workloads": [],
        "localVmWorkloads": [{
            "identity": {
                "workloadId": "corp-vm",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "corp-vm.host.d2b",
                "runtimeKind": "nixos",
                "providerId": "wrk-corp-vm-cloud-hypervisor"
            },
            "defaultItemId": "browser",
            "items": [{
                "type": "exec",
                "id": "browser",
                "name": "Browser",
                "icon": {"name": "firefox"},
                "argv": ["firefox"],
                "graphical": true
            }]
        }]
    }))
    .expect("legacy local-vm workload fixture serializes");
    let bundle_path = write_unsafe_local_bundle_with(dir.path(), &policy, &legacy);
    let error = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("legacy placeholder runtimeKind must fail schema validation");
    match error {
        Error::Manifest(ManifestError::ParseError { artifact, .. }) => {
            assert_eq!(artifact, "unsafe-local-workloads.json");
        }
        other => panic!("expected Manifest ParseError, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 7: schema v2 bundle with bundleHash deleted →
//         BundleTampered { reason: "missing-bundle-hash" }
// ---------------------------------------------------------------
#[test]
fn tamper_missing_bundle_hash() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // A schema v2 bundle without bundleHash must be rejected outright.
    write_private(&bundle_path, &minimal_bundle_json_no_hash());

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("v2 bundle without bundleHash should be rejected");
    assert_tampered(&err, "missing-bundle-hash");
}

// ---------------------------------------------------------------
// Test 8: artifactHashes present but missing the `processes.json`
//         entry → BundleTampered { reason: "unhashed" }
// ---------------------------------------------------------------
#[test]
fn tamper_artifact_unhashed() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();

    let host_bytes = minimal_host_json();
    let processes_bytes = minimal_processes_json();

    // Bundle declares hashes for host.json but not for processes.json.
    let pre_hash = minimal_bundle_json_with_null_artifact_hashes();
    let artifact_hashes = serde_json::json!({
        "host.json": sha256_hex(&host_bytes),
        // "processes.json" intentionally absent → "unhashed"
    });
    let bundle_bytes = bundle_json_with_full_hashes(&pre_hash, artifact_hashes);

    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &bundle_bytes);

    let host_path = dir.path().join("host.json");
    let processes_path = dir.path().join("processes.json");
    let vms_path = dir.path().join("vms.json");
    write_private(&host_path, &host_bytes);
    write_private(&processes_path, &processes_bytes);
    fs::write(&vms_path, minimal_vms_json()).expect("write vms.json");
    set_mode_to(&host_path, policy.required_mode);
    set_mode_to(&processes_path, policy.required_mode);

    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("processes.json absent from artifactHashes should be rejected");
    assert_tampered(&err, "unhashed");
}

// ---------------------------------------------------------------
// Test 9: processes.json modified after bundle hash computed →
//         BundleTampered { reason: "hash" }
// ---------------------------------------------------------------
#[test]
fn tamper_artifact_hash_mismatch() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();

    let host_bytes = minimal_host_json();
    let processes_bytes = minimal_processes_json();

    // Bundle carries correct hashes for the original artifact content.
    let pre_hash = minimal_bundle_json_with_null_artifact_hashes();
    let artifact_hashes = serde_json::json!({
        "host.json": sha256_hex(&host_bytes),
        "processes.json": sha256_hex(&processes_bytes),
    });
    let bundle_bytes = bundle_json_with_full_hashes(&pre_hash, artifact_hashes);

    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &bundle_bytes);

    let host_path = dir.path().join("host.json");
    let processes_path = dir.path().join("processes.json");
    let vms_path = dir.path().join("vms.json");
    write_private(&host_path, &host_bytes);
    // Write tampered processes.json — different bytes → hash mismatch.
    let tampered = b"{\"schemaVersion\":\"v2\",\"vms\":[],\"tampered\":true}";
    write_private(&processes_path, tampered);
    fs::write(&vms_path, minimal_vms_json()).expect("write vms.json");
    set_mode_to(&host_path, policy.required_mode);
    set_mode_to(&processes_path, policy.required_mode);

    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("tampered processes.json should be rejected");
    assert_tampered(&err, "hash");
}

// ---------------------------------------------------------------
// P0fu3 H1 (security-r2-medium): schemaVersion >= 2 — including
// future v3+ shapes — MUST carry bundleHash. The original code
// path matched `schemaVersion == "v2"` exactly, so a future
// "v3" bundle missing bundleHash would silently downgrade to
// warning-only. These tests fail-closed on that path.
// ---------------------------------------------------------------

fn minimal_bundle_json_no_hash_with_schema(schema_version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "bundleVersion": 4,
        "schemaVersion": schema_version,
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle json serializes")
}

#[test]
fn tamper_missing_bundle_hash_schema_v3() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // A future schema v3 bundle without bundleHash must also be rejected.
    // (The old `is_v2` check would have downgraded this to warning-only.)
    write_private(&bundle_path, &minimal_bundle_json_no_hash_with_schema("v3"));

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("v3 bundle without bundleHash should be rejected");
    assert_tampered(&err, "missing-bundle-hash");
}

#[test]
fn tamper_missing_bundle_hash_unknown_schema_fails_closed() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // An unparseable schemaVersion ("v2-experimental") that is not
    // recognized as the legacy v1 shape must fail closed — we don't
    // know whether the unknown future schema needs bundleHash so we
    // require it.
    write_private(
        &bundle_path,
        &minimal_bundle_json_no_hash_with_schema("v2-experimental"),
    );

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("unknown schemaVersion without bundleHash should be rejected");
    assert_tampered(&err, "missing-bundle-hash");
}

#[test]
fn allocator_launch_authority_is_integrity_loaded_and_exactly_resolved() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = write_allocator_bundle(
        dir.path(),
        &allocator_artifact(vec![allocator_launch_row()]),
    );
    let resolver = BundleResolver::load_with_policy(&bundle_path, &current_user_policy())
        .expect("allocator authority bundle loads");

    let row = resolver
        .find_realm_child_launch_record("work", "generation-1")
        .expect("exact authority row");
    assert_eq!(row.controller.uid, 61001);
    assert_eq!(row.broker.uid, 61002);
    assert!(
        resolver
            .find_realm_child_launch_record("work", "generation-2")
            .is_none()
    );
    assert!(
        resolver
            .find_realm_child_launch_record("Work", "generation-1")
            .is_none()
    );
}

#[test]
fn allocator_launch_authority_has_no_missing_row_fallback() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = write_allocator_bundle(dir.path(), &allocator_artifact(Vec::new()));
    let resolver = BundleResolver::load_with_policy(&bundle_path, &current_user_policy())
        .expect("empty allocator authority loads");
    assert!(
        resolver
            .find_realm_child_launch_record("work", "generation-1")
            .is_none()
    );
}

#[test]
fn allocator_launch_authority_rejects_duplicate_and_mismatched_rows() {
    for allocator in [
        allocator_artifact(vec![allocator_launch_row(), allocator_launch_row()]),
        {
            let mut row = allocator_launch_row();
            row["realmPath"] = serde_json::json!("personal");
            allocator_artifact(vec![row])
        },
    ] {
        let dir = TempDir::new().expect("tempdir");
        let bundle_path = write_allocator_bundle(dir.path(), &allocator);
        let error = BundleResolver::load_with_policy(&bundle_path, &current_user_policy())
            .expect_err("invalid allocator row must fail load");
        assert!(matches!(error, Error::Manifest(_)));
    }
}

#[test]
fn allocator_launch_authority_rejects_artifact_tampering() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = write_allocator_bundle(
        dir.path(),
        &allocator_artifact(vec![allocator_launch_row()]),
    );
    let allocator_path = dir.path().join("allocator.json");
    let mut allocator: serde_json::Value =
        serde_json::from_slice(&fs::read(&allocator_path).expect("read allocator"))
            .expect("parse allocator");
    allocator["processLaunch"][0]["controller"]["uid"] = serde_json::json!(61003);
    write_private(
        &allocator_path,
        &serde_json::to_vec(&allocator).expect("serialize tamper"),
    );
    set_mode_to(&allocator_path, current_user_policy().required_mode);

    let error = BundleResolver::load_with_policy(&bundle_path, &current_user_policy())
        .expect_err("tampered allocator must fail");
    assert_tampered(&error, "hash");
}

// ---------------------------------------------------------------
// io::Write import needed by write_private
// ---------------------------------------------------------------
use std::io::Write as _;
