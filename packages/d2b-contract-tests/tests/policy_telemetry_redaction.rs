//! Structural telemetry cardinality and redaction policy.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

const RESOURCE_ATTRIBUTES: &[&str] = &[
    "deployment.environment",
    "host.name",
    "service.name",
    "service.namespace",
    "source",
    "vm.env",
    "vm.name",
    "vm.role",
    "d2b.zone",
    "d2b.provider",
    "d2b.component",
    "service.version",
];

const FORBIDDEN_LABEL_KEYS: &[&str] = &[
    "vm",
    "zone",
    "zone_id",
    "zone_uid",
    "credential_name",
    "network",
    "network_name",
    "link_name_hash",
];

const FORBIDDEN_SPAN_FIELDS: &[&str] = &[
    "path",
    "socket",
    "argv",
    "env",
    "pid",
    "exe",
    "realm",
    "workload_id",
];

const SOURCE_ROOTS: &[&str] = &[
    "packages/d2b-telemetry/src",
    "packages/d2b-audit/src",
    "packages/d2b-provider-observability-otel/src",
    "packages/d2b-resource-store-redb/src",
    "packages/d2b-resource-api/src",
    "packages/d2b-core-controller/src",
    "packages/d2b-provider-supervisor/src",
    "packages/d2b-provider-system-core/src",
    "packages/d2b-session/src",
    "packages/d2b-bus/src",
    "packages/d2b-resource-client/src",
    "packages/d2b/src",
];

// These are named format arguments, not telemetry fields. Each exemption is
// exact and stale-checked below; a whole file or source-root exemption would
// make it too easy to hide a new field.
const FIELD_ASSIGNMENT_EXEMPTIONS: &[(&str, &str)] =
    &[("packages/d2b/src/doctor.rs", "path = parsed.path,")];

#[derive(Debug, Clone, Copy)]
struct LiteralExemption {
    path: &'static str,
    snippet: &'static str,
    occurrences: usize,
}

const LITERAL_FIELD_EXEMPTIONS: &[LiteralExemption] = &[
    // The following source paths deliberately name forbidden fields in
    // redaction predicates or negative tests. They are line-scoped below.
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"path\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"socket\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"argv\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"env\",",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"pid\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"exe\",",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"realm\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b-telemetry/src/redaction_guard.rs",
        snippet: "\"workload_id\",",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-resource-store-redb/src/tracing.rs",
        snippet: "[(\"path\", \"/tmp\")]",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-provider-supervisor/src/tracing.rs",
        snippet: "[(\"pid\", \"1\")]",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-audit/src/export.rs",
        snippet: "contains(\"\\\"realm\\\"\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-audit/src/record_types.rs",
        snippet: "matches!(key.as_str(), \"realm\" | \"node\" | \"workload_id\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-audit/src/record_types.rs",
        snippet: "value.get(\"realm\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-audit/src/record_types.rs",
        snippet: "value.get(\"workload_id\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-resource-store-redb/src/audit.rs",
        snippet: "contains(\"\\\"realm\\\"\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-core-controller/src/owner_reconcile.rs",
        snippet: "target(\"work\", \"Endpoint\", \"socket\", 4)",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-session/src/audit.rs",
        snippet: "value.get(\"realm\")",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b-bus/src/transport/unix.rs",
        snippet: "[\"std::\", \"env\"].concat()",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/endpoint.rs",
        snippet: "\"path\",",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/endpoint.rs",
        snippet: "\"socket\",",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/resource.rs",
        snippet: "label_selector: Some(\"env\"),",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/share.rs",
        snippet: "\"path\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b/src/share.rs",
        snippet: "\"socket\",",
        occurrences: 2,
    },
    LiteralExemption {
        path: "packages/d2b/src/dispatch.rs",
        snippet: "[\"d2b\", \"realm\", \"list\"]",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/lib.rs",
        snippet: "[\"d2b\", \"realm\", \"enter\", \"work\"]",
        occurrences: 1,
    },
    LiteralExemption {
        path: "packages/d2b/src/lib.rs",
        snippet: "[\"d2b\", \"realm\", \"run\", \"work\", \"--\", \"d2b\", \"vm\", \"list\"]",
        occurrences: 1,
    },
];

