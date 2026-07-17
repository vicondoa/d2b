use d2b_core::allocator_config::{
    AllocatorEnvBridgeMode, AllocatorJson, AllocatorProviderKind, AllocatorRealmPath,
    MAX_ALLOCATOR_PROCESS_LAUNCH_ROWS, MAX_ALLOCATOR_PROVIDER_KIND_BYTES,
    MAX_ALLOCATOR_REALM_PATH_LABELS,
};
use sha2::Digest as _;

fn digest(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).expect("canonical json");
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn namespace(prefix: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "refId": format!("{prefix}-{kind}-ns"),
        "digest": format!("sha256:{}", "1".repeat(64))
    })
}

fn child(role: &str) -> serde_json::Value {
    let prefix = if role == "controller" {
        "ctrl"
    } else {
        "broker"
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
            "user": namespace(prefix, "user"),
            "mount": namespace(prefix, "mount"),
            "network": namespace(prefix, "network"),
            "ipc": namespace(prefix, "ipc"),
            "pid": namespace(prefix, "pid"),
            "cgroup": namespace(prefix, "cgroup")
        },
        "resourceRefs": [format!("{prefix}-resource-a"), format!("{prefix}-resource-b")],
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

fn launch_row(realm_id: &str, generation: &str) -> serde_json::Value {
    let mut material = serde_json::json!({
        "realmId": realm_id,
        "realmPath": realm_id,
        "controllerGeneration": generation,
        "controller": child("controller"),
        "broker": child("broker")
    });
    let launch_record_digest = digest(&material);
    material.as_object_mut().expect("launch row object").insert(
        "launchRecordDigest".to_owned(),
        serde_json::Value::String(launch_record_digest),
    );
    material
}

fn allocator_json(rows: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
      "schemaVersion": "v2",
      "allocator": {
        "enabled": true,
        "runtimeState": "metadata-only",
        "rootSocket": "/run/d2b/allocator/local-root.sock",
        "stateDir": "/var/lib/d2b/allocator",
        "leaseLedger": "/var/lib/d2b/allocator/leases.jsonl",
        "auditDir": "/var/lib/d2b/allocator/audit",
        "runtime": {
          "spawnsService": false,
          "socketActivated": false
        }
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
    })
}

#[test]
fn allocator_realm_path_bounds_are_enforced() {
    assert!(AllocatorRealmPath::parse("payments.work").is_ok());
    assert!(AllocatorRealmPath::parse("").is_err());
    assert!(AllocatorRealmPath::parse("Work").is_err());
    assert!(AllocatorRealmPath::parse("work..payments").is_err());

    let too_many = std::iter::repeat_n("a", MAX_ALLOCATOR_REALM_PATH_LABELS + 1)
        .collect::<Vec<_>>()
        .join(".");
    assert!(AllocatorRealmPath::parse(too_many).is_err());
}

#[test]
fn allocator_provider_kind_is_bounded_slug() {
    assert!(AllocatorProviderKind::parse("aca").is_ok());
    assert!(AllocatorProviderKind::parse("azure-container-apps").is_ok());
    assert!(AllocatorProviderKind::parse("Azure").is_err());
    assert!(AllocatorProviderKind::parse("aca/provider").is_err());
    assert!(
        AllocatorProviderKind::parse("a".repeat(MAX_ALLOCATOR_PROVIDER_KIND_BYTES + 1)).is_err()
    );
}

#[test]
fn allocator_env_bridge_mode_is_closed_on_decode() {
    let json = r#"{
      "schemaVersion": "v2",
      "allocator": {
        "enabled": true,
        "runtimeState": "metadata-only",
        "rootSocket": "/run/d2b/allocator/local-root.sock",
        "stateDir": "/var/lib/d2b/allocator",
        "leaseLedger": "/var/lib/d2b/allocator/leases.jsonl",
        "auditDir": "/var/lib/d2b/allocator/audit",
        "runtime": {
          "spawnsService": false,
          "socketActivated": false
        }
      },
      "realms": [{
        "realmName": "work",
        "realmId": "work",
        "realmPath": "work",
        "enabled": true,
        "placement": "host-local",
        "hostMutation": false
      }],
      "envBridge": [{
        "realmPath": "work",
        "envName": "work",
        "declared": true,
        "enabled": true,
        "mode": "inherit-env"
      }],
      "invariants": {
        "noRuntimeAllocatorService": true,
        "preservesEnvRuntimeSourceOfTruth": true,
        "privateMetadataOnly": true
      }
    }"#;
    let parsed: AllocatorJson = serde_json::from_str(json).expect("closed mode parses");
    assert_eq!(
        parsed.env_bridge[0].mode,
        Some(AllocatorEnvBridgeMode::InheritEnv)
    );

    let bad = json.replace("\"inherit-env\"", "\"surprise\"");
    assert!(serde_json::from_str::<AllocatorJson>(&bad).is_err());
}

