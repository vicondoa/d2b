#[path = "../src/bazel.rs"]
mod bazel;
#[path = "../src/bazel_yanked.rs"]
mod bazel_yanked;
#[path = "../src/hermeticity.rs"]
mod hermeticity;
#[path = "../src/package_policy.rs"]
mod package_policy;

use std::{fs, path::PathBuf};

#[test]
fn module_declares_exactly_the_two_accepted_hubs() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let module = fs::read_to_string(root.join("MODULE.bazel")).expect("module");
    assert_eq!(module.matches("name = \"product\"").count(), 1);
    assert_eq!(module.matches("name = \"walker\"").count(), 1);
    assert_eq!(
        module
            .matches("skip_cargo_lockfile_overwrite = True")
            .count(),
        2
    );
    assert_eq!(module.matches("cargo_lockfile =").count(), 2);
    assert_eq!(module.matches("\n    lockfile =").count(), 2);
    assert!(!module.contains("crate.spec"));
    assert!(!module.contains("Cargo.guest.lock"));
    assert!(module.contains("manifests = [\"//:packages/Cargo.toml\"]"));
    assert!(module.contains("cargo_lockfile = \"//:packages/Cargo.lock\""));
    assert!(module.contains("manifests = [\"//:tests/tools/no-bash-ast-walker/Cargo.toml\"]"));
    assert!(module.contains("cargo_lockfile = \"//:tests/tools/no-bash-ast-walker/Cargo.lock\""));
    assert!(!module.contains("//packages:"));
    assert!(!module.contains("//tests/tools/no-bash-ast-walker:"));
    let root_build = fs::read_to_string(root.join("BUILD.bazel")).expect("root BUILD");
    assert!(root_build.contains("\"packages/Cargo.toml\""));
    assert!(root_build.contains("\"packages/Cargo.lock\""));
    assert!(root_build.contains("\"tests/tools/no-bash-ast-walker/Cargo.toml\""));
    assert!(root_build.contains("\"tests/tools/no-bash-ast-walker/Cargo.lock\""));
    assert!(!root.join("packages").join("BUILD.bazel").exists());
    assert!(
        !root
            .join("tests/tools/no-bash-ast-walker")
            .join("BUILD.bazel")
            .exists()
    );
    for retired in ["main", "broker", "guest"] {
        let error = bazel::parse_repin(&["--hub".into(), retired.into()])
            .expect_err("retired hub must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "Hub '{retired}' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product"
            )
        );
        let (_, argv, cwd) = bazel::retired_hub_remediation(retired).expect("remediation");
        bazel::validate_retired_hub_remediation(&argv, cwd).expect("closed remediation");
        assert!(!argv.iter().any(|argument| argument == "cd"));
    }
}

#[test]
fn toolchain_and_action_network_pins_are_closed() {
    assert_eq!(
        bazel::parse_repin(&["--hub".into(), "product".into()]).unwrap(),
        "product"
    );
    assert_eq!(
        bazel::parse_repin(&["--hub".into(), "walker".into()]).unwrap(),
        "walker"
    );
    let inventory = hermeticity::complete_action_network_inventory();
    hermeticity::validate_action_network_inventory(&inventory).expect("complete inventory");
    assert_eq!(inventory.action_network, "none");
    assert_eq!(
        inventory.strategy_inventory.len(),
        hermeticity::GOVERNED_ACTION_KINDS.len()
    );
    assert_eq!(inventory.socket_plants.len(), 8);
    assert_eq!(inventory.inherited_descriptor_plants.len(), 4);
}

#[test]
fn adr0054_drift_table_is_closed_and_redacted() {
    let codes = [
        "D2B-CARGODRIFT-PRODUCT",
        "D2B-CARGODRIFT-WALKER",
        "D2B-BZLDRIFT-PRODUCT-HUB",
        "D2B-BZLDRIFT-WALKER-HUB",
        "D2B-BZLDRIFT-MODULE",
        "D2B-BZLDRIFT-GENERATOR",
        "D2B-BZLDRIFT-PACKAGE-POLICY",
        "D2B-BZLDRIFT-YANKED",
        "D2B-BZL-AMBIENT-REPIN",
        "D2B-BZL-UNEXPECTED-MUTATION",
    ];
    for code in codes {
        let message = bazel::adr0054_drift_message(code).expect("closed diagnostic");
        assert!(message.starts_with(code));
        assert!(message.contains("From the repository root, run: nix develop"));
        assert!(message.contains("Then run: cd packages"));
        assert!(!message.contains("/home/"));
        assert!(!message.contains("private"));
        assert!(!message.contains("$!"));
    }
    assert!(bazel::adr0054_drift_message("D2B-UNKNOWN").is_none());
}

#[test]
fn policy_contexts_are_native_and_have_no_guest_lock_authority() {
    let contexts = package_policy::policy_contexts().expect("contexts");
    assert_eq!(contexts.len(), 4);
    assert!(contexts.iter().all(|context| context.package != "d2b"));
    assert!(
        contexts
            .iter()
            .all(|context| context.target.contains("unknown-linux"))
    );
    assert!(!package_policy::committed_git_archive_pins().is_empty());
}

#[test]
fn yanked_refresh_is_product_only() {
    let source = include_str!("../src/bazel_yanked.rs");
    assert!(source.contains("\"packages/Cargo.lock\""));
    assert!(!source.contains("Cargo.guest.lock"));
    assert!(!source.contains("no-bash-ast-walker/Cargo.lock"));
}

