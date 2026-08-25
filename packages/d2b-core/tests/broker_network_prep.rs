use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::{
    BundleResolver, intent_id_bridge_env, intent_id_nft_projection_env,
    intent_id_ownership_marker_env,
};
use d2b_core::host::HostJson;
use d2b_core::manifest_v04::ManifestV04;
use d2b_core::processes::ProcessesJson;

const HOST_JSON: &str = include_str!("../../../tests/fixtures/deny-unknown/host-valid.json");
const MANIFEST_JSON: &[u8] = include_bytes!("../../../tests/golden/manifest_v04/baseline-vms.json");

#[test]
fn trusted_bundle_resolves_network_operation_rows_without_wire_paths() {
    let mut host: HostJson = serde_json::from_str(HOST_JSON).expect("host fixture parses");
    host.nftables.ownership_id = "bundle-owner".to_owned();
    let env = host.environments[0].env.clone();
    let expected_bridge = host.environments[0].bridge.clone();
    let expected_mtu = host.environments[0].mtu;
    let bundle_hash = format!("sha256:{}", "a".repeat(64));
    let resolver = BundleResolver::from_artifacts(
        Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            realm_workloads_launcher_v2_path: None,
            unsafe_local_workloads_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: Some(bundle_hash.clone()),
            artifact_hashes: None,
        },
        host,
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        ManifestV04::from_slice(MANIFEST_JSON).expect("manifest fixture parses"),
    );

    let bridge = resolver
        .find_bridge_intent(&intent_id_bridge_env(&env))
        .expect("bridge intent resolves");
    assert_eq!(bridge.bridge_ifname, expected_bridge);
    assert_eq!(bridge.mtu, expected_mtu);
    assert!(bridge.stp_disabled);
    assert!(bridge.multicast_snooping_disabled);
    assert!(bridge.ipv6_suppressed);

    let marker = resolver
        .find_ownership_marker_intent(&intent_id_ownership_marker_env(&env))
        .expect("ownership marker resolves");
    assert_eq!(marker.marker, format!("bundle-owner:env:{env}"));

    let projection = resolver
        .find_nft_projection_intent(&intent_id_nft_projection_env(&env))
        .expect("nft projection resolves");
    assert_eq!(projection.ownership_marker_intent_ref, marker.intent_id);
    assert!(!projection.script_body.is_empty());
    assert!(projection.desired_hash.starts_with("fnv1a64:"));

    assert_eq!(
        resolver
            .installed_generation_identity()
            .expect("installed generation resolves")
            .as_str(),
        bundle_hash
    );
}

#[test]
fn installed_generation_identity_fails_closed_on_invalid_or_absent_hash() {
    for bundle_hash in [Some(format!("sha256:{}", "A".repeat(64))), None] {
        let resolver = resolver_with_bundle_hash(bundle_hash);
        assert!(resolver.installed_generation_identity().is_none());
    }
}

fn resolver_with_bundle_hash(bundle_hash: Option<String>) -> BundleResolver {
    BundleResolver::from_artifacts(
        Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            realm_workloads_launcher_v2_path: None,
            unsafe_local_workloads_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash,
            artifact_hashes: None,
        },
        serde_json::from_str(HOST_JSON).expect("host fixture parses"),
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        ManifestV04::from_slice(MANIFEST_JSON).expect("manifest fixture parses"),
    )
}
