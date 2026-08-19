//! The Wave 6 accounting manifest is an ordinary, fail-closed Layer-1 gate.
//!
//! Provider dossier state is historical planning metadata and is not used as
//! delivery evidence. Every current Wave 6 dossier heading, plus the two
//! integration work items, must appear once and point at one canonical
//! foundation or Provider package with named validation and removal proof.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use serde::Deserialize;

const MANIFEST: &str = "docs/reference/wave6-foundation-manifest.json";
const EXPECTED_ENTRY_COUNT: usize = 258;
const EXPECTED_PROVIDER_SPEC_COUNT: usize = 27;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    artifact_kind: String,
    source: Source,
    owners: BTreeMap<String, Owner>,
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    adr: String,
    selection: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    kind: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    work_item_id: String,
    spec_id: String,
    spec_path: String,
    owner: String,
    status: String,
    implementation_path: String,
    validation_proof: String,
    removal_proof: String,
    source_state: String,
    #[serde(default)]
    source_work_item: Option<String>,
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| {
        panic!(
            "failed to read specification directory {}: {err}",
            dir.display()
        )
    }) {
        let entry = entry.expect("read specification directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
}

fn table_value(line: &str, field: &str) -> Option<String> {
    let prefix = format!("| {field} |");
    let value = line.strip_prefix(&prefix)?.strip_suffix('|')?.trim();
    Some(value.trim_matches('`').to_owned())
}

fn wave6_source_work_items() -> BTreeMap<String, (String, String)> {
    let root = d2b_contract_tests::repo_root();
    let specs = root.join("docs/specs/providers");
    let mut files = Vec::new();
    collect_markdown_files(&specs, &mut files);
    files.sort();

    let mut work_items = BTreeMap::new();
    for path in files {
        if path.file_name().is_some_and(|name| name == "README.md") {
            continue;
        }
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read specification {}: {err}", path.display()));
        let lines: Vec<&str> = content.lines().collect();
        let spec_id = lines
            .iter()
            .find_map(|line| table_value(line, "Spec ID"))
            .unwrap_or_else(|| panic!("specification {} is missing Spec ID", path.display()));
        let spec_path = path
            .strip_prefix(&root)
            .expect("specification is below repository root")
            .to_string_lossy()
            .replace('\\', "/");
        for line in lines {
            let Some(heading) = line.strip_prefix("### ") else {
                continue;
            };
            let work_item_id = heading.split_whitespace().next().unwrap_or_default();
            if !work_item_id.starts_with("ADR046-") {
                continue;
            }
            assert!(
                work_items
                    .insert(
                        work_item_id.to_owned(),
                        (spec_id.clone(), spec_path.clone())
                    )
                    .is_none(),
                "Wave 6 work item {work_item_id} appears in more than one dossier"
            );
        }
    }
    assert_eq!(
        work_items.len(),
        EXPECTED_ENTRY_COUNT - 2,
        "the 27 current Provider dossiers must account for 256 work items"
    );
    work_items
}

#[test]
fn wave6_manifest_maps_every_work_item_once_to_landed_code_and_proof() {
    assert!(repo_path_exists(MANIFEST), "missing {MANIFEST}");
    let manifest: Manifest =
        serde_json::from_str(&read_repo_file(MANIFEST)).expect("valid Wave 6 manifest JSON");

    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.artifact_kind, "d2b-wave6-foundation-manifest");
    assert_eq!(manifest.source.adr, "0046");
    assert_eq!(
        manifest.source.selection,
        "all-wave6-provider-and-integration-work-items"
    );
    assert_eq!(manifest.entries.len(), EXPECTED_ENTRY_COUNT);

    let provider_source = wave6_source_work_items();
    let mut expected = provider_source.clone();
    expected.insert(
        "wi:core-controller-coordination:w6".to_owned(),
        (
            "ADR-046-provider-system-core".to_owned(),
            "docs/specs/providers/ADR-046-provider-system-core.md".to_owned(),
        ),
    );
    expected.insert(
        "wi:process-provider-integration:w6".to_owned(),
        (
            "ADR-046-provider-system-systemd".to_owned(),
            "docs/specs/providers/ADR-046-provider-system-systemd.md".to_owned(),
        ),
    );

    let mut work_items = BTreeSet::new();
    let mut used_owners = BTreeSet::new();
    for entry in &manifest.entries {
        assert!(
            work_items.insert(&entry.work_item_id),
            "work item {} is mapped more than once",
            entry.work_item_id
        );
        let (expected_spec_id, expected_spec_path) = expected
            .get(&entry.work_item_id)
            .unwrap_or_else(|| panic!("unrecognized Wave 6 work item {}", entry.work_item_id));
        assert_eq!(&entry.spec_id, expected_spec_id);
        assert_eq!(&entry.spec_path, expected_spec_path);
        assert_eq!(
            entry.status, "Landed",
            "accounting status must not inherit stale Planned dossier state"
        );
        assert!(
            !entry.source_state.is_empty(),
            "{} must retain the source dossier state for auditability",
            entry.work_item_id
        );
        if entry.work_item_id.starts_with("wi:") {
            assert_eq!(
                entry.source_work_item.as_deref(),
                Some(entry.work_item_id.as_str()),
                "integration rows must retain their graph work-item identity"
            );
        }
        assert!(repo_path_exists(&entry.spec_path));
        assert!(
            manifest.owners.contains_key(&entry.owner),
            "{} names an unregistered owner",
            entry.work_item_id
        );
        let owner = &manifest.owners[&entry.owner];
        assert_eq!(
            entry.implementation_path, owner.path,
            "{} must map to its owner's canonical implementation path",
            entry.work_item_id
        );
        assert!(repo_path_exists(&entry.implementation_path));
        assert!(
            !entry.validation_proof.trim().is_empty() && !entry.removal_proof.trim().is_empty(),
            "{} must name both validation and removal proof",
            entry.work_item_id
        );
        let proof =
            format!("{} {}", entry.validation_proof, entry.removal_proof).to_ascii_lowercase();
        for incomplete in ["not verified", "not delivered", "scaffold only"] {
            assert!(
                !proof.contains(incomplete),
                "{} retains incomplete proof text: {incomplete}",
                entry.work_item_id
            );
        }
        used_owners.insert(&entry.owner);
    }
    let manifest_work_items: BTreeSet<&String> = manifest
        .entries
        .iter()
        .map(|entry| &entry.work_item_id)
        .collect();
    let expected_work_items: BTreeSet<&String> = expected.keys().collect();
    assert_eq!(manifest_work_items, expected_work_items);
    assert_eq!(
        manifest
            .entries
            .iter()
            .map(|entry| entry.spec_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        EXPECTED_PROVIDER_SPEC_COUNT
    );

    let registered_owners: BTreeSet<&String> = manifest.owners.keys().collect();
    assert_eq!(used_owners, registered_owners);
    for (name, owner) in &manifest.owners {
        assert!(
            name.starts_with("foundation.") || name.starts_with("provider."),
            "owner {name} is outside the foundation/Provider namespace"
        );
        assert!(
            owner.kind == "foundation" || owner.kind == "provider",
            "owner {name} has invalid kind {}",
            owner.kind
        );
        assert_eq!(
            name.starts_with("provider."),
            owner.kind == "provider",
            "owner {name} kind does not match its namespace"
        );
        assert!(
            repo_path_exists(&owner.path),
            "owner {name} points at missing path {}",
            owner.path
        );
        if owner.kind == "provider" {
            assert!(
                owner.path.starts_with("packages/d2b-provider-"),
                "Provider owner {name} must point at a canonical Provider package"
            );
            assert!(
                repo_path_exists(&format!("{}/Cargo.toml", owner.path)),
                "Provider owner {name} is missing Cargo.toml"
            );
        }
    }

    let serialized = read_repo_file(MANIFEST).to_ascii_lowercase();
    for forbidden in [
        "packages/xtask/src/delivery",
        "packages/xtask/src/attestation",
        "candidate-snapshot",
        "panel/",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "Wave 6 manifest contains out-of-scope process tooling: {forbidden}"
        );
    }
}
