#[path = "../src/package_policy.rs"]
#[allow(dead_code)]
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
Review the scratch preview, then run cargo xtask gen-package-policy-inputs --install.
Run git status --short --untracked-files=all and review and commit only changes below packages/policy-inputs/.
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
    let second = package_policy::package_policy_preview(root).expect("second policy preview");
    assert_eq!(outputs, second);
    assert_eq!(outputs.len(), 16);
    assert!(
        outputs
            .keys()
            .all(|path| path.starts_with("packages/policy-inputs/"))
    );
    let lock_outputs = outputs
        .iter()
        .filter(|(path, _)| path.ends_with("/Cargo.lock"))
        .collect::<Vec<_>>();
    assert_eq!(lock_outputs.len(), 8);
    for (path, contents) in lock_outputs {
        assert!(
            contents.contains("\nversion = 4\n"),
            "{path} has no lock version"
        );
        assert!(
            !package_policy::parse_product_lock(contents)
                .expect("generated policy lock")
                .is_empty(),
            "{path} has no package records"
        );
    }
    let after = fs::read_dir(&policy_root).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<BTreeSet<_>>()
    });
    assert_eq!(before, after);
    let _ = fs::remove_dir_all(root.join(".scratch/bazel/policy-inputs"));
}

#[test]
fn policy_generator_preview_replaces_stale_sidecars_and_returns_written_paths() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let preview_root = root.join(".scratch/bazel/policy-inputs");
    let outputs = package_policy::gen_package_policy_inputs(&[]).expect("policy preview");
    assert_eq!(outputs.len(), 16);
    assert!(
        outputs
            .iter()
            .all(|path| root.join(path).starts_with(&preview_root))
    );
    assert!(outputs.iter().all(|path| root.join(path).is_file()));

    let stale = preview_root.join("obsolete-sidecar.json");
    fs::write(&stale, b"obsolete\n").expect("plant stale preview sidecar");
    let second = package_policy::gen_package_policy_inputs(&[]).expect("replaced preview");
    assert_eq!(outputs, second);
    assert!(!stale.exists());
    assert!(second.iter().all(|path| root.join(path).is_file()));

    let checked =
        package_policy::gen_package_policy_inputs(&["--check".to_owned()]).expect("policy check");
    assert_eq!(
        checked,
        package_policy::package_policy_preview(root)
            .expect("policy outputs")
            .keys()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    );
    let _ = fs::remove_dir_all(preview_root);
}

#[test]
fn policy_check_requires_an_exact_nonempty_regular_census() {
    let root = std::env::temp_dir().join(format!("d2b-policy-census-{}", std::process::id()));
    let output_root = root.join("packages/policy-inputs");
    fs::create_dir_all(output_root.join("context")).expect("policy root");
    let expected = std::collections::BTreeMap::from([(
        "packages/policy-inputs/context/Cargo.lock".to_owned(),
        "lock\n".to_owned(),
    )]);
    let output = output_root.join("context/Cargo.lock");
    fs::write(&output, b"lock\n").expect("policy output");
    package_policy::check_policy_outputs(&root, &expected).expect("exact policy census");

    fs::remove_file(&output).expect("remove expected output");
    let missing = package_policy::check_policy_outputs(&root, &expected)
        .expect_err("missing output must fail closed");
    assert!(missing.to_string().contains("Missing paths"));

    fs::write(&output, b"lock\n").expect("restore policy output");
    fs::write(output_root.join("stale.json"), b"stale\n").expect("extra output");
    let extra = package_policy::check_policy_outputs(&root, &expected)
        .expect_err("extra output must fail closed");
    assert!(extra.to_string().contains("Extra paths"));

    fs::remove_file(output_root.join("stale.json")).expect("remove extra output");
    fs::remove_file(&output).expect("remove output for nonregular mutation");
    fs::create_dir(&output).expect("replace output with directory");
    assert!(
        package_policy::check_policy_outputs(&root, &expected).is_err(),
        "nonregular output must fail closed"
    );

    fs::remove_dir_all(&root).expect("remove policy census root");
    assert!(
        package_policy::check_policy_outputs(&root, &expected).is_err(),
        "absent policy root must fail closed"
    );
}

#[test]
fn all_committed_policy_locks_parse() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let mut paths = Vec::new();
    for system in ["x86_64-linux", "aarch64-linux"] {
        let (gnu, musl) = if system == "x86_64-linux" {
            ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl")
        } else {
            ("aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl")
        };
        for (target, context) in [(gnu, "broker-production"), (musl, "guest-real-libshpool")] {
            for variant in ["production", "policy"] {
                paths.push(format!(
                    "packages/policy-inputs/{system}/{target}/{context}/{variant}/Cargo.lock"
                ));
            }
        }
    }
    assert_eq!(paths.len(), 8);
    for path in paths {
        let lock = fs::read_to_string(root.join(&path)).expect("committed policy lock");
        assert!(
            !package_policy::parse_product_lock(&lock)
                .expect("committed policy lock parses")
                .is_empty(),
            "{path} has no package records"
        );
    }
}

fn synthetic_lock(packages: &str) -> String {
    format!("version = 4\n\n{packages}")
}

