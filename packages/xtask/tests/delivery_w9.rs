#![forbid(unsafe_code)]

//! Focused policy tests for the W9 (toolkit and sibling cutover) delivery
//! authority: `delivery/manifests/w9.json`. These tests are self-contained
//! within the `github.com/vicondoa/d2b` checkout: they never resolve an
//! absolute local path into a sibling repository, since the five sibling
//! repositories are not part of this Git history and must not be assumed
//! present on every machine or CI runner that exercises this test binary.

use std::{collections::BTreeMap, collections::BTreeSet, path::Path};

use serde_json::Value;
use xtask::delivery::{
    DeliveryManifest,
    model::{expected_wave_manifest_path, is_authoritative_manifest_path},
};

fn repo_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf()
    })
}

fn w9_manifest() -> DeliveryManifest {
    let bytes =
        std::fs::read(repo_root().join("delivery/manifests/w9.json")).expect("w9 manifest bytes");
    let manifest: DeliveryManifest = serde_json::from_slice(&bytes).expect("w9 manifest JSON");
    manifest.validate().expect("w9 manifest is valid");
    manifest
}

fn coordination_doc() -> Value {
    let bytes = std::fs::read(repo_root().join("docs/adr/0045-toolkit-sibling-coordination.json"))
        .expect("coordination doc bytes");
    serde_json::from_slice(&bytes).expect("coordination doc JSON")
}

fn toolkit_source_contract() -> Value {
    let bytes = std::fs::read(repo_root().join("docs/reference/toolkit-source-contract.json"))
        .expect("toolkit source contract bytes");
    serde_json::from_slice(&bytes).expect("toolkit source contract JSON")
}

/// Proves exact six-repository membership: the W9 authority names precisely
/// the six repositories named in the task (d2b, d2b-toolkit,
/// d2b-provider-toolkit, d2b-wlcontrol, d2b-wlterm, weezterm), and no other,
/// under `authority_repository = github.com/vicondoa/d2b`.
#[test]
fn w9_manifest_declares_exact_six_repository_membership() {
    let manifest = w9_manifest();
    assert_eq!(manifest.wave, "w9");
    assert_eq!(manifest.authority_repository, "github.com/vicondoa/d2b");

    let repository_ids: BTreeSet<&str> = manifest
        .repositories
        .iter()
        .map(|repository| repository.id.as_str())
        .collect();
    let expected: BTreeSet<&str> = [
        "github.com/vicondoa/d2b",
        "github.com/vicondoa/d2b-toolkit",
        "github.com/vicondoa/d2b-provider-toolkit",
        "github.com/vicondoa/d2b-wlcontrol",
        "github.com/vicondoa/d2b-wlterm",
        "github.com/vicondoa/weezterm",
    ]
    .into_iter()
    .collect();
    assert_eq!(repository_ids, expected);
    assert_eq!(manifest.repositories.len(), 6);

    // Every repository trunk is main, and every integration ref is the
    // specific adr0045-w9-* branch audited for this wave; none may collide.
    let mut integration_refs = BTreeSet::new();
    for repository in &manifest.repositories {
        assert_eq!(repository.trunk_ref, "main");
        assert!(repository.integration_ref.starts_with("adr0045-w9-"));
        assert!(integration_refs.insert(repository.integration_ref.as_str()));
    }
}