const NO_ISOLATION_SOURCES: &[&str] = &[
    "packages/d2b-audit/src/export.rs",
    "packages/d2b-audit/src/record_types.rs",
    "packages/d2b-audit/src/segment.rs",
    "packages/d2b-audit/src/sink.rs",
    "packages/d2b-provider-observability-otel/src/agent.rs",
    "packages/d2b-provider-system-core/src/host.rs",
    "packages/d2b-provider-system-core/src/host_process_audit.rs",
    "packages/d2b-provider-system-core/src/host_status.rs",
    "packages/d2b-session/src/audit.rs",
    "packages/d2b-telemetry/src/redaction_guard.rs",
    "packages/d2b/src/context.rs",
    "packages/d2b/src/zone_doctor.rs",
    "packages/d2b/src/zone_support_bundle.rs",
];

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

fn destination_sources() -> Vec<(String, String)> {
    SOURCE_ROOTS
        .iter()
        .filter(|root| repo_path_exists(root))
        .flat_map(|root| {
            rust_files(&repo_root().join(root))
                .into_iter()
                .filter_map(|path| {
                    let rel = path
                        .strip_prefix(repo_root())
                        .ok()?
                        .to_string_lossy()
                        .into_owned();
                    let content = fs::read_to_string(path).ok()?;
                    Some((rel, content))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn mask_non_code(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;

    let mask = |masked: &mut [u8], start: usize, end: usize| {
        for byte in &mut masked[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };

    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            let start = index;
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            mask(&mut masked, start, index);
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth != 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            mask(&mut masked, start, index);
            continue;
        }
        if bytes[index] == b'r' {
            let mut cursor = index + 1;
            while cursor < bytes.len() && bytes[cursor] == b'#' {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'"' {
                let hashes = cursor - index - 1;
                let close = {
                    let mut search = cursor + 1;
                    let mut found = None;
                    while search < bytes.len() {
                        if bytes[search] == b'"'
                            && bytes
                                .get(search + 1..search + 1 + hashes)
                                .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
                        {
                            found = Some(search + 1 + hashes);
                            break;
                        }
                        search += 1;
                    }
                    found.unwrap_or(bytes.len())
                };
                mask(&mut masked, index, close);
                index = close;
                continue;
            }
        }
        if bytes[index] == b'"' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            mask(&mut masked, start, index);
            continue;
        }
        index += 1;
    }

    String::from_utf8(masked).expect("masking source must preserve UTF-8")
}

fn line_number(content: &str, offset: usize) -> usize {
    content[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn exempt_field_assignment(rel: &str, line: &str) -> bool {
    FIELD_ASSIGNMENT_EXEMPTIONS
        .iter()
        .any(|(path, snippet)| *path == rel && line.contains(snippet))
}

fn exempt_literal_field(rel: &str, line: &str) -> bool {
    LITERAL_FIELD_EXEMPTIONS
        .iter()
        .any(|exemption| exemption.path == rel && line.contains(exemption.snippet))
}

fn quoted_field_occurrences(content: &str) -> Vec<(usize, String)> {
    let bytes = content.as_bytes();
    let mut occurrences = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth != 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b'r' {
            let mut cursor = index + 1;
            while cursor < bytes.len() && bytes[cursor] == b'#' {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'"' {
                let hashes = cursor - index - 1;
                index = cursor + 1;
                while index < bytes.len() {
                    if bytes[index] == b'"'
                        && bytes
                            .get(index + 1..index + 1 + hashes)
                            .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
                    {
                        index += 1 + hashes;
                        break;
                    }
                    index += 1;
                }
                continue;
            }
        }
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        let mut value = Vec::new();
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == b'"' {
                index += 1;
                break;
            }
            value.push(bytes[index]);
            index += 1;
        }
        let mut before = start;
        while before > 0 && bytes[before - 1].is_ascii_whitespace() {
            before -= 1;
        }
        let mut after = index;
        while after < bytes.len() && bytes[after].is_ascii_whitespace() {
            after += 1;
        }
        if (before == 0 || matches!(bytes[before - 1], b'(' | b'[' | b'{' | b','))
            && matches!(
                bytes.get(after),
                Some(b'=') | Some(b',') | Some(b')') | Some(b']') | Some(b'}') | Some(b'|')
            )
            && String::from_utf8(value)
                .ok()
                .is_some_and(|value| FORBIDDEN_SPAN_FIELDS.contains(&value.as_str()))
        {
            occurrences.push((start, "quoted field".to_owned()));
        }
    }
    occurrences
}

fn forbidden_field_violations(rel: &str, content: &str) -> Vec<String> {
    let masked = mask_non_code(content);
    let assignment = Regex::new(
        r#"(?m)(?:^|[,({])\s*(?:[?%])?(path|socket|argv|env|pid|exe|realm|workload_id)\s*="#,
    )
    .expect("valid telemetry field assignment regex");
    let shorthand =
        Regex::new(r#"(?m)(?:^|[,({])\s*[?%](path|socket|argv|env|pid|exe|realm|workload_id)\b"#)
            .expect("valid telemetry field shorthand regex");
    let mut violations = BTreeSet::new();

    for captures in assignment.captures_iter(&masked) {
        let field_capture = captures.get(1).expect("field capture");
        let field = field_capture.as_str();
        let line = line_number(content, field_capture.start());
        let source_line = content.lines().nth(line - 1).unwrap_or_default();
        if !exempt_field_assignment(rel, source_line) {
            violations.insert(format!("{rel}:{line}: forbidden telemetry field `{field}`"));
        }
    }
    for captures in shorthand.captures_iter(&masked) {
        let field_capture = captures.get(1).expect("field capture");
        let field = field_capture.as_str();
        let line = line_number(content, field_capture.start());
        let source_line = content.lines().nth(line - 1).unwrap_or_default();
        if !exempt_field_assignment(rel, source_line) {
            violations.insert(format!("{rel}:{line}: forbidden telemetry field `{field}`"));
        }
    }
    for (offset, _) in quoted_field_occurrences(content) {
        let line = line_number(content, offset);
        let source_line = content.lines().nth(line - 1).unwrap_or_default();
        if !exempt_literal_field(rel, source_line) {
            violations.insert(format!("{rel}:{line}: quoted forbidden telemetry field"));
        }
    }

    violations.into_iter().collect()
}

fn contains_retired_config_source(content: &str) -> bool {
    Regex::new(r#"config_source\s*=\s*"realm-controllers""#)
        .expect("valid retired config source regex")
        .is_match(content)
}

fn assert_field_exemptions_are_live(sources: &[(String, String)]) {
    for (path, snippet) in FIELD_ASSIGNMENT_EXEMPTIONS {
        let matches = sources
            .iter()
            .filter(|(rel, content)| {
                rel == path
                    && content
                        .lines()
                        .filter(|line| line.contains(snippet))
                        .count()
                        == 1
            })
            .count();
        assert_eq!(
            matches, 1,
            "telemetry field exemption {path:?} / {snippet:?} must match exactly one source line"
        );
    }
    for exemption in LITERAL_FIELD_EXEMPTIONS {
        let matches = sources
            .iter()
            .filter(|(rel, content)| {
                rel == exemption.path
                    && content
                        .lines()
                        .filter(|line| line.contains(exemption.snippet))
                        .count()
                        == exemption.occurrences
            })
            .count();
        assert_eq!(
            matches, 1,
            "telemetry literal exemption {:?} must match exactly {} source lines",
            exemption, exemption.occurrences
        );
    }
}

#[test]
fn startup_tracing_and_v3_sources_do_not_leak_forbidden_fields() {
    let sources = destination_sources();
    assert!(
        sources.len() >= 8,
        "telemetry source inventory is unexpectedly empty or narrowed"
    );
    for root in SOURCE_ROOTS {
        if repo_path_exists(root) {
            assert!(
                sources
                    .iter()
                    .any(|(path, _)| path.starts_with(&format!("{root}/"))),
                "telemetry source root {root} exists but contributed no Rust source"
            );
        }
    }
    assert!(
        sources
            .iter()
            .any(|(path, _)| path.contains("d2b-telemetry")),
        "telemetry foundation source must be present"
    );
    assert_field_exemptions_are_live(&sources);
    let violations = sources
        .iter()
        .flat_map(|(path, content)| forbidden_field_violations(path, content))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "forbidden telemetry fields found in source:\n{}",
        violations.join("\n")
    );
    for (path, content) in &sources {
        assert!(
            !contains_retired_config_source(content),
            "{path} contains the retired realm-controllers tracing source"
        );
    }
}

#[test]
fn resource_attribute_allowlist_is_closed() {
    let telemetry = read_repo_file("packages/d2b-telemetry/src/metric_label_policy.rs");
    for attribute in RESOURCE_ATTRIBUTES {
        assert!(
            telemetry.contains(&format!("\"{attribute}\"")),
            "telemetry resource-attribute allowlist is missing {attribute}"
        );
    }
    assert!(!telemetry.contains("\"config_source\""));
}

#[test]
fn metric_label_policy_rejects_identity_keys_and_suffixes() {
    let telemetry = read_repo_file("packages/d2b-telemetry/src/metric_label_policy.rs");
    for key in FORBIDDEN_LABEL_KEYS {
        assert!(
            telemetry.contains(&format!("\"{key}\"")),
            "forbidden metric key {key} is not pinned"
        );
    }
    for suffix in ["_name", "_name_hash", "_name_digest", "_uid"] {
        assert!(
            telemetry.contains(&format!("\"{suffix}\"")),
            "forbidden metric suffix {suffix} is not pinned"
        );
    }
    for key in ["vm", "zone", "zone_id", "zone_uid"] {
        assert!(
            !telemetry.contains(&format!("label(\"{key}\"")),
            "metric descriptor emits forbidden key {key}"
        );
    }
}

#[test]
fn no_isolation_is_confined_to_audit_and_status_surfaces() {
    let sources = destination_sources();
    let mut observed = BTreeSet::new();
    for (path, content) in sources {
        if content.contains("no_isolation") {
            observed.insert(path);
        }
    }
    assert!(
        observed.iter().any(|path| path.contains("audit")),
        "no_isolation must be represented by an audit/status destination"
    );
    for path in observed {
        assert!(
            NO_ISOLATION_SOURCES.contains(&path.as_str()),
            "no_isolation leaked to a non-audit/status surface: {path}"
        );
    }
}

#[test]
fn telemetry_field_scan_is_multiline_and_comment_safe() {
    let multiline = r#"tracing::warn!(
        path
            = %path,
    );"#;
    assert_eq!(forbidden_field_violations("fixture.rs", multiline).len(), 1);

    let quoted = "event!(\n    \"socket\"\n        = socket_value,\n);";
    assert_eq!(forbidden_field_violations("fixture.rs", quoted).len(), 1);

    let ordinary_code = r#"
        // path = must not count
        let path = "/tmp";
        let json = "{\"socket\": \"redacted\"}";
    "#;
    assert!(
        forbidden_field_violations("fixture.rs", ordinary_code).is_empty(),
        "ordinary code and comments must not become false telemetry findings"
    );

    assert!(contains_retired_config_source(
        "tracing::info!(config_source\n    = \"realm-controllers\");"
    ));
}
