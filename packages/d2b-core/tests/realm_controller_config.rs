use d2b_core::realm_controller_config::RealmControllersJson;
use serde_json::json;

fn realm_controllers_fixture() -> serde_json::Value {
    json!({
        "schemaVersion": "v2",
        "runtimeState": "metadata-only",
        "controllers": [
            {
                "realmName": "Work",
                "realmId": "work",
                "realmPath": "corp.work",
                "placement": "host-local",
                "daemon": {
                    "user": "d2br-0123456789abcdef",
                    "group": "d2br-0123456789abcdef",
                    "publicSocketGroup": "d2br-0123456789abcdef",
                    "serviceName": "d2b-realm-work-daemon.service",
                    "configPath": "/etc/d2b/realms/work/daemon-config.json",
                    "stateLockPath": "/run/d2b/realms/work/daemon.lock",
                    "locksDir": "/run/d2b/realms/work/locks",
                    "socketActivated": false,
                    "materializedService": false
                },
                "broker": {
                    "enabled": true,
                    "hostMutation": false,
                    "user": "root",
                    "group": "d2br-0123456789abcdef",
                    "socketPath": "/run/d2b/realms/work/priv.sock",
                    "socketUnitName": "d2b-realm-work-priv-broker.socket",
                    "serviceUnitName": "d2b-realm-work-priv-broker.service",
                    "auditDir": "/var/lib/d2b/realms/work/audit",
                    "materializedSocket": false,
                    "materializedService": false
                },
                "paths": {
                    "runDir": "/run/d2b/realms/work",
                    "stateDir": "/var/lib/d2b/realms/work",
                    "auditDir": "/var/lib/d2b/realms/work/audit"
                },
                "sockets": {
                    "publicSocketPath": "/run/d2b/realms/work/public.sock",
                    "brokerSocketPath": "/run/d2b/realms/work/priv.sock"
                },
                "allocator": {
                    "kind": "local-root-metadata",
                    "configPath": "/etc/d2b/allocator.json",
                    "rootSocket": "/run/d2b/allocator.sock",
                    "resourceRequestRefs": ["storage:realm/work"]
                },
                "access": {
                    "allowedUsers": ["alice"],
                    "allowedGroups": ["d2b"],
                    "inheritedAdminUsers": ["admin"]
                },
                "localRuntime": {
                    "runtimeState": "metadata-only",
                    "providers": [
                        {
                            "kind": "nixos",
                            "provider": {
                                "id": "local-cloud-hypervisor",
                                "driver": "cloud-hypervisor",
                                "type": "local"
                            },
                            "capabilities": {
                                "lifecycle": true,
                                "display": true,
                                "usbHotplug": true,
                                "guestControl": true,
                                "exec": true,
                                "configSync": true,
                                "ssh": true,
                                "storeSync": true,
                                "keys": true,
                                "inGuestObservability": true
                            },
                            "operationCapabilities": {
                                "lifecycle": {
                                    "start": true,
                                    "stop": true,
                                    "restart": true,
                                    "switch": true,
                                    "hostPrepare": true
                                },
                                "media": {
                                    "usbHotplug": true,
                                    "removableMedia": false,
                                    "qemuMedia": false
                                },
                                "display": {
                                    "display": true,
                                    "graphics": true,
                                    "video": true,
                                    "waylandProxy": true
                                },
                                "guest": {
                                    "guestControl": true,
                                    "exec": true,
                                    "shell": true,
                                    "configSync": true,
                                    "ssh": true,
                                    "keys": true,
                                    "inGuestObservability": true
                                },
                                "storage": {
                                    "storeSync": true,
                                    "virtiofs": true,
                                    "volumes": true
                                }
                            },
                            "autostartPolicy": "host-boot-eligible",
                            "services": [
                                { "id": "host-reconcile", "role": "host", "optional": false },
                                { "id": "cloud-hypervisor", "role": "hypervisor", "optional": false }
                            ]
                        }
                    ],
                    "workloads": [
                        {
                            "workloadId": "corp-vm",
                            "vmName": "corp-vm",
                            "env": "work",
                            "runtime": {
                                "kind": "nixos",
                                "provider": {
                                    "id": "local-cloud-hypervisor",
                                    "driver": "cloud-hypervisor",
                                    "type": "local"
                                },
                                "capabilities": {
                                    "lifecycle": true,
                                    "display": true,
                                    "usbHotplug": true,
                                    "guestControl": true,
                                    "exec": true,
                                    "configSync": true,
                                    "ssh": true,
                                    "storeSync": true,
                                    "keys": true,
                                    "inGuestObservability": true
                                },
                                "operationCapabilities": {
                                    "lifecycle": {
                                        "start": true,
                                        "stop": true,
                                        "restart": true,
                                        "switch": true,
                                        "hostPrepare": true
                                    },
                                    "media": {
                                        "usbHotplug": true,
                                        "removableMedia": false,
                                        "qemuMedia": false
                                    },
                                    "display": {
                                        "display": true,
                                        "graphics": true,
                                        "video": true,
                                        "waylandProxy": true
                                    },
                                    "guest": {
                                        "guestControl": true,
                                        "exec": true,
                                        "shell": true,
                                        "configSync": true,
                                        "ssh": true,
                                        "keys": true,
                                        "inGuestObservability": true
                                    },
                                    "storage": {
                                        "storeSync": true,
                                        "virtiofs": true,
                                        "volumes": true
                                    }
                                },
                                "autostartPolicy": "host-boot-eligible",
                                "services": [
                                    { "id": "host-reconcile", "role": "host", "optional": false },
                                    { "id": "cloud-hypervisor", "role": "hypervisor", "optional": false }
                                ]
                            },
                            "paths": {
                                "stateDir": "/var/lib/d2b/vms/corp-vm",
                                "runDir": "/run/d2b/vms/corp-vm",
                                "storeView": "/var/lib/d2b/vms/corp-vm/store-view",
                                "guestControlDir": "/run/d2b/vms/corp-vm/guest-control"
                            }
                        }
                    ],
                    "invariants": {
                        "metadataOnly": true,
                        "existingGlobalVmPathsPreserved": true,
                        "noStateMigrationDuringActivation": true,
                        "brokerEffectsRemainRealmDelegated": true
                    }
                },
                "providers": [
                    {
                        "providerName": "entra",
                        "providerId": "entra",
                        "enabled": true,
                        "kind": "entra",
                        "placement": "provider-controller",
                        "capabilityRefs": ["login"],
                        "configRef": "provider:entra"
                    }
                ]
            }
        ],
        "invariants": {
            "metadataOnly": true,
            "noSystemdUnitsMaterialized": true,
            "preservesGlobalDaemonBehavior": true,
            "preservesDirectUnixSocketSemantics": true
        }
    })
}

