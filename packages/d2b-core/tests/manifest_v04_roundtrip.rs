mod manifest_v04_roundtrip {
    use d2b_core::manifest_v04::ManifestV04;
    use serde_json::Value;
    use std::path::PathBuf;

    const BASELINE_FIXTURE: &str = "../../tests/golden/manifest_v04/baseline-vms.json";

    #[test]
    fn baseline_vms_json_round_trips_semantically() {
        let baseline_path = runfile_path(BASELINE_FIXTURE);
        let baseline_bytes =
            std::fs::read(&baseline_path).expect("read manifest v04 baseline fixture");

        let manifest = ManifestV04::from_path(&baseline_path).expect("baseline fixture parses");
        let rendered = manifest
            .to_compact_json()
            .expect("baseline fixture serializes");

        let baseline_json: Value =
            serde_json::from_slice(&baseline_bytes).expect("baseline fixture JSON parses");
        let rendered_json: Value =
            serde_json::from_str(&rendered).expect("rendered manifest JSON parses");
        assert_eq!(
            rendered_json, baseline_json,
            "manifest-v04-roundtrip: rendered manifest differs from baseline"
        );

        fn runfile_path(relative: &str) -> PathBuf {
            if let Some(runfiles) = std::env::var_os("RUNFILES_DIR") {
                let candidate = PathBuf::from(runfiles)
                    .join("_main")
                    .join("tests/golden/manifest_v04/baseline-vms.json");
                if candidate.exists() {
                    return candidate;
                }
            }
            std::env::current_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join(relative)
        }
    }

}
