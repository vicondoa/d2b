//! Broker request-disposition policy lint, migrated from
//! `tests/broker-enum-disposition.sh`.
//!
//! The legacy shell gate cross-referenced
//! `docs/reference/broker-w2-dispositions.md`,
//! `docs/reference/schemas/v2/privileges.json`, and the broker dispatcher in
//! `packages/nixling-priv-broker/src/runtime.rs`. This Rust port keeps the
//! same closed-world assertions while evaluating the production real-wire
//! dispatcher (`RealBrokerRequest::...`) instead of the retired bootstrap
//! dispatcher.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nixling_contract_tests::{read_repo_file, repo_root};
use regex::Regex;
use serde_json::Value;

const DOC_REL: &str = "docs/reference/broker-w2-dispositions.md";
const SCHEMA_REL: &str = "docs/reference/schemas/v2/privileges.json";
const BROKER_SRC_REL: &str = "packages/nixling-priv-broker/src";
const RUNTIME_REL: &str = "packages/nixling-priv-broker/src/runtime.rs";

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| {
        panic!(
            "broker-enum-disposition: cannot read {}: {err}",
            dir.display()
        )
    });
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("target") {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn read_lossy(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|err| {
        panic!(
            "broker-enum-disposition: cannot read {}: {err}",
            path.display()
        )
    });
    String::from_utf8_lossy(&bytes).into_owned()
}