#[test]
fn realm_controller_metadata_parses_and_validates_socket_path_user_group_metadata() {
    let config: RealmControllersJson =
        serde_json::from_value(realm_controllers_fixture()).expect("fixture parses");

    let summary = config
        .validate_metadata_only()
        .expect("metadata-only config validates");
    assert_eq!(summary.controller_count, 1);
    assert_eq!(summary.host_local_controller_count, 1);
    assert_eq!(summary.broker_enabled_count, 1);

    let controller = &config.controllers[0];
    assert_eq!(controller.daemon.user.as_str(), "d2br-0123456789abcdef");
    assert_eq!(controller.daemon.group.as_str(), "d2br-0123456789abcdef");
    assert_eq!(
        controller.daemon.public_socket_group.as_str(),
        "d2br-0123456789abcdef"
    );
    assert_eq!(
        controller.broker.socket_path.as_str(),
        controller.sockets.broker_socket_path.as_str()
    );
    assert_eq!(
        controller.sockets.public_socket_path.as_str(),
        "/run/d2b/realms/work/public.sock"
    );
    let local_runtime = controller
        .local_runtime
        .as_ref()
        .expect("local runtime metadata exists");
    assert_eq!(local_runtime.providers.len(), 1);
    assert_eq!(
        local_runtime.providers[0].provider.id.as_str(),
        "local-cloud-hypervisor"
    );
    assert_eq!(local_runtime.workloads.len(), 1);
    assert_eq!(local_runtime.workloads[0].vm_name.as_str(), "corp-vm");
    assert!(
        local_runtime.workloads[0]
            .runtime
            .operation_capabilities
            .guest
            .exec
    );
}

#[test]
fn realm_controller_metadata_uses_strict_serde() {
    let mut config = realm_controllers_fixture();
    config["controllers"][0]["unexpected"] = json!(true);

    let err = serde_json::from_value::<RealmControllersJson>(config)
        .expect_err("unknown controller field is rejected");
    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn realm_controller_metadata_rejects_materialized_runtime_and_socket_drift() {
    let mut materialized = realm_controllers_fixture();
    materialized["controllers"][0]["broker"]["materializedService"] = json!(true);
    let config: RealmControllersJson =
        serde_json::from_value(materialized).expect("materialized fixture parses");
    assert!(
        config
            .validate_metadata_only()
            .expect_err("materialized broker service is invalid")
            .to_string()
            .contains("broker.materializedService")
    );

    let mut mismatched = realm_controllers_fixture();
    mismatched["controllers"][0]["broker"]["socketPath"] = json!("/run/d2b/other.sock");
    let config: RealmControllersJson =
        serde_json::from_value(mismatched).expect("mismatched fixture parses");
    assert!(
        config
            .validate_metadata_only()
            .expect_err("broker socket must match socket metadata")
            .to_string()
            .contains("broker.socketPath")
    );
}

#[test]
fn realm_controller_metadata_accepts_emitted_host_local_unit_metadata() {
    let mut materialized = realm_controllers_fixture();
    materialized["controllers"][0]["daemon"]["materializedService"] = json!(true);
    materialized["controllers"][0]["broker"]["materializedSocket"] = json!(true);
    materialized["controllers"][0]["broker"]["materializedService"] = json!(true);
    materialized["invariants"]["noSystemdUnitsMaterialized"] = json!(false);

    let config: RealmControllersJson =
        serde_json::from_value(materialized).expect("materialized fixture parses");
    let summary = config
        .validate_metadata_only()
        .expect("host-local unit metadata remains metadata-only");

    assert_eq!(summary.controller_count, 1);
    assert_eq!(summary.host_local_controller_count, 1);
    assert_eq!(summary.broker_enabled_count, 1);
}

#[test]
fn realm_controller_metadata_rejects_invalid_local_runtime_metadata() {
    let mut non_host_local = realm_controllers_fixture();
    non_host_local["controllers"][0]["placement"] = json!("provider-controller");
    let config: RealmControllersJson =
        serde_json::from_value(non_host_local).expect("non-host-local fixture parses");
    assert!(
        config
            .validate_metadata_only()
            .expect_err("local runtime is host-local only")
            .to_string()
            .contains("localRuntime metadata")
    );

    let mut missing_provider = realm_controllers_fixture();
    missing_provider["controllers"][0]["localRuntime"]["providers"] = json!([]);
    let config: RealmControllersJson =
        serde_json::from_value(missing_provider).expect("missing provider fixture parses");
    assert!(
        config
            .validate_metadata_only()
            .expect_err("workload runtime provider must be declared")
            .to_string()
            .contains("references undeclared provider")
    );
}
