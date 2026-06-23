//! Daemon metrics descriptor-inventory policy lint, migrated from
//! `tests/daemon-metrics-eval.sh`.
//!
//! The retired bash gate was a pure source/doc parser: it did not start the
//! daemon or assert runtime series. This Rust port keeps the same shape by
//! reading the real checkout via `nixling_contract_tests` repo-file helpers and
//! checking the canonical `METRIC_INVENTORY` table in
//! `packages/nixlingd/src/metrics.rs`.

use std::collections::BTreeSet;

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

const METRICS_SRC: &str = "packages/nixlingd/src/metrics.rs";
const METRICS_DOC: &str = "docs/reference/daemon-metrics.md";

#[derive(Debug, Clone, Copy)]
struct ExpectedMetric {
    name: &'static str,
    kind: &'static str,
    labels: &'static [&'static str],
    buckets_expr: &'static str,
    bucket_values: Option<&'static [f64]>,
}

const EXPECTED_METRICS: &[ExpectedMetric] = &[
    ExpectedMetric {
        name: "nixling_daemon_vm_state",
        kind: "Gauge",
        labels: &["vm", "state"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_vm_start_duration_seconds",
        kind: "Histogram",
        labels: &["vm", "outcome"],
        buckets_expr: "VM_START_BUCKETS_SECONDS",
        bucket_values: Some(&[0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0]),
    },
    ExpectedMetric {
        name: "nixling_daemon_host_prep_step_duration_seconds",
        kind: "Histogram",
        labels: &["step"],
        buckets_expr: "HOST_PREP_STEP_BUCKETS_SECONDS",
        bucket_values: Some(&[0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
    },
    ExpectedMetric {
        name: "nixling_daemon_broker_request_total",
        kind: "Counter",
        labels: &["op", "outcome"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_broker_request_duration_seconds",
        kind: "Histogram",
        labels: &["op"],
        buckets_expr: "BROKER_REQUEST_BUCKETS_SECONDS",
        bucket_values: Some(&[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
    },
    ExpectedMetric {
        name: "nixling_daemon_vm_shutdown_total",
        kind: "Counter",
        labels: &["vm", "vmm", "outcome"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_vm_shutdown_duration_seconds",
        kind: "Histogram",
        labels: &["vm", "vmm", "outcome"],
        buckets_expr: "VM_SHUTDOWN_BUCKETS_SECONDS",
        bucket_values: Some(&[
            0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 90.0, 120.0, 300.0, 600.0,
        ]),
    },
    ExpectedMetric {
        name: "nixling_daemon_ownership_drift_total",
        kind: "Counter",
        labels: &["vm"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_ssh_host_key_drift_total",
        kind: "Counter",
        labels: &["vm"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_pidfd_table_size",
        kind: "Gauge",
        labels: &[],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_uptime_seconds",
        kind: "Gauge",
        labels: &[],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_guest_control_exec_total",
        kind: "Counter",
        labels: &["subsystem", "outcome", "error_kind"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
    ExpectedMetric {
        name: "nixling_daemon_guest_control_shell_total",
        kind: "Counter",
        labels: &["subsystem", "outcome", "error_kind"],
        buckets_expr: "&[]",
        bucket_values: None,
    },
];

const RETIRED_DOC_METRIC_COUNT: usize = EXPECTED_METRICS.len();

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMetric {
    name: String,
    kind: String,
    labels: Vec<String>,
    buckets_expr: String,
}

fn metric_inventory_body(src: &str) -> &str {
    let inventory_re =
        Regex::new(r#"(?s)pub const METRIC_INVENTORY:\s*&\[MetricDescriptor\]\s*=\s*&\[(.*?)\];"#)
            .expect("valid METRIC_INVENTORY regex");
    let caps = inventory_re
        .captures(src)
        .unwrap_or_else(|| panic!("daemon-metrics-eval: {METRICS_SRC} missing METRIC_INVENTORY"));
    caps.get(1).expect("METRIC_INVENTORY body capture").as_str()
}

fn parse_inventory(src: &str) -> Vec<ParsedMetric> {
    let descriptor_re = Regex::new(
        r#"(?s)MetricDescriptor\s*\{\s*name:\s*"(?P<name>[A-Za-z0-9_]+)"\s*,\s*kind:\s*MetricKind::(?P<kind>Counter|Gauge|Histogram)\s*,\s*labels:\s*&\[(?P<labels>[^\]]*)\]\s*,\s*buckets_seconds:\s*(?P<buckets>[^,]+)\s*,\s*\}"#,
    )
    .expect("valid MetricDescriptor regex");

    descriptor_re
        .captures_iter(metric_inventory_body(src))
        .map(|caps| {
            let labels_raw = caps.name("labels").expect("labels capture").as_str().trim();
            let labels = if labels_raw.is_empty() {
                Vec::new()
            } else {
                labels_raw
                    .split(',')
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .map(|label| label.trim_matches('"').to_string())
                    .collect()
            };
            ParsedMetric {
                name: caps
                    .name("name")
                    .expect("name capture")
                    .as_str()
                    .to_string(),
                kind: caps
                    .name("kind")
                    .expect("kind capture")
                    .as_str()
                    .to_string(),
                labels,
                buckets_expr: caps
                    .name("buckets")
                    .expect("buckets capture")
                    .as_str()
                    .trim()
                    .to_string(),
            }
        })
        .collect()
}

fn extract_bucket_const(src: &str, const_name: &str) -> Option<Vec<f64>> {
    let const_re = Regex::new(&format!(
        r"(?s)pub const {}:\s*&\[f64\]\s*=\s*&\[(.*?)\];",
        regex::escape(const_name)
    ))
    .expect("valid bucket-constant regex");
    const_re.captures(src).map(|caps| {
        caps.get(1)
            .expect("bucket constant body capture")
            .as_str()
            .split(',')
            .map(str::trim)
            .filter(|tok| !tok.is_empty())
            .map(|tok| {
                    tok.parse::<f64>().unwrap_or_else(|err| {
                        panic!(
                            "daemon-metrics-eval: bucket token {tok:?} in {const_name} is not f64: {err}"
                        )
                    })
            })
            .collect()
    })
}

fn doc_section<'a>(doc: &'a str, metric_name: &str) -> Option<&'a str> {
    let marker = format!("### `{metric_name}`");
    let (_, rest) = doc.split_once(&marker)?;
    Some(
        rest.split("### `")
            .next()
            .expect("split always yields first"),
    )
}

fn labels_doc_line(labels: &[&str]) -> String {
    labels
        .iter()
        .map(|label| format!("`{label}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn daemon_metrics_inventory_descriptors_match_policy() {
    assert!(
        repo_path_exists(METRICS_SRC),
        "daemon-metrics-eval: missing required file: {METRICS_SRC}"
    );
    let src = read_repo_file(METRICS_SRC);
    let descriptors = parse_inventory(&src);
    assert!(
        !descriptors.is_empty(),
        "daemon-metrics-eval: no MetricDescriptor entries found in METRIC_INVENTORY"
    );

    let names: Vec<&str> = descriptors
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect();
    let expected_names: Vec<&str> = EXPECTED_METRICS.iter().map(|metric| metric.name).collect();
    assert_eq!(
        names, expected_names,
        "daemon-metrics-eval: METRIC_INVENTORY names/order changed"
    );

    let mut seen = BTreeSet::new();
    for name in &names {
        assert!(
            seen.insert(*name),
            "daemon-metrics-eval: duplicate MetricDescriptor for {name}"
        );
    }

    for (descriptor, expected) in descriptors.iter().zip(EXPECTED_METRICS) {
        assert_eq!(
            descriptor.kind, expected.kind,
            "daemon-metrics-eval: src {} kind mismatch",
            expected.name
        );
        let labels: Vec<&str> = descriptor.labels.iter().map(String::as_str).collect();
        assert_eq!(
            labels.as_slice(),
            expected.labels,
            "daemon-metrics-eval: src {} labels mismatch",
            expected.name
        );
        assert_eq!(
            descriptor.buckets_expr, expected.buckets_expr,
            "daemon-metrics-eval: src {} buckets expression mismatch",
            expected.name
        );
    }
}

#[test]
fn daemon_metrics_histogram_buckets_match_policy() {
    assert!(
        repo_path_exists(METRICS_SRC),
        "daemon-metrics-eval: missing required file: {METRICS_SRC}"
    );
    let src = read_repo_file(METRICS_SRC);

    for expected in EXPECTED_METRICS
        .iter()
        .filter(|metric| metric.kind == "Histogram")
    {
        let expected_values = expected
            .bucket_values
            .expect("histogram metrics carry bucket values");
        let got = extract_bucket_const(&src, expected.buckets_expr).unwrap_or_else(|| {
            panic!(
                "daemon-metrics-eval: src constant {} missing",
                expected.buckets_expr
            )
        });
        assert_eq!(
            got.as_slice(),
            expected_values,
            "daemon-metrics-eval: src constant {} values mismatch",
            expected.buckets_expr
        );
    }
}

#[test]
fn daemon_metrics_reference_doc_rows_match_retired_gate_policy() {
    assert!(
        repo_path_exists(METRICS_DOC),
        "daemon-metrics-eval: missing required file: {METRICS_DOC}"
    );
    let doc = read_repo_file(METRICS_DOC);
    let bucket_re =
        Regex::new(r"\*\*Buckets \(seconds\):\*\* `([^`]+)`").expect("valid buckets regex");

    for expected in &EXPECTED_METRICS[..RETIRED_DOC_METRIC_COUNT] {
        let section = doc_section(&doc, expected.name).unwrap_or_else(|| {
            panic!(
                "daemon-metrics-eval: doc missing section for metric: {}",
                expected.name
            )
        });
        let want_kind = expected.kind.to_ascii_lowercase();
        assert!(
            section.contains(&format!("**Type:** {want_kind}")),
            "daemon-metrics-eval: doc {} type line missing or wrong (expected {want_kind})",
            expected.name
        );
        if expected.labels.is_empty() {
            assert!(
                section.contains("**Labels:** *(none)*"),
                "daemon-metrics-eval: doc {} expected labels = *(none)*",
                expected.name
            );
        } else {
            let want = labels_doc_line(expected.labels);
            assert!(
                section.contains(&format!("**Labels:** {want}")),
                "daemon-metrics-eval: doc {} labels line missing or wrong (expected {want})",
                expected.name
            );
        }

        if let Some(expected_values) = expected.bucket_values {
            let caps = bucket_re.captures(section).unwrap_or_else(|| {
                panic!(
                    "daemon-metrics-eval: doc {} missing buckets line",
                    expected.name
                )
            });
            let parsed: Vec<f64> = caps[1]
                .split(',')
                .map(|tok| {
                    tok.trim().parse::<f64>().unwrap_or_else(|err| {
                        panic!(
                            "daemon-metrics-eval: doc {} bucket token is not f64: {err}",
                            expected.name
                        )
                    })
                })
                .collect();
            assert_eq!(
                parsed.as_slice(),
                expected_values,
                "daemon-metrics-eval: doc {} buckets line mismatch",
                expected.name
            );
        }
    }
}
