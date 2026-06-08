//! Regression coverage: BundleResolver runner-intent loading must succeed
//! for a representative `vm start --apply` shape.
//!
//! Context: during the v1.0 closeout side-task,
//! `nixling vm start --apply <vm>` from the new CLI returned an
//! `internal-io` envelope on the operator's live host. This focused
//! integration test exercises the resolver against a fixture bundle whose
//! shape mirrors the live
//! `/etc/nixling/bundle.json`. This file lands that regression
//! coverage: the test constructs a tempdir bundle with a non-empty
//! `processes.json` containing a `CloudHypervisorRunner` node (the
//! common case for VM start), invokes `BundleResolver::load`, and
//! asserts the resulting `runner_intents` map contains the expected
//! entries. The original operator failure could not be reproduced
//! remotely (it required the actual host's filesystem layout), but
//! this test guards against the regression class going forward.

use nixling_core::bundle_resolver::{BundleResolver, BundleVerifyPolicy};
use sha2::Digest as _;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;
use tempfile::TempDir;

fn current_user_policy() -> BundleVerifyPolicy {
    BundleVerifyPolicy {
        required_uid: rustix::process::getuid().as_raw(),
        required_gid: Some(rustix::process::getgid().as_raw()),
        required_mode: 0o640,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let digest: [u8; 32] = sha2::Sha256::digest(data).into();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn write_private(path: &Path, content: &[u8]) {
    use std::io::Write as _;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o640)
        .open(path)
        .expect("create file")
        .write_all(content)
        .expect("write file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).expect("set 0640");
}

fn minimal_host_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "site": { "allowUnsafeEastWest": false },
        "environments": [],
        "nftables": {
            "family": "inet",
            "table": "nixling",
            "chains": [],
            "tableHashAfterApply": null,
            "ownershipId": "test"
        },
        "networkManager": {
            "filePath": "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf",
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
            "startMarker": "# nixling-managed begin",
            "endMarker": "# nixling-managed end",
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

/// `processes.json` with a single workload VM whose DAG has one
/// CloudHypervisorRunner node and one Virtiofsd node — the minimum
/// shape that exercises `build_runner_intents` for both per-role
/// SpawnRunner intents.
fn processes_json_with_runner_intents() -> Vec<u8> {
    let role_profile = serde_json::json!({
        "profileId": "test-profile",
        "uid": 0,
        "gid": 0,
        "adr_carve_out": null,
        "caps": [],
        "namespaces": {
            "mount": false,
            "pid": false,
            "net": false,
            "ipc": false,
            "uts": false,
            "user": false
        },
        "seccompPolicyRef": null,
        "mountPolicy": {
            "readOnlyPaths": [],
            "writablePaths": [],
            "nixStoreReadOnly": true,
            "hideDeviceNodesByDefault": false
        },
        "cgroupPlacement": {
            "subtree": "nixling.slice/test",
            "controllers": [],
            "delegated": false
        }
    });
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": "v2",
        "vms": [{
            "vm": "personal-dev",
            "nodes": [
                {
                    "id": "cloud-hypervisor:personal-dev",
                    "role": "cloud-hypervisor-runner",
                    "unit": null,
                    "binaryPath": "/run/current-system/sw/bin/cloud-hypervisor",
                    "argv": ["microvm@personal-dev"],
                    "profile": role_profile,
                    "readiness": []
                },
                {
                    "id": "virtiofsd:personal-dev:store",
                    "role": "virtiofsd",
                    "unit": null,
                    "binaryPath": "/run/current-system/sw/bin/virtiofsd",
                    "argv": ["microvm-virtiofsd@personal-dev"],
                    "profile": role_profile,
                    "readiness": []
                }
            ],
            "edges": [
                {
                    "from": "virtiofsd:personal-dev:store",
                    "to": "cloud-hypervisor:personal-dev",
                    "reason": "virtiofsd-must-precede-vmm"
                }
            ],
            "invariants": {
                "swtpmPreStartFlush": false,
                "perVmAuditPipeline": false,
                "usbipGating": false,
                "tpmOwnershipMigrationWithoutRunningVmMutation": true
            }
        }]
    }))
    .expect("processes json serializes")
}

fn minimal_vms_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "_manifest": {
            "manifestVersion": 3
        },
        "_observability": {
            "chExporter": { "listenPort": 9100 },
            "enabled": false,
            "grafanaUrl": "",
            "obsVsockCid": 0,
            "obsVsockHostSocket": "",
            "vmName": ""
        }
    }))
    .expect("vms json serializes")
}

fn bundle_json_no_hash_with(processes_path: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "bundleVersion": 4,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": processes_path,
        "privilegesPath": "privileges.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/nixling/keys",
            "knownHostsPath": "/var/lib/nixling/known_hosts.nixling",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle pre-hash serializes")
}

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

/// Setup a bundle tempdir with a processes.json containing real
/// runner intents and return the loaded BundleResolver.
fn load_bundle_with_runner_intents() -> (TempDir, BundleResolver) {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();

    let host_path = dir.path().join("host.json");
    let processes_path = dir.path().join("processes.json");
    let vms_path = dir.path().join("vms.json");
    let bundle_path = dir.path().join("bundle.json");

    write_private(&host_path, &minimal_host_json());
    write_private(&processes_path, &processes_json_with_runner_intents());
    fs::write(&vms_path, minimal_vms_json()).expect("write vms.json");

    let pre_hash = bundle_json_no_hash_with("processes.json");
    let with_hash = bundle_json_with_hash(&pre_hash);
    write_private(&bundle_path, &with_hash);

    let resolver = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect("BundleResolver loads runner-intent-bearing bundle");
    (dir, resolver)
}

#[test]
fn bundle_resolver_loads_runner_intents_for_workload_vm() {
    let (_dir, resolver) = load_bundle_with_runner_intents();

    let intent_ids: Vec<&str> = resolver.runner_intent_ids().collect();
    assert!(
        !intent_ids.is_empty(),
        "expected non-empty runner_intents from processes.json with CloudHypervisorRunner + Virtiofsd nodes; got: {intent_ids:?}"
    );

    // The cloud-hypervisor and virtiofsd nodes are both runner-shaped
    // (vs HostReconcile/StoreVirtiofsPreflight/GuestSshReadiness which
    // are readiness-only and skipped by build_runner_intents).
    let ch_present = intent_ids.iter().any(|id| id.contains("cloud-hypervisor"));
    let vfsd_present = intent_ids.iter().any(|id| id.contains("virtiofsd"));
    assert!(
        ch_present,
        "expected a cloud-hypervisor runner intent in {intent_ids:?}"
    );
    assert!(
        vfsd_present,
        "expected a virtiofsd runner intent in {intent_ids:?}"
    );
}

#[test]
fn bundle_resolver_find_runner_intent_returns_resolved_entry() {
    let (_dir, resolver) = load_bundle_with_runner_intents();

    // Pick any intent and confirm find_runner_intent returns it.
    let any_id: String = resolver
        .runner_intent_ids()
        .next()
        .expect("at least one runner intent")
        .to_owned();
    let resolved = resolver
        .find_runner_intent(&any_id)
        .expect("find_runner_intent returns the entry we just enumerated");
    assert_eq!(resolved.intent_id, any_id);
}