#[test]
fn generator_preview_is_the_only_generation_side_effect() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repository root");
    let before = fs::read_dir(root.join(".scratch")).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<std::collections::BTreeSet<_>>()
    });
    let outputs = bazel::gen_bazel(&[]).expect("generator preview");
    let approved = [
        ".bazelignore",
        "bazel/generated/BUILD.bazel",
        "bazel/generated/action-network-policy.json",
        "bazel/generated/configured-targets.json",
        "bazel/generated/evidence-sink-policy.json",
        "bazel/generated/no-shell-inventory.json",
        "bazel/generated/output-manifest.json",
        "bazel/generated/package-policy-targets.bzl",
        "bazel/generated/product-targets.bzl",
        "bazel/generated/source-census.json",
    ];
    assert_eq!(outputs.len(), approved.len());
    let preview_root = PathBuf::from(".scratch/bazel/generated-preview");
    let actual = outputs
        .iter()
        .map(|path| {
            path.strip_prefix(&preview_root)
                .expect("preview path is rooted in the scratch census")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, approved);
    assert_eq!(
        outputs
            .iter()
            .filter(|path| fs::metadata(root.join(path)).is_ok_and(|metadata| metadata.is_file()))
            .count(),
        approved.len()
    );
    let no_shell: serde_json::Value = serde_json::from_slice(
        &fs::read(
            root.join(&preview_root)
                .join("bazel/generated/no-shell-inventory.json"),
        )
        .expect("no-shell inventory"),
    )
    .expect("no-shell JSON");
    let governed = no_shell["governedSources"]
        .as_array()
        .expect("governed source set");
    let declared = no_shell["declaredInputs"]
        .as_array()
        .expect("declared input set");
    assert!(!governed.is_empty());
    assert_eq!(governed, declared);
    assert_eq!(
        no_shell["scanResults"]
            .as_array()
            .expect("scan results")
            .len(),
        governed.len()
    );
    assert_eq!(
        no_shell["spawnSites"]
            .as_array()
            .expect("spawn sites")
            .iter()
            .filter(|site| site["shellInvocation"].as_bool() == Some(true))
            .count(),
        0
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            root.join(&preview_root)
                .join("bazel/generated/output-manifest.json"),
        )
        .expect("output manifest"),
    )
    .expect("output manifest JSON");
    assert_eq!(manifest["outputCount"], 10);
    assert!(manifest["selfDigest"].is_null());
    assert_eq!(
        manifest["outputs"]
            .as_array()
            .expect("manifest digests")
            .len(),
        9
    );
    assert!(
        fs::read_to_string(
            root.join(&preview_root)
                .join("bazel/generated/product-targets.bzl")
        )
        .expect("product targets")
        .contains("PRODUCT_TARGETS")
    );
    assert!(
        fs::read_to_string(
            root.join(&preview_root)
                .join("bazel/generated/package-policy-targets.bzl")
        )
        .expect("package policy targets")
        .contains("PACKAGE_POLICY_TARGETS")
    );
    assert!(outputs.iter().all(|path| path.starts_with(".scratch")));
    let nested_workspace_build = PathBuf::from("packages").join("BUILD.bazel");
    assert!(
        outputs
            .iter()
            .all(|path| !path.ends_with(&nested_workspace_build))
    );
    let walker_workspace_build =
        PathBuf::from("tests/tools/no-bash-ast-walker").join("BUILD.bazel");
    assert!(
        outputs
            .iter()
            .all(|path| !path.ends_with(&walker_workspace_build))
    );
    let first_bytes = outputs
        .iter()
        .map(|path| fs::read(root.join(path)).expect("preview output"))
        .collect::<Vec<_>>();
    let second = bazel::gen_bazel(&[]).expect("idempotent generator preview");
    assert_eq!(outputs, second);
    assert_eq!(
        first_bytes,
        second
            .iter()
            .map(|path| fs::read(root.join(path)).expect("second preview output"))
            .collect::<Vec<_>>()
    );
    let checked = bazel::gen_bazel(&["--check".to_owned()]).expect("generator check");
    assert_eq!(outputs, checked);

    let stale = root.join("bazel/generated/obsolete-inventory.json");
    fs::create_dir_all(stale.parent().expect("generated parent")).expect("generated directory");
    fs::write(&stale, b"obsolete\n").expect("plant stale generated output");
    let stale_result = bazel::gen_bazel(&["--check".to_owned()]);
    fs::remove_file(&stale).expect("remove stale generated output");
    assert!(
        stale_result
            .expect_err("stale committed output must fail the check")
            .to_string()
            .contains("obsolete-inventory.json")
    );

    let after = fs::read_dir(root.join(".scratch")).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert!(before.is_none() || after.is_some());
}

#[test]
fn generator_metadata_is_offline_and_linux_target_filtered() {
    let source = include_str!("../src/bazel.rs");
    assert!(
        source.contains("const GENERATOR_METADATA_TARGET: &str = \"x86_64-unknown-linux-gnu\"")
    );
    assert!(source.contains("\"--offline\""));
    assert!(source.contains("\"--filter-platform\""));
    assert!(source.contains("GENERATOR_METADATA_TARGET"));
}
