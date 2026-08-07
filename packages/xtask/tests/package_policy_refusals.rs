#[path = "../src/package_policy.rs"]
mod package_policy;

use std::{collections::BTreeSet, fs, path::PathBuf};

use serde_json::{Value, json};

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
    let policy_root = root.join("packages/policy-inputs");
    let before = fs::read_dir(&policy_root).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<BTreeSet<_>>()
    });
    let outputs = package_policy::package_policy_preview(root).expect("policy preview");
    assert_eq!(outputs.len(), 16);
    assert!(
        outputs
            .keys()
            .all(|path| path.starts_with("packages/policy-inputs/"))
    );
    let after = fs::read_dir(&policy_root).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<BTreeSet<_>>()
    });
    assert_eq!(before, after);
    let _ = fs::remove_dir_all(root.join(".scratch/bazel/policy-inputs"));
}

fn metadata_fixture(
    root: &std::path::Path,
) -> (
    String,
    package_policy::SelectedContext,
    BTreeSet<package_policy::PackageIdentity>,
) {
    let context = package_policy::SelectedContext::for_system(
        "x86_64-linux",
        package_policy::PolicyContext::BrokerProduction,
    )
    .expect("context");
    let workspace_root = root.join("packages");
    let root_id = format!(
        "path+file://{}#{}@1.0.0",
        workspace_root.join(&context.package).display(),
        context.package
    );
    let dependency_id =
        "registry+https://github.com/rust-lang/crates.io-index#fixture-dependency@1.0.0";
    let extra_id = "registry+https://github.com/rust-lang/crates.io-index#fixture-extra@1.0.0";
    let package =
        |name: &str, id: &str, source: Option<&str>, manifest_path: &str, src_path: &str| {
            json!({
                "name": name,
                "version": "1.0.0",
                "id": id,
                "license": "MIT",
                "license_file": null,
                "source": source,
                "manifest_path": manifest_path,
                "targets": [{
                    "crate_types": ["lib"],
                    "doc": true,
                    "doctest": true,
                    "edition": "2024",
                    "kind": ["lib"],
                    "name": name,
                    "required-features": null,
                    "src_path": src_path,
                    "test": true
                }],
                "dependencies": [],
                "features": {},
                "authors": [],
                "categories": [],
                "description": null,
                "documentation": null,
                "edition": "2024",
                "homepage": null,
                "keywords": [],
                "links": null,
                "metadata": {},
                "publish": null,
                "readme": null,
                "repository": null,
                "rust_version": null,
                "default_run": null
            })
        };
    let workspace_manifest = workspace_root.join(&context.package).join("Cargo.toml");
    let workspace_src = workspace_root.join(&context.package).join("src/lib.rs");
    let registry_manifest =
        "/home/fixture/.cargo/registry/src/index/fixture-dependency-1.0.0/Cargo.toml";
    let registry_src =
        "/home/fixture/.cargo/registry/src/index/fixture-dependency-1.0.0/src/lib.rs";
    let extra_manifest = "/home/fixture/.cargo/registry/src/index/fixture-extra-1.0.0/Cargo.toml";
    let extra_src = "/home/fixture/.cargo/registry/src/index/fixture-extra-1.0.0/src/lib.rs";
    let metadata = json!({
        "build_directory": workspace_root.join("target"),
        "metadata": null,
        "packages": [
            package(
                &context.package,
                &root_id,
                None,
                &workspace_manifest.to_string_lossy(),
                &workspace_src.to_string_lossy(),
            ),
            package(
                "fixture-dependency",
                dependency_id,
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                registry_manifest,
                registry_src,
            ),
            package(
                "fixture-extra",
                extra_id,
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                extra_manifest,
                extra_src,
            )
        ],
        "resolve": {
            "nodes": [
                {
                    "id": root_id,
                    "dependencies": [dependency_id, extra_id],
                    "deps": [
                        {
                            "name": "fixture-dependency",
                            "pkg": dependency_id,
                            "dep_kinds": [{"kind": null, "target": null}]
                        },
                        {
                            "name": "fixture-extra",
                            "pkg": extra_id,
                            "dep_kinds": [{"kind": "dev", "target": null}]
                        }
                    ],
                    "features": ["root-feature"]
                },
                {
                    "id": dependency_id,
                    "dependencies": [],
                    "deps": [],
                    "features": []
                },
                {
                    "id": extra_id,
                    "dependencies": [],
                    "deps": [],
                    "features": []
                }
            ],
            "root": null
        },
        "target_directory": workspace_root.join("target"),
        "version": 1,
        "workspace_default_members": [root_id],
        "workspace_members": [root_id],
        "workspace_root": workspace_root
    });
    let selected = BTreeSet::from([
        package_policy::PackageIdentity::new(context.package.clone(), "1.0.0", None),
        package_policy::PackageIdentity::new(
            "fixture-dependency",
            "1.0.0",
            Some("registry+https://github.com/rust-lang/crates.io-index".to_owned()),
        ),
    ]);
    (
        serde_json::to_string(&metadata).expect("fixture metadata"),
        context,
        selected,
    )
}