fn synthetic_package(
    name: &str,
    version: &str,
    source: Option<&str>,
    dependencies: &[&str],
) -> String {
    let mut package = format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n");
    if let Some(source) = source {
        package.push_str(&format!("source = \"{source}\"\n"));
        package.push_str("checksum = \"");
        package.push_str(&"a".repeat(64));
        package.push_str("\"\n");
    }
    if !dependencies.is_empty() {
        package.push_str("dependencies = [\n");
        for dependency in dependencies {
            package.push_str(&format!(" \"{dependency}\",\n"));
        }
        package.push_str("]\n");
    }
    package.push('\n');
    package
}

#[test]
fn selected_lock_prunes_removed_edges_and_retains_selected_edges() {
    let lock = synthetic_lock(&format!(
        "{}{}{}",
        synthetic_package("root", "1.0.0", None, &["kept", "removed"]),
        synthetic_package(
            "kept",
            "1.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
        synthetic_package(
            "removed",
            "1.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
    ));
    let selected = BTreeSet::from([
        package_policy::PackageIdentity::new("root", "1.0.0", None),
        package_policy::PackageIdentity::new(
            "kept",
            "1.0.0",
            Some("registry+https://example.invalid/index".to_owned()),
        ),
    ]);
    let rendered =
        package_policy::filtered_lock_text(&lock, &selected).expect("filtered lock renders");
    let packages = package_policy::parse_product_lock(&rendered).expect("rendered lock parses");
    assert_eq!(
        packages
            .iter()
            .map(|package| package.identity.clone())
            .collect::<BTreeSet<_>>(),
        selected
    );
    assert_eq!(
        packages
            .iter()
            .find(|package| package.identity.name == "kept")
            .expect("kept package")
            .dependencies,
        Vec::<String>::new()
    );
    let root = packages
        .iter()
        .find(|package| package.identity.name == "root")
        .expect("root package");
    assert_eq!(root.dependencies, vec!["kept"]);
    assert!(!rendered.contains("\"removed\""));
    assert!(rendered.contains(&format!("checksum = \"{}\"", "a".repeat(64))));
}

#[test]
fn selected_lock_resolves_same_name_versions_and_sources() {
    let lock = synthetic_lock(&format!(
        "{}{}{}{}",
        synthetic_package(
            "root",
            "1.0.0",
            None,
            &[
                "same 1.0.0 (registry+https://example.invalid/index)",
                "same 2.0.0",
                "same 1.0.0 (registry+https://example.invalid/alternate)",
            ],
        ),
        synthetic_package(
            "same",
            "1.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
        synthetic_package(
            "same",
            "1.0.0",
            Some("registry+https://example.invalid/alternate"),
            &[],
        ),
        synthetic_package(
            "same",
            "2.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
    ));
    let selected = BTreeSet::from([
        package_policy::PackageIdentity::new("root", "1.0.0", None),
        package_policy::PackageIdentity::new(
            "same",
            "1.0.0",
            Some("registry+https://example.invalid/alternate".to_owned()),
        ),
        package_policy::PackageIdentity::new(
            "same",
            "2.0.0",
            Some("registry+https://example.invalid/index".to_owned()),
        ),
    ]);
    let rendered =
        package_policy::filtered_lock_text(&lock, &selected).expect("filtered lock renders");
    let packages = package_policy::parse_product_lock(&rendered).expect("rendered lock parses");
    let root = packages
        .iter()
        .find(|package| package.identity.name == "root")
        .expect("root package");
    assert_eq!(
        root.dependencies,
        vec![
            "same 2.0.0",
            "same 1.0.0 (registry+https://example.invalid/alternate)"
        ]
    );

    let ambiguous = synthetic_lock(&format!(
        "{}{}{}",
        synthetic_package("root", "1.0.0", None, &["same"]),
        synthetic_package(
            "same",
            "1.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
        synthetic_package(
            "same",
            "2.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
    ));
    assert!(matches!(
        package_policy::filtered_lock_text(
            &ambiguous,
            &BTreeSet::from([
                package_policy::PackageIdentity::new("root", "1.0.0", None),
                package_policy::PackageIdentity::new(
                    "same",
                    "1.0.0",
                    Some("registry+https://example.invalid/index".to_owned()),
                ),
            ])
        ),
        Err(package_policy::PolicyError::AmbiguousLockDependency(_))
    ));
}

#[test]
fn selected_lock_refuses_malformed_dependency_tokens() {
    let lock = synthetic_lock(&format!(
        "{}{}",
        synthetic_package("root", "1.0.0", None, &["kept 1.0.0 extra"]),
        synthetic_package(
            "kept",
            "1.0.0",
            Some("registry+https://example.invalid/index"),
            &[],
        ),
    ));
    let selected = BTreeSet::from([
        package_policy::PackageIdentity::new("root", "1.0.0", None),
        package_policy::PackageIdentity::new(
            "kept",
            "1.0.0",
            Some("registry+https://example.invalid/index".to_owned()),
        ),
    ]);
    assert!(matches!(
        package_policy::filtered_lock_text(&lock, &selected),
        Err(package_policy::PolicyError::MalformedLockDependency(_))
    ));
}

#[test]
fn selected_lock_refuses_dangling_dependency_tokens() {
    let lock = synthetic_lock(&synthetic_package("root", "1.0.0", None, &["missing"]));
    let selected = BTreeSet::from([package_policy::PackageIdentity::new("root", "1.0.0", None)]);
    assert!(matches!(
        package_policy::filtered_lock_text(&lock, &selected),
        Err(package_policy::PolicyError::UnresolvableLockDependency(_))
    ));
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
        Err(package_policy::PolicyError::UnrecognizedAbsolutePath)
    ));
}
