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
        let closures_dir = root.join("closures");
        fs::create_dir_all(&closures_dir).expect("create closures dir");

        fs::write(
            &public_manifest_path,
            serde_json::to_vec(&json!({
                "_manifest": { "manifestVersion": 6 },
                "vm-a": {
                    "name": "vm-a",
                    "env": "work",
                    "staticIp": "10.20.0.10",
                    "sshUser": "alice",
                    "isNetVm": false,
                    "graphics": false,
                    "tpm": false,
                    "usbipYubikey": false,
                    "audio": false,
                    "runtime": {
                        "kind": "nixos",
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
                    "env": "work",
                    "staticIp": "10.20.0.11",
                    "sshUser": "bob",
                    "isNetVm": false,
                    "graphics": false,
                    "tpm": false,
                    "usbipYubikey": false,
                    "audio": false,
                    "runtime": {
                        "kind": "nixos",
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
            &bundle_path,
            serde_json::to_vec(&json!({
                "bundleVersion": 4,
                "schemaVersion": "v2",
                "publicManifestPath": public_manifest_path.display().to_string(),
                "hostPath": host_path.display().to_string(),
                "processesPath": processes_path.display().to_string(),
                "privilegesPath": root.join("privileges.json").display().to_string(),
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
