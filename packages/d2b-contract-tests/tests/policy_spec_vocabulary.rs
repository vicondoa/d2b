//! Vocabulary and authoritative-spec policy.
//!
//! The surviving ADR 0046 specifications use the current Zone/ResourceType
//! vocabulary while explicitly mapping legacy Realm/Workload evidence.  This
//! policy checks that translation boundary and the qualified ResourceType
//! grammar without recreating the retired spec registry or implementation
//! graph.

use std::{
    fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::{repo_path_exists, repo_root};
use regex::Regex;

const OBSOLETE_SPEC_ARTIFACTS: &[&str] = &[
    "docs/specs/ADR-046-streamline.md",
    "docs/specs/ADR-046-spec-set.json",
    "docs/specs/ADR-046-work-items.json",
    "docs/specs/ADR-046-implementation-graph.json",
    "docs/specs/ADR-046-implementation-graph.md",
];

fn spec_markdown_files() -> Vec<(String, PathBuf)> {
    let root = repo_root().join("docs/specs");
    let mut paths = Vec::new();
    collect_markdown(&root, &mut paths);
    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(repo_root())
                .expect("spec path is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, path)
        })
        .collect()
}

fn collect_markdown(root: &Path, paths: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).expect("read specification directory");
    for entry in entries {
        let entry = entry.expect("read specification entry");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, paths);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            paths.push(path);
        }
    }
}

fn resource_type_context(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "resourcetype",
        "resource type",
        "servicetype",
        "bindingtype",
        "expectedservicetype",
        "qualified type",
        "type =",
        "type:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn qualified_type_violations(path: &str, source: &str) -> Vec<String> {
    let candidates = Regex::new(r"[A-Za-z0-9-]+\.d2bus\.org\.[A-Za-z0-9-]+")
        .expect("valid qualified ResourceType regex");
    let valid = Regex::new(r"^[a-z][a-z0-9-]*\.d2bus\.org\.[A-Z][A-Za-z0-9]*$")
        .expect("valid ResourceType grammar");
    let mut violations = Vec::new();
    for (line_number, line) in source.lines().enumerate() {
        if !resource_type_context(line) {
            continue;
        }
        for candidate in candidates.find_iter(line) {
            if !valid.is_match(candidate.as_str()) {
                violations.push(format!(
                    "{path}:{}: invalid qualified ResourceType `{}`",
                    line_number + 1,
                    candidate.as_str()
                ));
            }
        }
    }
    violations
}

fn public_host_identity_field_violations(path: &str, source: &str) -> Vec<String> {
    let field = Regex::new(r#"^\s*(?:[-*]\s*)?["']?(hostUid|hostGid)["']?\s*:"#)
        .expect("valid public host identity field regex");
    source
        .lines()
        .enumerate()
        .filter_map(|(line_number, line)| {
            field.captures(line).map(|capture| {
                format!("{path}:{}: public field `{}`", line_number + 1, &capture[1])
            })
        })
        .collect()
}

fn source_vocabulary_violations(path: &str, source: &str) -> Vec<String> {
    let field = Regex::new(r"\b(?:resourceKind|hostUid|hostGid|host_uid|host_gid)\s*:")
        .expect("valid source vocabulary regex");
    source
        .lines()
        .enumerate()
        .filter_map(|(line_number, line)| {
            let code = line.split("//").next().unwrap_or_default();
            (code.contains("ResourceKind") || field.is_match(code)).then_some(format!(
                "{path}:{}: forbidden source vocabulary",
                line_number + 1
            ))
        })
        .collect()
}

fn rust_source_files() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    collect_rust(
        &repo_root().join("packages/d2b-contracts/src/v3"),
        &mut files,
    );
    files
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(repo_root())
                .expect("source path is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            (name, path)
        })
        .collect()
}

fn collect_rust(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).expect("read source directory");
    for entry in entries {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            collect_rust(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn authoritative_specs_use_the_frozen_resource_type_grammar() {
    let violations = spec_markdown_files()
        .into_iter()
        .flat_map(|(relative, path)| {
            let source = fs::read_to_string(path).expect("read specification");
            qualified_type_violations(&relative, &source)
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "qualified ResourceType vocabulary drifted:\n{}",
        violations.join("\n")
    );
}

#[test]
fn authoritative_specs_do_not_publish_host_uid_or_gid_fields() {
    let violations = spec_markdown_files()
        .into_iter()
        .flat_map(|(relative, path)| {
            let source = fs::read_to_string(path).expect("read specification");
            public_host_identity_field_violations(&relative, &source)
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "public host identity fields appeared in specifications:\n{}",
        violations.join("\n")
    );
}

#[test]
fn current_v3_sources_use_resource_type_and_opaque_identity_vocabulary() {
    let violations = rust_source_files()
        .into_iter()
        .flat_map(|(relative, path)| {
            let source = fs::read_to_string(path).expect("read v3 source");
            source_vocabulary_violations(&relative, &source)
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "current v3 source vocabulary drifted:\n{}",
        violations.join("\n")
    );
}

#[test]
fn terminology_and_migration_maps_are_the_single_translation_authority() {
    let terminology =
        fs::read_to_string(repo_root().join("docs/specs/ADR-046-terminology-and-identities.md"))
            .expect("read terminology specification");
    let migration =
        fs::read_to_string(repo_root().join("docs/specs/ADR-046-current-code-migration-map.md"))
            .expect("read current-code migration map");
    assert!(terminology.contains("`Realm` remains current v3 baseline terminology"));
    assert!(terminology.contains("3.0 public schemas, CLI, APIs, errors, and docs use `Zone`"));
    assert!(migration.contains("Canonical Terminology Mapping"));
    assert!(migration.contains("`Workload` / `WorkloadId`"));
    assert!(migration.contains("`Guest` ResourceType"));
}

#[test]
fn obsolete_spec_task_graph_artifacts_are_not_reintroduced() {
    for path in OBSOLETE_SPEC_ARTIFACTS {
        assert!(
            !repo_path_exists(path),
            "retired spec/task graph artifact must not become a parallel authority: {path}"
        );
    }
    let index = fs::read_to_string(repo_root().join("docs/specs/README.md"))
        .expect("read specification index");
    assert!(!index.contains("ADR-046-streamline.md"));
    assert!(!index.contains("implementation-graph"));
}

#[test]
fn malformed_qualified_types_and_public_identity_fields_are_rejected() {
    let type_violations = qualified_type_violations(
        "fixture.md",
        "`type = \"Acme.d2bus.org.Widget\"` and `type = \"acme.d2bus.org.widget\"`",
    );
    assert_eq!(type_violations.len(), 2);
    let field_violations =
        public_host_identity_field_violations("fixture.md", "\"hostUid\": 1000\n  hostGid: 1000\n");
    assert_eq!(field_violations.len(), 2);
    assert_eq!(
        source_vocabulary_violations("fixture.rs", "pub resourceKind: String;"),
        vec!["fixture.rs:1: forbidden source vocabulary"]
    );
}
