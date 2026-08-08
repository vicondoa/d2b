#[path = "../src/hermeticity.rs"]
mod hermeticity;

#[test]
fn complete_inventory_covers_configured_aquery_and_strategy_sets() {
    let inventory = hermeticity::complete_action_network_inventory();
    hermeticity::validate_action_network_inventory(&inventory).expect("complete inventory");
    assert_eq!(inventory.action_network, "none");
    assert_eq!(inventory.sandbox_provider, "pkgs/bazel-8.6.0-seccomp");
    assert_eq!(inventory.capability_abi, "d2b-bazel-seccomp-abi-v1");
    assert!(inventory.repository_fetches_outside_actions);
    assert!(inventory.fallback_strategies.is_empty());
    assert!(
        inventory
            .strategy_inventory
            .values()
            .all(|strategy| strategy == "sandboxed")
    );
}

#[test]
fn non_sandboxed_strategy_and_missing_pre_action_plants_refuse() {
    let mut inventory = hermeticity::complete_action_network_inventory();
    inventory
        .strategy_inventory
        .insert("stable:Rustc".to_owned(), "process".to_owned());
    let error = hermeticity::validate_action_network_inventory(&inventory)
        .expect_err("process strategy must refuse");
    assert!(error.to_string().contains("non-sandbox"));

    let mut inventory = hermeticity::complete_action_network_inventory();
    inventory
        .socket_plants
        .retain(|plant| plant != "action-network-io-uring");
    assert!(matches!(
        hermeticity::validate_action_network_inventory(&inventory),
        Err(hermeticity::ActionNetworkError::MissingPlant(_))
    ));
}

#[test]
fn rendered_action_network_inventory_is_deterministic_json() {
    let first = hermeticity::action_network_json().expect("first inventory");
    let second = hermeticity::action_network_json().expect("second inventory");
    assert_eq!(first, second);
    assert!(first.ends_with('\n'));
    assert!(first.contains("\"action_network\": \"none\""));
    assert!(first.contains("\"strategy_inventory\""));
}

#[test]
fn observed_configured_aquery_and_effective_strategy_sets_reconcile() {
    let configured = serde_json::json!({
        "actionKinds": hermeticity::GOVERNED_ACTION_KINDS,
        "coverageTargets": hermeticity::CONFIGURED_COVERAGE_TARGETS,
    })
    .to_string();
    let aquery = serde_json::json!({
        "actions": hermeticity::GOVERNED_ACTION_KINDS
            .iter()
            .map(|kind| serde_json::json!({"kind": kind}))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let strategies = serde_json::json!({
        "strategies": hermeticity::GOVERNED_ACTION_KINDS
            .iter()
            .map(|kind| (*kind, hermeticity::SANDBOX_STRATEGY))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "fallbackStrategies": [],
    })
    .to_string();
    let observation =
        hermeticity::ActionNetworkObservation::from_json(&configured, &aquery, &strategies)
            .expect("real observation shape");
    let inventory = hermeticity::complete_action_network_inventory_from_observation(&observation);
    hermeticity::validate_observed_action_network(&inventory, &observation)
        .expect("observations reconcile");
}

#[test]
fn observed_strategy_mutation_refuses_local_fallback() {
    let configured = serde_json::json!({
        "actionKinds": hermeticity::GOVERNED_ACTION_KINDS,
        "coverageTargets": hermeticity::CONFIGURED_COVERAGE_TARGETS,
    })
    .to_string();
    let aquery = serde_json::json!({
        "actions": hermeticity::GOVERNED_ACTION_KINDS,
    })
    .to_string();
    let strategies = serde_json::json!({
        "strategies": {
            "stable:Rustc": "local",
            "stable:RustcMetadata": "sandboxed",
        },
    })
    .to_string();
    let observation =
        hermeticity::ActionNetworkObservation::from_json(&configured, &aquery, &strategies)
            .expect("mutated observation parses");
    let inventory = hermeticity::complete_action_network_inventory_from_observation(&observation);
    assert!(matches!(
        hermeticity::validate_observed_action_network(&inventory, &observation),
        Err(hermeticity::ActionNetworkError::WrongStrategy { .. })
    ));
}

#[test]
fn pinned_toolchain_record_requires_both_native_outputs_and_patch_identity() {
    let record = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .join("tests/golden/bazel-toolchain.json"),
    )
    .expect("toolchain record");
    hermeticity::validate_pinned_toolchain_record(&record).expect("pinned record");
    let mut wrong = record;
    wrong = wrong.replace("\"sandboxed\"", "\"process\"");
    assert!(hermeticity::validate_pinned_toolchain_record(&wrong).is_err());
}
