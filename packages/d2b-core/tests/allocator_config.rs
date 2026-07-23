use d2b_core::allocator_config::{
    AllocatorEnvBridgeMode, AllocatorJson, AllocatorProviderKind, AllocatorRealmPath,
    MAX_ALLOCATOR_PROVIDER_KIND_BYTES, MAX_ALLOCATOR_REALM_PATH_LABELS,
};

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
