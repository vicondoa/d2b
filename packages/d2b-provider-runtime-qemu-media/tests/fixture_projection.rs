use std::fs;
use std::path::Path;

use serde_json::Value;

fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Array(values) => values.iter().any(|value| contains_key(value, key)),
        Value::Object(fields) => {
            fields.contains_key(key) || fields.values().any(|value| contains_key(value, key))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

#[test]
fn rendered_zone_processes_keep_provider_owned_locator_free_shape() {
    let root = std::env::var_os("D2B_FIXTURES")
        .expect("D2B_FIXTURES must point at the enforcing fixture tree");
    let zones = Path::new(&root).join("zones");

    for zone in fs::read_dir(&zones).expect("rendered fixture zones") {
        let zone = zone.expect("rendered fixture zone entry");
        let bundle_path = zone.path().join("resource-bundle.json");
        if !bundle_path.is_file() {
            continue;
        }
        let bytes = fs::read(&bundle_path).expect("rendered Zone resource bundle");
        let bundle: Value = serde_json::from_slice(&bytes).expect("valid resource bundle JSON");
        let resources = bundle["resources"]
            .as_array()
            .expect("resource bundle resources array");

        for resource in resources {
            if resource["type"] != "Process" && resource["type"] != "EphemeralProcess" {
                continue;
            }
            let provider = resource["spec"]["providerRef"]
                .as_str()
                .expect("projected process providerRef");
            assert!(provider.starts_with("Provider/"));
            for forbidden in [
                "argv",
                "binaryPath",
                "commandLine",
                "environment",
                "env",
                "path",
                "mountPath",
                "hostPath",
                "socketPath",
                "devicePath",
                "numericUid",
                "numericGid",
            ] {
                assert!(
                    !contains_key(resource, forbidden),
                    "projected process contains forbidden field {forbidden}"
                );
            }
        }
    }
}
