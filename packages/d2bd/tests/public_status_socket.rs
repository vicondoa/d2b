mod common;

mod public_status_socket {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::common::{
        DaemonFixture, HELLO_FRAME, TestPeer, spawn_d2bd_serve, test_client,
        write_daemon_config_with_artifacts,
    };

    fn write_status_artifacts(root: &Path) -> Value {
        let public_manifest_path = root.join("vms.json");
        let bundle_path = root.join("bundle.json");
        let host_path = root.join("host.json");
        let processes_path = root.join("processes.json");
        let provider_registry_path = root.join("provider-registry-v2.json");
        let closures_dir = root.join("closures");
        fs::create_dir_all(&closures_dir).expect("create closures dir");

        fs::write(
            &public_manifest_path,
            serde_json::to_vec(&json!({
                "_manifest": { "manifestVersion": 6 },
                "_observability": {
                    "enabled": false,
                    "signozUrl": "http://127.0.0.1:8080",
                    "signozOtlpGrpcPort": 4317,
                    "signozOtlpHttpPort": 4318,
                    "obsVsockCid": 1000,
                    "obsVsockHostSocket": "/run/d2b/obs.sock",
                    "vmName": "sys-obs"
                },
                "vm-a": {
                    "name": "vm-a",
                    "apiSocket": "/run/d2b/vm-a.sock",
                    "audioService": null,
                    "audioStateFile": null,
                    "bridge": "br-work-lan",
                    "env": "work",
                    "gpuSocket": null,
                    "staticIp": "10.20.0.10",
                    "sshUser": "alice",
                    "isNetVm": false,
                    "netVm": "sys-work-net",
                    "stateDir": root.join("vm-a-state").display().to_string(),
                    "graphics": false,
                    "tpm": false,
                    "tpmSocket": null,
                    "usbipYubikey": false,
                    "usbipdHostIp": null,
                    "audio": false,
                    "tap": "work-l2",
                    "observability": {
                        "agentSocket": "/run/d2b/otlp.sock",
                        "enabled": false,
                        "vsockCid": 110,
                        "vsockHostSocket": "/run/d2b/vm-a-vsock.sock"
                    },
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "driver": "cloud-hypervisor",
                            "id": "local-cloud-hypervisor",
                            "type": "local"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": false,
                            "usbHotplug": false,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    }
                },
                "vm-b": {
                    "name": "vm-b",
                    "apiSocket": "/run/d2b/vm-b.sock",
                    "audioService": null,
                    "audioStateFile": null,
                    "bridge": "br-work-lan",
                    "env": "work",
                    "gpuSocket": null,
                    "staticIp": "10.20.0.11",
                    "sshUser": "bob",
                    "isNetVm": false,
                    "netVm": "sys-work-net",
                    "stateDir": root.join("vm-b-state").display().to_string(),
                    "graphics": false,
                    "tpm": false,
                    "tpmSocket": null,
                    "usbipYubikey": false,
                    "usbipdHostIp": null,
                    "audio": false,
                    "tap": "work-l3",
                    "observability": {
                        "agentSocket": "/run/d2b/otlp.sock",
                        "enabled": false,
                        "vsockCid": 111,
                        "vsockHostSocket": "/run/d2b/vm-b-vsock.sock"
                    },
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "driver": "cloud-hypervisor",
                            "id": "local-cloud-hypervisor",
                            "type": "local"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": false,
                            "usbHotplug": false,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    }
                }
            }))
            .expect("serialize manifest"),
        )
        .expect("write manifest");
        fs::write(
            &processes_path,
            serde_json::to_vec(&json!({ "schemaVersion": "v2", "vms": [] }))
                .expect("serialize processes"),
        )
        .expect("write processes");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/deny-unknown/host-valid.json"),
            &host_path,
        )
        .expect("copy host fixture");
        fs::write(
            &provider_registry_path,
            serde_json::to_vec(&json!({
                "schemaVersion": "v2",
                "registryGeneration": 1,
                "configurationFingerprint": "0".repeat(64),
                "publishedAtUnixMs": 0,
                "providers": []
            }))
            .expect("serialize explicit empty provider registry"),
        )
        .expect("write explicit empty provider registry");
        fs::write(
            &bundle_path,
            serde_json::to_vec(&json!({
                "bundleVersion": 12,
                "schemaVersion": "v1",
                "publicManifestPath": "vms.json",
                "hostPath": "host.json",
                "processesPath": "processes.json",
                "privilegesPath": "privileges.json",
                "providerRegistryV2Path": "provider-registry-v2.json",
                "closures": [],
                "minijailProfiles": [],
                "managedKeys": {},
                "generation": {
                    "generator": "public-status-socket-test",
                    "sourceRevision": null,
                    "generatedAt": null
                }
            }))
            .expect("serialize bundle"),
        )
        .expect("write bundle");
        for path in [
            &public_manifest_path,
            &bundle_path,
            &host_path,
            &processes_path,
            &provider_registry_path,
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o640)).expect("chmod artifact");
        }
        json!({
            "publicManifestPath": public_manifest_path.display().to_string(),
            "bundlePath": bundle_path.display().to_string(),
            "hostPath": host_path.display().to_string(),
            "processesPath": processes_path.display().to_string(),
            "closuresDir": closures_dir.display().to_string()
        })
    }

    #[test]
    fn unfiltered_status_over_public_socket_preserves_all_vm_order() {
        let fixture = DaemonFixture::new("public-status-socket.");
        let artifacts = write_status_artifacts(fixture.root());
        write_daemon_config_with_artifacts(
            &fixture,
            &["launcher-user"],
            &["admin-user"],
            Some(artifacts),
        );
        let server = spawn_d2bd_serve(&fixture, &TestPeer::launcher(), true, None);

        let (rc, output) = test_client(
            &fixture.socket_path,
            &[HELLO_FRAME, r#"{"type":"status","checkBridges":false}"#],
        );
        let status = server.wait();
        assert!(status.success(), "d2bd serve exited with {status:?}");
        assert_eq!(rc, 0, "status request succeeds; output:\n{output}");
        let frame = output
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|value| value.get("type").and_then(Value::as_str) == Some("statusResponse"))
            .unwrap_or_else(|| panic!("missing statusResponse frame:\n{output}"));
        let names = frame
            .pointer("/status/entries")
            .and_then(Value::as_array)
            .expect("status entries")
            .iter()
            .map(|entry| {
                entry
                    .get("vm")
                    .and_then(Value::as_str)
                    .expect("vm name")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["vm-a", "vm-b"]);
    }
}