fn broker_source_tree() -> String {
    let mut files = Vec::new();
    collect_rs_files(&repo_root().join(BROKER_SRC_REL), &mut files);
    files.sort();
    files
        .iter()
        .map(|path| read_lossy(path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_doc_rows() -> BTreeMap<String, String> {
    let doc = read_repo_file(DOC_REL);
    let mut rows = BTreeMap::new();
    let mut in_table = false;

    for raw_line in doc.lines() {
        let line = raw_line.trim();
        if line.starts_with("| Variant |") {
            in_table = true;
            continue;
        }
        if in_table && !line.starts_with('|') {
            break;
        }
        if !in_table || line.starts_with("| ---") {
            continue;
        }

        let parts: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(
            parts.len(),
            4,
            "broker-enum-disposition: malformed table row: {raw_line}"
        );
        let variant = parts[0].to_owned();
        let disposition = parts[1].to_owned();
        assert!(
            rows.insert(variant.clone(), disposition).is_none(),
            "broker-enum-disposition: duplicate table row for {variant}"
        );
    }

    assert!(
        !rows.is_empty(),
        "broker-enum-disposition: no rows parsed from {DOC_REL}"
    );
    rows
}

fn schema_variants() -> BTreeSet<String> {
    let schema: Value = serde_json::from_str(&read_repo_file(SCHEMA_REL))
        .expect("broker-enum-disposition: privileges schema is valid JSON");
    let enum_values = schema
        .pointer("/definitions/OperationAuthz/properties/operation/enum")
        .and_then(Value::as_array)
        .expect("broker-enum-disposition: OperationAuthz.operation.enum is present");

    let mut expected: BTreeSet<String> = enum_values
        .iter()
        .filter_map(Value::as_str)
        .filter(|op| matches!(op.as_bytes().first(), Some(b'A'..=b'Z')))
        .map(str::to_owned)
        .collect();
    expected.insert("Hello".to_owned());
    expected
}

fn real_dispatcher_source() -> String {
    let runtime = read_repo_file(RUNTIME_REL);
    let start_marker = "use nixling_ipc::broker_wire::BrokerRequest as RealBrokerRequest;";
    let start = runtime
        .find(start_marker)
        .expect("broker-enum-disposition: real-wire dispatcher alias not found in runtime.rs");
    let rest = &runtime[start..];
    let end_marker = "\n#[derive(Clone, Copy)]\nstruct AuditBundleMetadata";
    let end = rest
        .find(end_marker)
        .expect("broker-enum-disposition: real-wire dispatcher end marker not found");
    rest[..end].to_owned()
}

fn dispatcher_arm_segments() -> BTreeMap<String, String> {
    let dispatch = real_dispatcher_source();
    let arm_re = Regex::new(r"(?m)^\s*RealBrokerRequest::([A-Z][A-Za-z0-9]+)\b")
        .expect("valid dispatcher-arm regex");
    let matches: Vec<(String, usize)> = arm_re
        .captures_iter(&dispatch)
        .map(|cap| {
            (
                cap.get(1).expect("variant capture").as_str().to_owned(),
                cap.get(0).expect("whole match").start(),
            )
        })
        .collect();
    assert!(
        !matches.is_empty(),
        "broker-enum-disposition: no real-wire dispatcher arms found in {RUNTIME_REL}"
    );

    let mut segments = BTreeMap::new();
    for (idx, (variant, start)) in matches.iter().enumerate() {
        let end = matches
            .get(idx + 1)
            .map_or(dispatch.len(), |(_, next_start)| *next_start);
        assert!(
            segments
                .insert(variant.clone(), dispatch[*start..end].to_owned())
                .is_none(),
            "broker-enum-disposition: duplicate dispatcher arm for {variant}"
        );
    }
    segments
}

#[test]
fn broker_disposition_doc_matches_schema_and_dispatcher() {
    let rows = parse_doc_rows();
    let expected = schema_variants();
    let row_set: BTreeSet<String> = rows.keys().cloned().collect();

    let missing: Vec<String> = expected.difference(&row_set).cloned().collect();
    let unexpected: Vec<String> = row_set.difference(&expected).cloned().collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "broker-enum-disposition: doc rows must equal OperationAuthz.operation enum \
         plus Hello; missing rows: {missing:?}; unexpected rows: {unexpected:?}"
    );

    let source_tree = broker_source_tree();
    let arm_segments = dispatcher_arm_segments();

    for (variant, disposition) in rows {
        assert!(
            source_tree.contains(&variant),
            "broker-enum-disposition: {variant} never appears under {BROKER_SRC_REL}"
        );

        let segment = arm_segments.get(&variant);
        match disposition.as_str() {
            "callable-read-only" => {
                let segment = segment.unwrap_or_else(|| {
                    panic!("broker-enum-disposition: callable variant {variant} is missing a dispatcher arm")
                });
                assert!(
                    !segment.contains("BrokerError::Unimplemented"),
                    "broker-enum-disposition: callable variant {variant} still routes to \
                     BrokerError::Unimplemented"
                );
            }
            "stubbed-unimplemented" => {
                let segment = segment.unwrap_or_else(|| {
                    panic!("broker-enum-disposition: stubbed variant {variant} is missing a dispatcher arm")
                });
                assert!(
                    segment.contains("BrokerError::Unimplemented"),
                    "broker-enum-disposition: stubbed variant {variant} does not return \
                     BrokerError::Unimplemented"
                );
            }
            "stubbed-unknown-operation" => {
                let segment = segment.unwrap_or_else(|| {
                    panic!(
                        "broker-enum-disposition: stubbed-unknown-operation variant {variant} \
                         is missing a dispatcher arm"
                    )
                });
                assert!(
                    segment.contains("BrokerError::UnknownOperation"),
                    "broker-enum-disposition: stubbed-unknown-operation variant {variant} \
                     does not return BrokerError::UnknownOperation"
                );
            }
            "compile-time-only" => {
                assert!(
                    segment.is_none(),
                    "broker-enum-disposition: compile-time-only variant {variant} reached \
                     the wire dispatcher"
                );
            }
            "promoted-live" => {
                let segment = segment.unwrap_or_else(|| {
                    panic!(
                        "broker-enum-disposition: promoted-live variant {variant} is missing \
                         a dispatcher arm"
                    )
                });
                assert!(
                    !segment.contains("BrokerError::Unimplemented"),
                    "broker-enum-disposition: promoted-live variant {variant} still routes \
                     to BrokerError::Unimplemented"
                );
                assert!(
                    !segment.contains("BrokerError::UnknownOperation"),
                    "broker-enum-disposition: promoted-live variant {variant} still routes \
                     to BrokerError::UnknownOperation"
                );
            }
            other => panic!("broker-enum-disposition: unknown disposition '{other}' for {variant}"),
        }
    }
}
