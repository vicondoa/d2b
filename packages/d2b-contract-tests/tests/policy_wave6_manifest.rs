//! The merged Wave 6 manifest is a fail-closed product contract.
//!
//! The historical work-item register contains planned delivery material as
//! well as landed foundation and Provider work.  U9 consumes the landed
//! product slice only.  This test keeps that slice explicit: every entry has
//! one owner, every owner is registered once, and every registered owner points
//! at a current foundation or canonical Provider path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use serde::Deserialize;

const MANIFEST: &str = "docs/reference/wave6-foundation-manifest.json";
const EXPECTED_ENTRY_COUNT: usize = 68;

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

fn merged_source_work_items() -> BTreeMap<String, (String, String)> {
    let root = d2b_contract_tests::repo_root();
    let specs = root.join("docs/specs");
    let mut files = Vec::new();
    collect_markdown_files(&specs, &mut files);
    files.sort();

    let mut merged = BTreeMap::new();
    for path in files {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read specification {}: {err}", path.display()));
        let lines: Vec<&str> = content.lines().collect();
        if !lines.iter().any(|line| {
            line.strip_prefix("### ")
                .and_then(|heading| heading.split_whitespace().next())
                .is_some_and(|work_item_id| work_item_id.starts_with("ADR046-"))
        }) {
            continue;
        }
        let spec_id = lines
            .iter()
            .find_map(|line| table_value(line, "Spec ID"))
            .unwrap_or_else(|| panic!("specification {} is missing Spec ID", path.display()));
        let mut index = 0;
        while index < lines.len() {
            let Some(heading) = lines[index].strip_prefix("### ") else {
                index += 1;
                continue;
            };
            let work_item_id = heading.split_whitespace().next().unwrap_or_default();
            if !work_item_id.starts_with("ADR046-") {
                index += 1;
                continue;
            }

            let block_start = index + 1;
            let mut block_end = block_start;
            while block_end < lines.len() && !lines[block_end].starts_with("### ") {
                block_end += 1;
            }
            let is_merged = lines[block_start..block_end]
                .iter()
                .any(|line| table_value(line, "Implementation state").as_deref() == Some("Merged"));
            if is_merged {
                let spec_path = path
                    .strip_prefix(&root)
                    .expect("specification is below repository root")
                    .to_string_lossy()
                    .replace('\\', "/");
                assert!(
                    merged
                        .insert(
                            work_item_id.to_owned(),
                            (spec_id.clone(), spec_path.clone()),
                        )
                        .is_none(),
                    "merged source work item {work_item_id} appears more than once"
                );
            }
            index = block_end;
        }
    }
    merged
}

#[test]
fn wave6_manifest_maps_every_merged_entry_exactly_once() {
    assert!(repo_path_exists(MANIFEST), "missing {MANIFEST}");
    let manifest: Manifest =
        serde_json::from_str(&read_repo_file(MANIFEST)).expect("valid Wave 6 manifest JSON");

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.artifact_kind, "d2b-wave6-foundation-manifest");
    assert_eq!(manifest.source.adr, "0046");
    assert_eq!(
        manifest.source.selection,
        "merged-foundation-and-provider-work-items"
    );
    assert_eq!(
        manifest.entries.len(),
        EXPECTED_ENTRY_COUNT,
        "the merged Wave 6 product slice changed; update the contract with the landed entry set"
    );

    let mut work_items = BTreeSet::new();
    let mut used_owners = BTreeSet::new();
    for entry in &manifest.entries {
        assert!(!entry.work_item_id.is_empty());
        assert!(
            work_items.insert(&entry.work_item_id),
            "work item {} is mapped more than once",
            entry.work_item_id
        );
        assert!(!entry.spec_id.is_empty());
        assert!(!entry.spec_path.is_empty());
        assert!(
            repo_path_exists(&entry.spec_path),
            "entry {} points at missing specification {}",
            entry.work_item_id,
            entry.spec_path
        );
        assert_eq!(
            entry.status, "Merged",
            "U9 must not silently promote a planned work item"
        );
        used_owners.insert(&entry.owner);
        assert!(
            manifest.owners.contains_key(&entry.owner),
            "entry {} names an owner not present in the owner registry",
            entry.work_item_id
        );
    }

    let source_work_items = merged_source_work_items();
    let manifest_work_items: BTreeSet<&String> = manifest
        .entries
        .iter()
        .map(|entry| &entry.work_item_id)
        .collect();
    let source_work_item_ids: BTreeSet<&String> = source_work_items.keys().collect();
    assert_eq!(
        manifest_work_items, source_work_item_ids,
        "manifest must contain exactly the work items marked Merged in docs/specs"
    );
    for entry in &manifest.entries {
        let (source_spec_id, source_spec_path) = source_work_items
            .get(&entry.work_item_id)
            .unwrap_or_else(|| panic!("missing merged source for {}", entry.work_item_id));
        assert_eq!(
            &entry.spec_id, source_spec_id,
            "entry {} has a stale or incorrect specId",
            entry.work_item_id
        );
        assert_eq!(
            &entry.spec_path, source_spec_path,
            "entry {} has a stale or incorrect specPath",
            entry.work_item_id
        );
    }

    let registered_owners: BTreeSet<&String> = manifest.owners.keys().collect();
    assert_eq!(
        used_owners, registered_owners,
        "owner registry must have no orphaned or multiply-resolved owner keys"
    );

    for (name, owner) in &manifest.owners {
        assert!(
            name.starts_with("foundation.") || name.starts_with("provider."),
            "owner {name} is outside the foundation/Provider namespace"
        );
        assert!(
            owner.kind == "foundation" || owner.kind == "provider",
            "owner {name} has an invalid kind {}",
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
        "panel",
        "attestation",
        "seal",
        "candidate-snapshot",
        "ledger",
        "delivery",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "Wave 6 manifest contains out-of-scope process tooling: {forbidden}"
        );
    }
}
