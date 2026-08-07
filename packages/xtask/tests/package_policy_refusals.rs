#[path = "../src/package_policy.rs"]
mod package_policy;

use std::{collections::BTreeSet, fs, path::PathBuf};

#[test]
fn selected_commands_pin_lock_offline_and_parser_format() {
    let context = package_policy::SelectedContext::for_system(
        "x86_64-linux",
        package_policy::PolicyContext::GuestProduction,
    )
    .expect("context");
    let command = package_policy::policy_tree_command(&context);
    package_policy::validate_tree_command(&command).expect("pinned tree command");
    assert!(
        command
            .windows(2)
            .any(|pair| pair == ["--locked", "--offline"])
    );
    assert!(
        command
            .windows(2)
            .any(|pair| pair == ["--format", "|{p}|{f}|"])
    );
    assert!(
        command
            .windows(2)
            .any(|pair| pair == ["--edges", "normal,build,dev"])
    );
}

#[test]
fn metadata_does_not_supply_checksums_and_filtered_lock_does() {
    let metadata = r#"{
      "packages": [{
        "id": "registry+https://example.invalid#example@1.0.0",
        "name": "example",
        "version": "1.0.0",
        "source": "registry+https://example.invalid/index"
      }]
    }"#;
    let view = package_policy::parse_metadata(metadata).expect("metadata");
    let identity = view.packages[0].clone();
    let selected = BTreeSet::from([identity.clone()]);
    let missing = vec![package_policy::LockPackage {
        identity: identity.clone(),
        checksum: None,
        dependencies: Vec::new(),
    }];
    assert!(matches!(
        package_policy::selected_source_census(&selected, &missing, &BTreeSet::new()),
        Err(package_policy::PolicyError::ChecksumMissing(_))
    ));
    let lock = vec![package_policy::LockPackage {
        identity,
        checksum: Some("a".repeat(64)),
        dependencies: Vec::new(),
    }];
    assert!(package_policy::selected_source_census(&selected, &lock, &BTreeSet::new()).is_ok());
}

#[test]
fn dev_edges_are_not_post_filtered_and_feature_union_is_refused() {
    let mut command = package_policy::policy_tree_command(
        &package_policy::SelectedContext::for_system(
            "aarch64-linux",
            package_policy::PolicyContext::BrokerProduction,
        )
        .unwrap(),
    );
    let index = command
        .iter()
        .position(|value| value == "normal,build,dev")
        .unwrap();
    command[index] = "normal,build".to_owned();
    assert!(matches!(
        package_policy::validate_tree_command_for(&command, "normal,build,dev"),
        Err(package_policy::PolicyError::InvalidEdgeKinds(_))
    ));

    let row = package_policy::TreeRow {
        depth: 1,
        package: "canary".to_owned(),
        features: BTreeSet::from(["unrelated-feature".to_owned()]),
    };
    assert!(matches!(
        package_policy::feature_union_refusal(
            &[row],
            &BTreeSet::from(["unrelated-feature".to_owned()]),
            &BTreeSet::new(),
        ),
        Err(package_policy::PolicyError::FeatureUnionLeak(_))
    ));
}

#[test]
fn policy_remediation_is_the_exact_repository_sequence() {
    assert_eq!(
        package_policy::package_policy_drift_message(),
        "\
D2B-BZLDRIFT-PACKAGE-POLICY: package-policy output is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command."
    );
}

#[test]
fn policy_preview_uses_locked_offline_commands_and_stays_in_scratch() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let outputs = package_policy::package_policy_preview(root).expect("policy preview");
    assert_eq!(outputs.len(), 16);
    assert!(
        outputs
            .keys()
            .all(|path| path.starts_with("packages/policy-inputs/"))
    );
    assert!(!root.join("packages/policy-inputs").exists());
    let _ = fs::remove_dir_all(root.join(".scratch/bazel/policy-inputs"));
}
