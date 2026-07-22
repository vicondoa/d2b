#![forbid(unsafe_code)]

//! Focused coverage for the checked-in W8 delivery authority
//! (`delivery/manifests/w8.json`). This is a manifest-prep artifact: it is
//! created ahead of the wave landing so the integrator can cherry-pick it once
//! `shared-root-w8-manifest-seam` is flipped. These tests only assert the
//! authority's own shape; they do not touch the W8 wave plan or any component
//! branch.
//!
//! ## Fingerprint lifecycle
//!
//! Every fingerprint path in this manifest (`generated_artifacts`,
//! `dependency_fingerprints`, and `contract_fingerprints`) must name a blob
//! that is already tracked at the current integration HEAD. Several
//! components in `tests/unit/nix/eval-cases/w8-integration-wave-plan.nix`
//! declare `ownedFiles`/`reservedPaths` for files that do not exist yet (most
//! are each component's final integration seam, created only when that
//! component's own commits land). This manifest intentionally does **not**
//! fingerprint those not-yet-created paths, and does **not** create
//! speculative stub files to make them exist early: a snapshot fingerprint
//! must attest to real, already-integrated content, never a placeholder.
//!
//! When a component's commits actually land on the trusted W8 integration
//! branch and create or modify one of its owned files, the integrator adds a
//! new entry to the relevant fingerprint array in `delivery/manifests/w8.json`
//! in the same commit (or immediately after) that lands the change,
//! preserving the strict `(name, repository, path)` sort order the schema
//! requires. The component branch itself never edits the manifest; per
//! `tests/unit/nix/eval-cases/w8-integration-wave-plan.nix`'s
//! `shared-root-w8-manifest-seam` external dependency, `delivery/manifests/`
//! stays integrator/shared-root territory.

use std::path::Path;
use std::process::{Command, Stdio};

use xtask::delivery::{
    DeliveryManifest, ValidationAuthority,
    model::{expected_wave_manifest_path, is_authoritative_manifest_path},
};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
}

fn load_w8_manifest() -> DeliveryManifest {
    let root = repository_root();
    let path = root.join("delivery/manifests/w8.json");
    let bytes = std::fs::read(&path).expect("w8 delivery manifest bytes");
    serde_json::from_slice(&bytes).expect("w8 delivery manifest JSON")
}