#[test]
fn selected_metadata_keeps_cargo_deny_fields_and_context_fields() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let (document, context, selected) = metadata_fixture(root);
    let metadata = package_policy::filter_selected_metadata(&document, &selected, &context, root)
        .expect("selected metadata");
    assert_eq!(metadata["version"], 1);
    assert_eq!(metadata["packages"].as_array().expect("packages").len(), 2);
    assert_eq!(
        metadata["resolve"]["nodes"]
            .as_array()
            .expect("resolve nodes")
            .len(),
        2
    );
    for field in [
        "id",
        "name",
        "version",
        "source",
        "license",
        "manifest_path",
        "targets",
        "features",
        "dependencies",
    ] {
        assert!(
            metadata["packages"][0].get(field).is_some(),
            "Cargo package field {field} is required"
        );
    }
    let input = package_policy::PolicyInput {
        context: context.clone(),
        variant: "policy",
        edge_kinds: context.policy_edges().to_owned(),
        root: context.package.clone(),
        source_census_digest: "0".repeat(64),
        identities: selected.iter().cloned().collect(),
    };
    let rendered = input
        .as_selected_metadata_json(&document, root)
        .expect("metadata with policy context");
    let rendered: Value = serde_json::from_str(&rendered).expect("rendered metadata");
    for field in [
        "packages",
        "resolve",
        "workspace_members",
        "workspace_default_members",
        "workspace_root",
        "target_directory",
        "system",
        "target",
        "package",
        "features",
        "defaultFeatures",
        "variant",
        "edgeKinds",
        "sourceCensusSha256",
        "identities",
        "root",
    ] {
        assert!(
            rendered.get(field).is_some(),
            "metadata field {field} is present"
        );
    }
    assert_eq!(rendered["variant"], "policy");
    assert_eq!(
        rendered["resolve"]["root"],
        "path+file:///workspace/packages/d2b-priv-broker#d2b-priv-broker@1.0.0"
    );
}

#[test]
fn selected_metadata_rejects_missing_and_extra_selected_identities() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let (document, context, mut selected) = metadata_fixture(root);
    selected.insert(package_policy::PackageIdentity::new(
        "missing",
        "1.0.0",
        Some("registry+https://github.com/rust-lang/crates.io-index".to_owned()),
    ));
    assert!(matches!(
        package_policy::filter_selected_metadata(&document, &selected, &context, root),
        Err(package_policy::PolicyError::SelectedPackageMissing(_))
    ));

    let actual = BTreeSet::from([
        package_policy::PackageIdentity::new(context.package, "1.0.0", None),
        package_policy::PackageIdentity::new("extra", "1.0.0", None),
    ]);
    let expected = BTreeSet::from([package_policy::PackageIdentity::new(
        "d2b-priv-broker",
        "1.0.0",
        None,
    )]);
    assert!(matches!(
        package_policy::validate_selected_identity_set(&expected, &actual),
        Err(package_policy::PolicyError::SelectedPackageExtra(_))
    ));
}

#[test]
fn selected_metadata_rejects_dangling_edges_and_wrong_root() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let (document, context, selected) = metadata_fixture(root);
    let mut dangling: Value = serde_json::from_str(&document).expect("fixture JSON");
    dangling["resolve"]["nodes"][0]["deps"][0]["pkg"] =
        Value::String("registry+https://example.invalid#missing@1.0.0".to_owned());
    assert!(matches!(
        package_policy::filter_selected_metadata(
            &serde_json::to_string(&dangling).expect("dangling JSON"),
            &selected,
            &context,
            root,
        ),
        Err(package_policy::PolicyError::DanglingResolveEdge(_))
    ));

    let mut wrong_root: Value = serde_json::from_str(&document).expect("fixture JSON");
    wrong_root["resolve"]["root"] = Value::String(
        "registry+https://github.com/rust-lang/crates.io-index#fixture-dependency@1.0.0".to_owned(),
    );
    assert!(matches!(
        package_policy::filter_selected_metadata(
            &serde_json::to_string(&wrong_root).expect("wrong-root JSON"),
            &selected,
            &context,
            root,
        ),
        Err(package_policy::PolicyError::MetadataRootMismatch(_))
    ));
}

#[test]
fn selected_metadata_normalizes_paths_rejects_unknown_absolute_paths_and_is_deterministic() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let (document, context, selected) = metadata_fixture(root);
    let first = package_policy::filter_selected_metadata(&document, &selected, &context, root)
        .expect("first metadata");
    let second = package_policy::filter_selected_metadata(&document, &selected, &context, root)
        .expect("second metadata");
    assert_eq!(
        serde_json::to_string(&first).expect("first JSON"),
        serde_json::to_string(&second).expect("second JSON")
    );
    let rendered = serde_json::to_string(&first).expect("normalized JSON");
    assert!(!rendered.contains(&root.to_string_lossy().to_string()));
    assert!(!rendered.contains("/nix/store/"));
    assert!(rendered.contains("path+file:///workspace/packages/d2b-priv-broker"));
    assert!(rendered.contains("/cargo/registry/src/index/fixture-dependency-1.0.0"));

    let mut unknown_path: Value = serde_json::from_str(&document).expect("fixture JSON");
    unknown_path["workspace_root"] = Value::String("/unrecognized/absolute/path".to_owned());
    assert!(matches!(
        package_policy::filter_selected_metadata(
            &serde_json::to_string(&unknown_path).expect("unknown-path JSON"),
            &selected,
            &context,
            root,
        ),
        Err(package_policy::PolicyError::UnrecognizedAbsolutePath(_))
    ));
}