/// Proves the checked-in PR/ref/dependency graph matches the exact ordinary
/// PR topology from `docs/adr/0045-toolkit-sibling-coordination.json` and the
/// PR numbers/branches given as ground truth for this wave: d2b PR314 on
/// `adr0045-w9-toolkits`; d2b-toolkit PR6 on `adr0045-w9-client-toolkit`;
/// d2b-provider-toolkit PR1 on `adr0045-w9-provider-toolkit`; d2b-wlcontrol
/// PR37 on `adr0045-w9-wlcontrol`; d2b-wlterm PR20 on `adr0045-w9-wlterm`;
/// weezterm PR48 on `adr0045-w9-weezterm`.
#[test]
fn w9_manifest_stack_topology_matches_expected_pr_ref_graph() {
    let manifest = w9_manifest();
    assert_eq!(manifest.stack_nodes.len(), 6);

    let expected: BTreeMap<&str, (&str, &str, u64)> = [
        (
            "d2b-w9",
            ("github.com/vicondoa/d2b", "adr0045-w9-toolkits", 314),
        ),
        (
            "client-toolkit",
            (
                "github.com/vicondoa/d2b-toolkit",
                "adr0045-w9-client-toolkit",
                6,
            ),
        ),
        (
            "provider-toolkit",
            (
                "github.com/vicondoa/d2b-provider-toolkit",
                "adr0045-w9-provider-toolkit",
                1,
            ),
        ),
        (
            "wlcontrol",
            (
                "github.com/vicondoa/d2b-wlcontrol",
                "adr0045-w9-wlcontrol",
                37,
            ),
        ),
        (
            "wlterm",
            ("github.com/vicondoa/d2b-wlterm", "adr0045-w9-wlterm", 20),
        ),
        (
            "weezterm",
            ("github.com/vicondoa/weezterm", "adr0045-w9-weezterm", 48),
        ),
    ]
    .into_iter()
    .collect();

    let mut seen_ids = BTreeSet::new();
    for node in &manifest.stack_nodes {
        let (repository, branch, pr_number) = *expected
            .get(node.id.as_str())
            .unwrap_or_else(|| panic!("unexpected stack node id {}", node.id));
        assert_eq!(node.repository, repository, "node {}", node.id);
        assert_eq!(node.branch, branch, "node {}", node.id);
        assert_eq!(node.pr_number, pr_number, "node {}", node.id);
        assert!(seen_ids.insert(node.id.as_str()));
    }
    assert_eq!(seen_ids, expected.keys().copied().collect::<BTreeSet<_>>());

    // The three desktop/terminal consumers depend on the client toolkit
    // distribution node; the two toolkit foundations are dependency roots;
    // the d2b integration node closes over every sibling, since it pins and
    // validates all five sibling revisions.
    let by_id: BTreeMap<&str, &xtask::delivery::model::StackNodePolicy> = manifest
        .stack_nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    for consumer in ["wlterm", "wlcontrol", "weezterm"] {
        assert_eq!(
            by_id[consumer].external_dependencies,
            vec!["client-toolkit".to_string()],
            "consumer {consumer}"
        );
    }
    for root in ["client-toolkit", "provider-toolkit"] {
        assert!(
            by_id[root].external_dependencies.is_empty(),
            "root {root} must have no external dependencies"
        );
    }
    let mut d2b_dependencies = by_id["d2b-w9"].external_dependencies.clone();
    d2b_dependencies.sort();
    assert_eq!(
        d2b_dependencies,
        vec![
            "client-toolkit".to_string(),
            "provider-toolkit".to_string(),
            "weezterm".to_string(),
            "wlcontrol".to_string(),
            "wlterm".to_string(),
        ]
    );

    // The dependency graph must be acyclic: a Kahn's-algorithm topological
    // sort must be able to consume every node.
    let mut indegree: BTreeMap<&str, usize> = by_id.keys().map(|id| (*id, 0usize)).collect();
    for node in by_id.values() {
        *indegree.get_mut(node.id.as_str()).unwrap() = node.external_dependencies.len();
    }
    let mut ready: Vec<&str> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut resolved = BTreeSet::new();
    while let Some(id) = ready.pop() {
        assert!(resolved.insert(id), "node {id} resolved twice");
        for node in by_id.values() {
            if node.external_dependencies.iter().any(|dep| dep == id)
                && node
                    .external_dependencies
                    .iter()
                    .all(|dep| resolved.contains(dep.as_str()))
                && !resolved.contains(node.id.as_str())
                && !ready.contains(&node.id.as_str())
            {
                ready.push(node.id.as_str());
            }
        }
    }
    assert_eq!(
        resolved,
        by_id.keys().copied().collect::<BTreeSet<_>>(),
        "stack dependency graph must be acyclic and fully resolvable"
    );

    // Cross-check against the coordination doc: every stack node's audited
    // revision-bearing branch matches the component's auditedRef, and PR
    // numbers are consistent with the coordination doc's own repository ids.
    let coordination = coordination_doc();
    let components = coordination["components"].as_array().expect("components");
    assert_eq!(components.len(), 5, "five toolkit/sibling components");
    for component in components {
        let repository = component["repository"].as_str().expect("repository");
        let audited_ref = component["auditedRef"].as_str().expect("auditedRef");
        let full_id = format!("github.com/{repository}");
        let node = manifest
            .stack_nodes
            .iter()
            .find(|node| node.repository == full_id)
            .unwrap_or_else(|| panic!("no stack node for coordinated repository {repository}"));
        assert_eq!(node.branch, audited_ref, "component {repository}");
    }
}

