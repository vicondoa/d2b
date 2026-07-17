use d2b_contract_tests::read_repo_file;
use std::{env, fs, path::PathBuf};

#[test]
fn usbip_uses_exclusive_realm_allocator_lease() {
    let devices = read_repo_file("nixos-modules/realm-device-rows.nix");
    let workloads = read_repo_file("nixos-modules/workload-process-rows.nix");

    for required in [
        r#"[ "usbip" "fido" ]"#,
        r#"resourceId = "device-security-key-global";"#,
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
        workloads.contains("cfg._index.devices.allocatorLeaseRequests")
            && workloads.contains(r#"lib.hasPrefix "device-${workload.workloadId}-""#),
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
    assert!(rows.contains(&("device-render-node-global", "shared-partition")));
    assert!(rows.contains(&("device-security-key-global", "exclusive")));
    assert!(rows
        .iter()
        .any(|(resource, share)| resource.starts_with("device-tpm-") && *share == "exclusive"));
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
fn usbip_declarative_rows_do_not_expose_physical_or_network_selectors() {
    let devices = read_repo_file("nixos-modules/realm-device-rows.nix");
    let processes = read_repo_file("nixos-modules/processes-json.nix");

    assert!(devices.contains(r#"selectorId = "selector-${workload.workloadId}-${kind}";"#));
    assert!(devices.contains(r#"attachment = "fd-only";"#));
    assert!(devices.contains(r#"endpointPath = endpointFor workload roleId kind;"#));
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