#[test]
fn process_launch_row_is_typed_and_exactly_resolvable() {
    let parsed: AllocatorJson =
        serde_json::from_value(allocator_json(vec![launch_row("work", "generation-1")]))
            .expect("valid launch authority");
    let row = parsed
        .find_process_launch("work", "generation-1")
        .expect("exact row");
    assert_eq!(row.realm_id.as_str(), "work");
    assert_eq!(row.controller.process_id.as_str(), "ctrl-process-1");
    assert_eq!(row.controller.executable_digest.as_bytes(), &[0x22; 32]);
    assert!(parsed.find_process_launch("work", "generation-2").is_none());
    assert!(parsed.find_process_launch("Work", "generation-1").is_none());
}

#[test]
fn process_launch_rejects_duplicate_unsorted_and_excess_rows() {
    let duplicate = allocator_json(vec![
        launch_row("work", "generation-1"),
        launch_row("work", "generation-1"),
    ]);
    assert!(serde_json::from_value::<AllocatorJson>(duplicate).is_err());

    let unsorted = allocator_json(vec![
        launch_row("work", "generation-2"),
        launch_row("work", "generation-1"),
    ]);
    assert!(serde_json::from_value::<AllocatorJson>(unsorted).is_err());

    let rows = (0..=MAX_ALLOCATOR_PROCESS_LAUNCH_ROWS)
        .map(|index| launch_row("work", &format!("generation-{index:03}")))
        .collect();
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(rows)).is_err());
}

#[test]
fn process_launch_rejects_missing_and_mismatched_pair_fields() {
    let mut missing = launch_row("work", "generation-1");
    missing
        .as_object_mut()
        .expect("row")
        .remove("broker")
        .expect("broker");
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![missing])).is_err());

    let mut wrong_role = launch_row("work", "generation-1");
    wrong_role["controller"]["role"] = serde_json::json!("broker");
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![wrong_role])).is_err());

    let mut wrong_realm = launch_row("work", "generation-1");
    wrong_realm["realmPath"] = serde_json::json!("personal");
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![wrong_realm])).is_err());
}

#[test]
fn process_launch_rejects_tampering_and_ambient_authority() {
    let mut tampered = launch_row("work", "generation-1");
    tampered["controller"]["uid"] = serde_json::json!(61003);
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![tampered])).is_err());

    let mut argv = launch_row("work", "generation-1");
    argv["controller"]["argv"] = serde_json::json!(["/bin/sh"]);
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![argv])).is_err());

    for (field, value) in [
        ("configRef", "/etc/d2b/controller.json"),
        ("configRef", "controller-secret"),
        ("executableRef", "/usr/bin/d2bd"),
    ] {
        let mut ambient = launch_row("work", "generation-1");
        ambient["controller"][field] = serde_json::json!(value);
        assert!(
            serde_json::from_value::<AllocatorJson>(allocator_json(vec![ambient])).is_err(),
            "{field} accepted ambient authority {value}"
        );
    }

    let mut unsafe_spawn = launch_row("work", "generation-1");
    unsafe_spawn["controller"]["spawn"]["emptyInitialCapabilities"] = serde_json::json!(false);
    assert!(serde_json::from_value::<AllocatorJson>(allocator_json(vec![unsafe_spawn])).is_err());

    let mut duplicate_resource = launch_row("work", "generation-1");
    duplicate_resource["controller"]["resourceRefs"] =
        serde_json::json!(["resource-a", "resource-a"]);
    assert!(
        serde_json::from_value::<AllocatorJson>(allocator_json(vec![duplicate_resource])).is_err()
    );
}

#[test]
fn process_launch_debug_redacts_authority() {
    let parsed: AllocatorJson =
        serde_json::from_value(allocator_json(vec![launch_row("work", "generation-1")]))
            .expect("valid launch authority");
    let debug = format!("{:?}", parsed.process_launch[0]);
    for forbidden in [
        "work",
        "generation-1",
        "ctrl-process-1",
        "/nix/store",
        "ctrl-listener",
        &"2".repeat(64),
    ] {
        assert!(!debug.contains(forbidden), "Debug leaked {forbidden}");
    }
    assert!(debug.contains("<redacted>"));
}