/// Proves manifest selection uniqueness for wave w9: exactly one checked-in
/// authority (`delivery/manifests/w9.json`) may declare `"wave": "w9"`, its
/// path is the canonical `expected_wave_manifest_path("w9")`, and no other
/// tracked authority (legacy `delivery/manifest.json` or any other
/// `delivery/manifests/*.json`) also claims wave w9.
#[test]
fn w9_manifest_is_the_unique_selected_authority_for_its_wave() {
    let root = repo_root();
    let w9_path = root.join("delivery/manifests/w9.json");
    let relative = w9_path
        .strip_prefix(root)
        .expect("repository-relative path");
    assert!(is_authoritative_manifest_path(relative));
    assert_eq!(
        relative,
        expected_wave_manifest_path("w9").expect("expected w9 path")
    );

    let mut wave_authorities = 0usize;
    let paths = std::iter::once(root.join("delivery/manifest.json")).chain(
        std::fs::read_dir(root.join("delivery/manifests"))
            .expect("per-wave manifest directory")
            .map(|entry| entry.expect("manifest entry").path()),
    );
    for path in paths {
        let manifest: DeliveryManifest =
            serde_json::from_slice(&std::fs::read(&path).expect("manifest bytes"))
                .expect("manifest JSON");
        if manifest.wave == "w9" {
            wave_authorities += 1;
            let relative = path.strip_prefix(root).expect("repository-relative path");
            assert_eq!(relative, w9_path.strip_prefix(root).unwrap());
        }
    }
    assert_eq!(
        wave_authorities, 1,
        "exactly one checked-in authority may declare wave w9"
    );
}

/// Proves there is no duplicate toolkit DTO ownership: the canonical
/// protocol-ownership contract (`docs/reference/toolkit-source-contract.json`,
/// referenced as `protocolSource` by the coordination doc) never re-declares
/// the same source path under two different `apiSurfaces` entries, and the
/// legacy per-sibling duplicate protocol sources the coordination doc lists
/// for deletion (`duplicateProtocolSourcesToDelete`) are disjoint from the
/// paths the contract currently recognizes as canonically owned.
#[test]
fn w9_toolkit_source_contract_has_no_duplicate_dto_ownership() {
    let coordination = coordination_doc();
    assert_eq!(
        coordination["protocolSource"]
            .as_str()
            .expect("protocolSource"),
        "docs/reference/toolkit-source-contract.json"
    );

    let contract = toolkit_source_contract();
    let surfaces = contract["apiSurfaces"].as_array().expect("apiSurfaces");
    assert!(!surfaces.is_empty());

    // A single-owner surface (`owner` has no comma) is a primary DTO
    // ownership claim: two different single-owner surfaces must never claim
    // the same path, since that would be a real duplicate-ownership
    // conflict. A comma-joined owner (for example "redaction", owned by
    // "d2b-client,d2b-session,d2b-provider-toolkit") is a deliberate,
    // explicitly-declared cross-cutting concern layered over already-owned
    // files, not a competing primary owner, so it is exempt from the
    // pairwise-uniqueness check below but still counts as "canonically
    // owned" for the legacy-duplicate cross-check further down.
    let mut single_owners_by_path: BTreeMap<&str, &str> = BTreeMap::new();
    let mut all_owned_paths: BTreeSet<&str> = BTreeSet::new();
    for surface in surfaces {
        let id = surface["id"].as_str().expect("surface id");
        let owner = surface["owner"].as_str().expect("surface owner");
        let is_shared = owner.contains(',');
        for path in surface["paths"].as_array().expect("surface paths") {
            let path = path.as_str().expect("surface path string");
            all_owned_paths.insert(path);
            if is_shared {
                continue;
            }
            if let Some(previous_owner) = single_owners_by_path.insert(path, owner) {
                panic!(
                    "path {path} is claimed by two distinct primary owners {previous_owner:?} and {owner:?} (surface {id})"
                );
            }
        }
    }

    // Collect every path the coordination doc still expects each component to
    // delete because it duplicates canonical protocol/DTO ownership.
    let components = coordination["components"].as_array().expect("components");
    let mut duplicate_paths = BTreeSet::new();
    for component in components {
        for path in component["duplicateProtocolSourcesToDelete"]
            .as_array()
            .expect("duplicateProtocolSourcesToDelete")
        {
            duplicate_paths.insert(path.as_str().expect("duplicate path string").to_string());
        }
    }
    assert!(
        !duplicate_paths.is_empty(),
        "at least one component must still name legacy duplicate sources"
    );

    // None of those legacy duplicate paths may be a path the contract
    // currently recognizes as canonically owned: that would mean the
    // "to delete" duplicate is simultaneously claimed as the canonical
    // single-owner source, an ownership contradiction.
    for duplicate in &duplicate_paths {
        assert!(
            !all_owned_paths.contains(duplicate.as_str()),
            "legacy duplicate source {duplicate} must not also be a canonically owned path"
        );
    }
}