/// True when `path` names a blob tracked at `HEAD` in the repository rooted
/// at `root` (a plain filesystem existence check would also accept
/// untracked/local-only files, which is not what a fingerprint must attest).
fn is_tracked_blob_at_head(root: &Path, path: &str) -> bool {
    Command::new("git")
        .current_dir(root)
        .arg("cat-file")
        .arg("-e")
        .arg(format!("HEAD:{path}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run git cat-file")
        .success()
}

/// The reservedPaths declared across every component in
/// `tests/unit/nix/eval-cases/w8-integration-wave-plan.nix` that are already
/// tracked at HEAD today. These are the most sensitive planned integration
/// seams (the file each component may not let any other component touch), so
/// the manifest's fingerprint matrix must cover every one of them regardless
/// of which category (generated, dependency, or contract) carries it. The
/// remaining reservedPaths (each component's not-yet-created final
/// integration file) are intentionally excluded here; see the module-level
/// "Fingerprint lifecycle" note above.
const W8_RESERVED_PATHS: &[&str] = &[
    "packages/d2bd/src/shell_backend.rs",
    "packages/d2bd/src/workload_dispatch.rs",
    "packages/d2b-gateway/src/orchestrator.rs",
    "packages/d2bd/src/realm_stubs.rs",
    "packages/d2bd/src/storage_lifecycle.rs",
];
#[test]
fn w8_manifest_is_valid_and_uniquely_authoritative() {
    let manifest = load_w8_manifest();
    manifest.validate().expect("valid W8 delivery manifest");
    assert_eq!(manifest.wave, "w8");
    assert_eq!(manifest.program, "adr0045");
    assert_eq!(manifest.authority_repository, "github.com/vicondoa/d2b");

    let relative = Path::new("delivery/manifests/w8.json");
    assert!(
        is_authoritative_manifest_path(relative),
        "delivery/manifests/w8.json must be recognised as an authority path"
    );
    assert_eq!(
        expected_wave_manifest_path(&manifest.wave).expect("wave path"),
        relative
    );
    assert!(
        manifest
            .contract_fingerprints
            .iter()
            .any(|fingerprint| fingerprint.path == "delivery/manifests/w8.json"),
        "the W8 manifest must fingerprint itself"
    );

    // No other checked-in authority may also claim wave w8.
    let root = repository_root();
    let legacy_bytes = std::fs::read(root.join("delivery/manifest.json")).expect("legacy bytes");
    let legacy: DeliveryManifest = serde_json::from_slice(&legacy_bytes).expect("legacy JSON");
    assert_ne!(legacy.wave, "w8", "legacy manifest must not also claim w8");
    for entry in std::fs::read_dir(root.join("delivery/manifests")).expect("wave manifests dir") {
        let path = entry.expect("entry").path();
        if path == root.join("delivery/manifests/w8.json") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("sibling manifest bytes");
        let sibling: DeliveryManifest = serde_json::from_slice(&bytes).expect("sibling JSON");
        assert_ne!(
            sibling.wave,
            "w8",
            "{} must not also claim wave w8",
            path.display()
        );
    }
}

#[test]
fn w8_stack_graph_matches_current_single_node_topology() {
    let manifest = load_w8_manifest();

    assert_eq!(
        manifest.repositories.len(),
        1,
        "W8 prep declares exactly one repository"
    );
    let repository = &manifest.repositories[0];
    assert_eq!(repository.id, "github.com/vicondoa/d2b");
    assert_eq!(repository.trunk_ref, "main");
    assert_eq!(repository.integration_ref, "adr0045-w8-integration");

    assert_eq!(
        manifest.stack_nodes.len(),
        1,
        "the current Git Town parent topology for W8 is a single open node \
         against main (all prior wave ancestors are already merged)"
    );
    let node = &manifest.stack_nodes[0];
    assert_eq!(node.id, "d2b-w8");
    assert_eq!(node.repository, "github.com/vicondoa/d2b");
    assert_eq!(node.branch, "adr0045-w8-integration");
    assert_eq!(node.pr_number, 324, "must bind exactly PR #324");
    assert!(
        node.external_dependencies.is_empty(),
        "the sole W8 stack node has no external stack dependencies"
    );
}

#[test]
fn w8_required_validation_is_a_hermetic_dev_shell_invocation() {
    let manifest = load_w8_manifest();

    let make_check = manifest
        .required_validations
        .iter()
        .find(|validation| validation.argv.first().map(String::as_str) != Some("make"))
        .expect("at least one required validation avoids the bare `make` ambient assumption");

    assert_eq!(
        make_check.argv,
        vec!["nix", "develop", "--command", "make", "check"],
        "the candidate-pinned local validation must run through `nix develop` so it is \
         hermetic in a controlled detached checkout, not `make check` against ambient tools"
    );
    assert_eq!(make_check.cwd.repository, "github.com/vicondoa/d2b");
    assert_eq!(make_check.cwd.path, ".");
    assert_eq!(make_check.authority, ValidationAuthority::LocalRunner);
    assert!(make_check.ci_publisher.is_none());
    assert!(make_check.ci_signer_workflow.is_none());
    assert!(make_check.timeout_seconds > 0);

    // No required validation may fall back to a bare, non-hermetic invocation.
    for validation in &manifest.required_validations {
        assert_ne!(
            validation.argv.first().map(String::as_str),
            Some("make"),
            "required validation {} must not assume ambient tooling",
            validation.id
        );
    }
}

#[test]
fn w8_required_checks_match_the_live_branch_protection_contexts() {
    let manifest = load_w8_manifest();

    let node_checks: std::collections::BTreeSet<&str> = manifest
        .required_checks
        .iter()
        .filter(|check| check.node == "d2b-w8")
        .map(|check| check.name.as_str())
        .collect();
    assert_eq!(
        node_checks,
        std::collections::BTreeSet::from(["check", "eval", "eval-shell-tests"]),
        "must match the exact required_status_checks contexts on main"
    );

    let expectations = [
        ("check", "pr-l1-static-fast", 286_033_206_u64),
        ("eval", "eval-with-entra-id", 283_041_463_u64),
        ("eval-shell-tests", "pr-eval-shell-tests", 289_704_928_u64),
    ];
    for (name, workflow, workflow_id) in expectations {
        let check = manifest
            .required_checks
            .iter()
            .find(|check| check.node == "d2b-w8" && check.name == name)
            .unwrap_or_else(|| panic!("missing required check {name}"));
        assert_eq!(check.publisher.app_slug, "github-actions");
        assert_eq!(check.publisher.app_id, 15368);
        assert_eq!(check.publisher.workflow, workflow);
        assert_eq!(check.publisher.workflow_id, workflow_id);
    }

    // Every configured stack node has at least one required check (already
    // enforced by `validate()`, re-asserted here as a direct regression guard
    // scoped to this manifest).
    let configured_nodes: std::collections::BTreeSet<&str> = manifest
        .stack_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    let checked_nodes: std::collections::BTreeSet<&str> = manifest
        .required_checks
        .iter()
        .map(|check| check.node.as_str())
        .collect();
    assert_eq!(configured_nodes, checked_nodes);
}

#[test]
fn w8_fingerprint_matrix_covers_every_reserved_integration_seam() {
    let manifest = load_w8_manifest();

    let all_paths: std::collections::BTreeSet<&str> = manifest
        .generated_artifacts
        .iter()
        .chain(&manifest.dependency_fingerprints)
        .chain(&manifest.contract_fingerprints)
        .map(|fingerprint| fingerprint.path.as_str())
        .collect();

    for reserved in W8_RESERVED_PATHS {
        assert!(
            all_paths.contains(reserved),
            "fingerprint matrix is missing reserved integration seam {reserved}"
        );
    }

    // The wave plan itself is a planned-integration-surface input and must be
    // fingerprinted so drift in the plan invalidates the prep manifest.
    assert!(
        all_paths.contains("tests/unit/nix/eval-cases/w8-integration-wave-plan.nix"),
        "the W8 wave plan must be fingerprinted by its own manifest prep"
    );

    assert!(
        !manifest.dependency_fingerprints.is_empty(),
        "dependency fingerprints must be non-empty"
    );
    assert!(
        !manifest.contract_fingerprints.is_empty(),
        "contract fingerprints must be non-empty"
    );
}

#[test]
fn w8_all_fingerprint_paths_are_tracked_blobs_at_head() {
    // A fingerprint is a snapshot attestation over already-integrated
    // content. If any `generated_artifacts` / `dependency_fingerprints` /
    // `contract_fingerprints` entry named a path that is not yet a tracked
    // blob at HEAD, the manifest would be attesting to a file that doesn't
    // exist in the tree it claims to describe -- exactly the
    // snapshot-blocking defect this test guards against. See the
    // "Fingerprint lifecycle" module doc for how future component paths are
    // added once their creating commits actually land.
    let manifest = load_w8_manifest();
    let root = repository_root();

    let mut untracked = Vec::new();
    for fingerprint in manifest
        .generated_artifacts
        .iter()
        .chain(&manifest.dependency_fingerprints)
        .chain(&manifest.contract_fingerprints)
    {
        if !is_tracked_blob_at_head(root, &fingerprint.path) {
            untracked.push(fingerprint.path.clone());
        }
    }

    assert!(
        untracked.is_empty(),
        "every fingerprint path must be a tracked blob at HEAD; found untracked/future paths: {untracked:#?}"
    );
}
