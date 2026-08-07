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
    assert!(!outputs.is_empty());
    assert!(outputs.iter().all(|path| path.starts_with(".scratch")));
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
    let after = fs::read_dir(root.join(".scratch")).ok().map(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert!(before.is_none() || after.is_some());
}