/// Proves the four sibling toolkit/consumer nodes' `required_checks`
/// publishers name the exact live GitHub Workflow `name` field, not the
/// workflow *filename*. The delivery command layer's `optional_workflow_run`
/// binds `RequiredCheck::publisher.workflow` against the live check suite's
/// `workflowRun.workflow.name`, which is the workflow's display name (for
/// example `ci`, `CI`, `check`) and can differ in case and form from its
/// `.github/workflows/<file>.yml` filename. Regressing this field back to a
/// filename silently makes every one of that sibling's required checks
/// permanently unmatchable, since the live workflow never reports a `.yml`
/// suffix as its name.
#[test]
fn w9_required_check_publishers_name_exact_live_workflow_names_not_filenames() {
    let manifest = w9_manifest();
    let expected_workflow_names: BTreeMap<&str, &str> = [
        ("client-toolkit", "ci"),
        ("provider-toolkit", "CI"),
        ("wlcontrol", "CI"),
        ("wlterm", "check"),
    ]
    .into_iter()
    .collect();
    // The live workflow `databaseId` for each of these four nodes must be
    // preserved exactly across the filename-to-name correction: only the
    // human-readable `name` was wrong, not the workflow identity.
    let expected_workflow_ids: BTreeMap<&str, u64> = [
        ("client-toolkit", 307600819),
        ("provider-toolkit", 314261598),
        ("wlcontrol", 296720226),
        ("wlterm", 307600733),
    ]
    .into_iter()
    .collect();

    let mut seen_nodes = BTreeSet::new();
    for check in &manifest.required_checks {
        let Some(expected) = expected_workflow_names.get(check.node.as_str()) else {
            continue;
        };
        seen_nodes.insert(check.node.as_str());
        assert_eq!(
            check.publisher.workflow, *expected,
            "node {} check {} publisher.workflow must be the live workflow name",
            check.node, check.name
        );
        assert_eq!(
            check.publisher.workflow_id,
            expected_workflow_ids[check.node.as_str()],
            "node {} check {} publisher.workflow_id must be preserved",
            check.node,
            check.name
        );
        assert!(
            !check.publisher.workflow.ends_with(".yml")
                && !check.publisher.workflow.ends_with(".yaml"),
            "node {} check {} publisher.workflow must not be a workflow filename",
            check.node,
            check.name
        );
    }
    assert_eq!(
        seen_nodes,
        expected_workflow_names
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        "every sibling toolkit/consumer node must have at least one required check"
    );
}
