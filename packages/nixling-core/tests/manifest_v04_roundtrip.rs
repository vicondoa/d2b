mod manifest_v04_roundtrip {
    use nixling_core::manifest_v04::ManifestV04;
    use serde_json::Value;
    use std::path::PathBuf;

    const BASELINE_FIXTURE: &str = "../../tests/golden/manifest_v04/baseline-vms.json";
    const REQUIRED_NETWORKING_PATHS: &[&[&str]] = &[
        &["corp-vm", "mtu"],
        &["corp-vm", "mssClamp"],
        &["corp-vm", "lan", "allowEastWest"],
        &["corp-vm", "lan", "effectiveEastWest"],
        &["sys-work-net", "mtu"],
        &["sys-work-net", "mssClamp"],
        &["sys-work-net", "lan", "allowEastWest"],
        &["sys-work-net", "lan", "effectiveEastWest"],
    ];

    #[test]
    fn baseline_vms_json_round_trips_byte_identically() {
        let baseline_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BASELINE_FIXTURE);
        let baseline_bytes =
            std::fs::read(&baseline_path).expect("read manifest v04 baseline fixture");

        let manifest = ManifestV04::from_path(&baseline_path).expect("baseline fixture parses");
        let rendered = manifest
            .to_compact_json()
            .expect("baseline fixture serializes");

        assert_eq!(
            rendered.as_bytes(),
            baseline_bytes.as_slice(),
            "manifest-v04-roundtrip: rendered manifest differs from baseline"
        );

        let baseline_json: Value =
            serde_json::from_slice(&baseline_bytes).expect("baseline fixture JSON parses");
        let rendered_json: Value =
            serde_json::from_str(&rendered).expect("rendered manifest JSON parses");

        for path in REQUIRED_NETWORKING_PATHS {
            let baseline_value = scalar_at_path(&baseline_json, path, "canonical baseline");
            let rendered_value = scalar_at_path(&rendered_json, path, "rendered manifest");
            assert_eq!(
                rendered_value,
                baseline_value,
                "manifest-v04-roundtrip: networking field at path {} changed from {} to {}",
                path_json(path),
                baseline_value,
                rendered_value
            );
        }
    }

    fn scalar_at_path<'a>(value: &'a Value, path: &[&str], label: &str) -> &'a Value {
        let found = path
            .iter()
            .try_fold(value, |current, segment| match current {
                Value::Object(map) => map.get(*segment),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "manifest-v04-roundtrip: {label} is missing required networking path {}",
                    path_json(path)
                )
            });
        assert!(
            !matches!(found, Value::Array(_) | Value::Object(_)),
            "manifest-v04-roundtrip: {label} has non-scalar networking path {}",
            path_json(path)
        );
        found
    }

    fn path_json(path: &[&str]) -> String {
        serde_json::to_string(path).expect("networking path serializes")
    }
}
