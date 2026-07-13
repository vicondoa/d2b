//! Source policy for the least-privilege d2b-contracts feature surface.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use d2b_contract_tests::{read_repo_file, repo_root};
use regex::Regex;

fn feature_graph(manifest: &str) -> BTreeMap<String, BTreeSet<String>> {
    let features = manifest
        .split_once("[features]")
        .expect("d2b-contracts must declare [features]")
        .1
        .split_once("\n[dependencies]")
        .expect("[features] must precede [dependencies]")
        .0;
    let assignment =
        Regex::new(r#"(?m)^([a-z0-9-]+)\s*=\s*\[([\s\S]*?)\]\s*$"#).expect("valid regex");
    let quoted = Regex::new(r#""([^"]+)""#).expect("valid regex");
    let mut graph = BTreeMap::new();
    for capture in assignment.captures_iter(features) {
        let name = capture[1].to_owned();
        let refs: BTreeSet<String> = quoted
            .captures_iter(&capture[2])
            .map(|reference| reference[1].to_owned())
            .collect();
        graph.insert(name, refs);
    }
    let names: BTreeSet<_> = graph.keys().cloned().collect();
    for refs in graph.values_mut() {
        refs.retain(|reference| names.contains(reference));
    }
    graph
}

fn visit_feature(
    feature: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) {
    if visited.contains(feature) {
        return;
    }
    assert!(
        visiting.insert(feature.to_owned()),
        "d2b-contracts feature cycle includes {feature}"
    );
    for dependency in &graph[feature] {
        visit_feature(dependency, graph, visiting, visited);
    }
    visiting.remove(feature);
    visited.insert(feature.to_owned());
}

#[test]
fn contracts_default_is_empty_and_feature_graph_is_acyclic() {
    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    let graph = feature_graph(&manifest);
    assert_eq!(graph.get("default"), Some(&BTreeSet::new()));

    let mut visited = BTreeSet::new();
    for feature in graph.keys() {
        visit_feature(feature, &graph, &mut BTreeSet::new(), &mut visited);
    }

    let expected: BTreeMap<&str, BTreeSet<&str>> = [
        ("default", BTreeSet::new()),
        ("common", BTreeSet::new()),
        ("guest-auth", BTreeSet::new()),
        ("usbip", BTreeSet::new()),
        ("security-key", BTreeSet::new()),
        ("guest", BTreeSet::from(["common", "guest-auth", "usbip"])),
        (
            "broker",
            BTreeSet::from(["common", "guest-auth", "security-key", "usbip"]),
        ),
        ("public", BTreeSet::from(["broker", "guest"])),
        ("cli-output", BTreeSet::from(["public"])),
        ("unsafe-local", BTreeSet::from(["public"])),
        (
            "schema",
            BTreeSet::from([
                "cli-output",
                "unsafe-local",
                "v2-component-session",
                "v2-identity",
                "v2-provider",
                "v2-services",
                "v2-state",
            ]),
        ),
        ("v2-identity", BTreeSet::new()),
        ("v2-component-session", BTreeSet::from(["v2-identity"])),
        (
            "v2-services",
            BTreeSet::from(["v2-component-session", "v2-provider", "v2-state"]),
        ),
        ("v2-provider", BTreeSet::from(["v2-component-session"])),
        ("v2-state", BTreeSet::from(["v2-identity"])),
    ]
    .into_iter()
    .collect();
    let actual: BTreeMap<&str, BTreeSet<&str>> = graph
        .iter()
        .map(|(feature, refs)| {
            (
                feature.as_str(),
                refs.iter().map(String::as_str).collect::<BTreeSet<_>>(),
            )
        })
        .collect();
    assert_eq!(
        actual, expected,
        "feature edges changed; update the reviewed least-privilege matrix intentionally"
    );
}

#[test]
fn schema_and_protobuf_dependencies_are_optional_and_scoped() {
    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    for dependency in [
        "d2b-core",
        "d2b-realm-core",
        "serde",
        "serde_json",
        "schemars",
        "protobuf",
        "sha2",
    ] {
        let dependency_line = manifest
            .lines()
            .find(|line| line.starts_with(dependency))
            .unwrap_or_else(|| panic!("missing {dependency} dependency"));
        assert!(
            dependency_line.contains("optional = true"),
            "{dependency} must remain optional: {dependency_line}"
        );
    }
    assert!(
        manifest.contains(r#"guest = ["#)
            && manifest.contains(r#""dep:protobuf""#)
            && !manifest.contains(r#"common = ["dep:protobuf""#),
        "protobuf must be activated only by guest and v2-services"
    );
}

#[test]
fn maintained_consumers_select_explicit_domain_features() {
    let packages = repo_root().join("packages");
    let allowed_schema_consumers = BTreeSet::from(["d2b-contract-tests", "xtask"]);
    let mut consumers = Vec::new();
    for entry in fs::read_dir(&packages).expect("read packages") {
        let entry = entry.expect("read package entry");
        if !entry.file_type().expect("package file type").is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let crate_name = entry.file_name().to_string_lossy().into_owned();
        let manifest = fs::read_to_string(&manifest_path).expect("read crate manifest");
        for line in manifest
            .lines()
            .filter(|line| line.trim_start().starts_with("d2b-contracts ="))
        {
            consumers.push(crate_name.clone());
            assert!(
                line.contains("features = ["),
                "{crate_name} must select explicit d2b-contracts features: {line}"
            );
            assert!(
                !line.contains("\"full\""),
                "{crate_name} must not use a catch-all full feature"
            );
            if line.contains("\"schema\"") {
                assert!(
                    allowed_schema_consumers.contains(crate_name.as_str()),
                    "{crate_name} must select production domain features, not schema"
                );
            }
        }
    }
    assert!(
        consumers.len() >= 11,
        "expected every maintained d2b-contracts consumer, found {consumers:?}"
    );

    let workspace = read_repo_file("packages/Cargo.toml");
    let workspace_line = workspace
        .lines()
        .find(|line| line.starts_with("d2b-contracts ="))
        .expect("workspace d2b-contracts declaration");
    assert!(
        workspace_line.contains("default-features = false"),
        "workspace dependency must disable defaults"
    );
}

#[test]
fn v2_rails_are_independently_owned_without_current_aliases() {
    let lib = read_repo_file("packages/d2b-contracts/src/lib.rs");
    assert!(
        !lib.contains("pub use d2b_core::"),
        "current d2b-core types must not be broadly re-exported from the crate root"
    );
    for (feature, module) in [
        ("v2-identity", "v2_identity"),
        ("v2-component-session", "v2_component_session"),
        ("v2-services", "v2_services"),
        ("v2-provider", "v2_provider"),
        ("v2-state", "v2_state"),
    ] {
        let gate = format!("#[cfg(feature = \"{feature}\")]\npub mod {module};");
        assert!(lib.contains(&gate), "missing independent rail gate: {gate}");
        let source = read_repo_file(&format!("packages/d2b-contracts/src/{module}.rs"));
        for forbidden in [
            "pub use",
            "broker_wire",
            "cli_output",
            "guest_auth",
            "guest_proto",
            "guest_wire",
            "public_wire",
            "security_key",
            "terminal_wire",
            "unsafe_local_wire",
            "usbip",
        ] {
            assert!(
                !source.lines().any(|line| {
                    let line = line.trim_start();
                    !line.starts_with("//!") && line.contains(forbidden)
                }),
                "{module} must not alias or re-export current contracts; found {forbidden:?}"
            );
        }
    }

    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    let schema = manifest
        .split_once("schema = [")
        .and_then(|(_, trailing)| trailing.split_once("]\n"))
        .map(|(schema, _)| schema)
        .expect("multiline schema feature");
    for rail in [
        "\"v2-identity\"",
        "\"v2-component-session\"",
        "\"v2-services\"",
        "\"v2-provider\"",
        "\"v2-state\"",
    ] {
        assert!(schema.contains(rail), "schema feature must include {rail}");
    }
}
