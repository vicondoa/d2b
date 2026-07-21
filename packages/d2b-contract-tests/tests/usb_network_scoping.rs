use d2b_contract_tests::read_repo_file;
use std::{env, fs, path::PathBuf};

#[test]
fn usbip_uses_exclusive_realm_allocator_lease() {
    let devices = read_repo_file("nixos-modules/realm-device-rows.nix");
    let workloads = read_repo_file("nixos-modules/workload-process-rows.nix");

    for required in [
        r#"[ "usbip" "fido" ]"#,
        r#"leaseId = "lease-device-security-key-global";"#,
        r#"share = "exclusive";"#,
        r#"phase = 50;"#,
        r#"kind = "realm-broker";"#,
        r#"refName = row.providerId;"#,
    ] {
        assert!(
            devices.contains(required),
            "USBIP realm lease policy missing {required:?}"
        );
    }

    assert!(
        workloads.contains("cfg._index.devices.byWorkloadId.${workload.workloadId}")
            && workloads.contains("resource.allocatorLeaseId")
            && workloads.contains("lib.unique deviceLeaseIds"),
        "workload process rows must reference only allocator-declared device leases"
    );
}

#[test]
fn rendered_allocator_deduplicates_canonical_device_leases() {
    let Some(dir) = env::var_os("D2B_FIXTURES_FULL").map(PathBuf::from) else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; allocator fixture unavailable");
        return;
    };
    let allocator: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("allocator.json")).expect("read allocator fixture"),
    )
    .expect("parse allocator fixture");
    let requests = allocator["resourceRequests"]
        .as_array()
        .expect("resourceRequests array");
    let device_requests: Vec<_> = requests
        .iter()
        .filter(|request| request["acquisitionOrder"]["phase"] == 50)
        .collect();

    assert_eq!(
        device_requests.len(),
        3,
        "render node and security key leases must be deduplicated across workloads"
    );
    let rows: Vec<_> = device_requests
        .iter()
        .map(|request| {
            (
                request["resourceId"].as_str().unwrap(),
                request["share"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(rows.contains(&("lease-device-render-node-global", "shared-partition")));
    assert!(rows.contains(&("lease-device-security-key-global", "exclusive")));
    assert!(rows.iter().any(
        |(resource, share)| resource.starts_with("lease-device-tpm-") && *share == "exclusive"
    ));
    assert!(device_requests.iter().all(|request| {
        request["kind"] == "host-file-partition"
            && request["source"]["kind"] == "realm-broker"
            && request["source"]["refName"].as_str().is_some()
    }));
    let rendered = serde_json::to_string(&device_requests).unwrap();
    for forbidden in ["busid", "busId", "/dev/bus/usb", "hidraw"] {
        assert!(
            !rendered.contains(forbidden),
            "allocator lease rows must not expose {forbidden:?}"
        );
    }
}

#[test]
fn rendered_device_registry_matches_realm_role_resources() {
    let Some(dir) = env::var_os("D2B_FIXTURES_FULL").map(PathBuf::from) else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; device registry fixture unavailable");
        return;
    };
    let registry: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("provider-registry-v2.json"))
            .expect("read provider registry fixture"),
    )
    .expect("parse provider registry fixture");
    let processes: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("processes.json")).expect("read processes fixture"),
    )
    .expect("parse processes fixture");

    let device_providers: Vec<_> = registry["providers"]
        .as_array()
        .expect("provider array")
        .iter()
        .filter(|provider| provider["binding"]["axis"] == "local-device")
        .collect();
    assert_eq!(
        device_providers.len(),
        1,
        "feature fixture must render one realm-local device provider"
    );
    let provider = device_providers[0];
    let realm_id = provider["descriptor"]["placement"]["realmId"]
        .as_str()
        .expect("device provider realm id");
    let mut resource_ids: Vec<_> = provider["binding"]["deviceResourceIds"]
        .as_array()
        .expect("device resource ids")
        .iter()
        .map(|value| value.as_str().expect("device resource id").to_owned())
        .collect();
    resource_ids.sort();

    let mut expected = Vec::new();
    for dag in processes["vms"].as_array().expect("process DAG array") {
        let identity = &dag["workloadIdentity"];
        if identity["realmId"] != realm_id {
            continue;
        }
        for node in dag["nodes"].as_array().expect("process node array") {
            let kind = match node["role"].as_str().expect("process role") {
                "swtpm" => Some("tpm"),
                "gpu" => Some("gpu"),
                "gpu-render-node" => Some("render-node"),
                "video" => Some("video"),
                "usbip" => Some("usbip"),
                "security-key-frontend" => Some("fido"),
                _ => None,
            };
            let Some(kind) = kind else {
                continue;
            };
            let role_id = node["id"].as_str().expect("device role id");
            expected.push(format!("device-{role_id}-{kind}"));
            assert_eq!(
                node["profile"]["mountPolicy"]["deviceBinds"],
                serde_json::json!([]),
                "device role {role_id} must consume allocator-delivered FDs"
            );
        }
    }
    expected.sort();
    assert_eq!(resource_ids, expected);

    let rendered = serde_json::to_string(provider).unwrap();
    for forbidden in [
        "selectorId",
        "endpointPath",
        "/dev/",
        "busid",
        "busId",
        "hidraw",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "device registry must not expose {forbidden:?}"
        );
    }
}

#[test]
fn usbip_declarative_rows_do_not_expose_physical_or_network_selectors() {
    let devices = read_repo_file("nixos-modules/realm-device-rows.nix");
    let processes = read_repo_file("nixos-modules/processes-json.nix");

    assert!(devices.contains(r#"resourceId = "device-${roleId}-${kind}";"#));
    assert!(devices.contains(r#"allocatorLeaseId = lease.leaseId;"#));
    assert!(devices.contains(r#"attachment = "fd-only";"#));
    assert!(devices.contains(r#"endpointId = endpointIdFor roleId kind;"#));
    for forbidden in [
        "busid",
        "busId",
        "/dev/bus/usb",
        "TCP-LISTEN:3240",
        "bind=0.0.0.0",
        "bind=::",
    ] {
        assert!(
            !devices.contains(forbidden) && !processes.contains(forbidden),
            "realm declarative USBIP surface must not contain {forbidden:?}"
        );
    }
}
