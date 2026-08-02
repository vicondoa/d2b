//! Structural telemetry cardinality and redaction policy.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};

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
    let roots = [
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
        "packages/d2b-client/src",
        "packages/d2b/src",
    ];
    roots
        .into_iter()
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

#[test]
fn startup_tracing_and_v3_sources_do_not_leak_forbidden_fields() {
    let sources = destination_sources();
    assert!(
        sources
            .iter()
            .any(|(path, _)| path.contains("d2b-telemetry")),
        "telemetry foundation source must be present"
    );
    for (path, content) in &sources {
        for field in FORBIDDEN_SPAN_FIELDS {
            let tracing_field = format!("{field} =");
            let span_field = format!("\"{field}\"");
            assert!(
                !content.contains(&tracing_field) && !content.contains(&span_field),
                "{path} contains forbidden telemetry field {field}"
            );
        }
        assert!(
            !content.contains(r#"config_source = "realm-controllers""#),
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
            path.contains("audit")
                || path.contains("host.rs")
                || path.contains("zone_doctor.rs")
                || path.contains("zone_support_bundle.rs")
                || path.contains("redaction_guard.rs")
                || path.contains("provider-observability-otel/src/agent.rs"),
            "no_isolation leaked to a non-audit/status surface: {path}"
        );
    }
}
