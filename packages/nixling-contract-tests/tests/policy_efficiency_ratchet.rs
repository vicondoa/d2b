#![forbid(unsafe_code)]
#![deny(warnings)]

//! ADR 0035 recurring efficiency ratchet policy.
//!
//! These checks intentionally live in the existing Rust policy-test layer rather
//! than a new shell gate. They scan only git-tracked files, so ignored build
//! outputs such as `packages/target/` and local scratch artifacts are never
//! considered.

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::{read_repo_file, repo_root};
use regex::Regex;

const PR_TEMPLATE: &str = ".github/PULL_REQUEST_TEMPLATE.md";
const PURE_POLICY_NA: &str = "N/A: pure policy/docs/checklist change with no daemon, broker, NixOS";

fn git_tracked_files(roots: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .arg("-c")
        .arg("core.quotePath=false")
        .args(["ls-files", "-z", "--"])
        .args(roots)
        .output()
        .expect("run `git ls-files -z`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut files = BTreeSet::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        files.insert(String::from_utf8(raw.to_vec()).expect("tracked paths are UTF-8"));
    }
    files.into_iter().collect()
}

fn read_tracked_text_file(rel: &str) -> Option<String> {
    let content = std::fs::read_to_string(repo_root().join(rel)).ok()?;
    if content.contains('\0') {
        return None;
    }
    Some(content)
}

fn pr_template_violations(template: &str) -> Vec<String> {
    let mut violations = Vec::new();
    if !template.contains("Do not include AI agent, assistant, or model metadata") {
        violations.push("PR template must ban AI/agent/model metadata".to_owned());
    }
    if !template.contains("`make test-integration` passes on the host before PR creation") {
        violations.push("PR template must keep the test-integration checklist item".to_owned());
    }
    if !template.contains("`make test-host-integration` passes on the host before PR creation") {
        violations
            .push("PR template must keep the test-host-integration checklist item".to_owned());
    }
    if template.matches(PURE_POLICY_NA).count() < 2 {
        violations.push(
            "PR template must provide N/A escape hatches for both host/manual gates".to_owned(),
        );
    }
    violations
}

#[test]
fn pr_template_carries_metadata_ban_and_host_gate_escape_hatches() {
    let template = read_repo_file(PR_TEMPLATE);
    let violations = pr_template_violations(&template);
    assert!(
        violations.is_empty(),
        "PR template ratchet violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn pr_template_ratchet_negative_fixtures_fail_closed() {
    let missing_metadata_ban = format!(
        "- [ ] **`make test-integration` passes on the host before PR creation**\n  {PURE_POLICY_NA}\n\
         - [ ] **`make test-host-integration` passes on the host before PR creation**\n  {PURE_POLICY_NA}\n"
    );
    assert!(
        pr_template_violations(&missing_metadata_ban)
            .iter()
            .any(|violation| violation.contains("metadata")),
        "negative fixture without AI/model metadata ban must fail closed"
    );

    let missing_na = "Do not include AI agent, assistant, or model metadata\n\
         - [ ] **`make test-integration` passes on the host before PR creation**\n\
         - [ ] **`make test-host-integration` passes on the host before PR creation**\n";
    assert!(
        pr_template_violations(missing_na)
            .iter()
            .any(|violation| violation.contains("N/A escape hatches")),
        "negative fixture without host-gate N/A hatches must fail closed"
    );
}

fn metric_label_violations(rel: &str, content: &str) -> Vec<String> {
    let labels_re =
        Regex::new(r#"labels\s*:\s*&\[(?P<labels>[^\]]*)\]"#).expect("valid metric labels regex");
    let string_re = Regex::new(r#""([^"]+)""#).expect("valid string regex");
    let forbidden = [
        "trace_id",
        "span_id",
        "request_id",
        "pid",
        "tid",
        "fd",
        "path",
        "argv",
        "env",
        "cmdline",
        "command",
    ];

    let mut violations = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        let Some(captures) = labels_re.captures(line) else {
            continue;
        };
        let labels = captures.name("labels").expect("labels capture").as_str();
        for label in string_re
            .captures_iter(labels)
            .filter_map(|capture| capture.get(1).map(|matched| matched.as_str()))
        {
            if forbidden.contains(&label) || label.ends_with("_pid") || label.ends_with("_fd") {
                violations.push(format!(
                    "{rel}:{}: metric label `{label}` is high-cardinality or sensitive",
                    line_index + 1
                ));
            }
        }
    }
    violations
}

fn noisy_pid_log_violations(rel: &str, content: &str) -> Vec<String> {
    let noisy_pid_log =
        Regex::new(r#"tracing::(trace|debug|warn)!\([^;\n]*(\bpid\b|pid\s*=|\btid\b|tid\s*=)"#)
            .expect("valid noisy pid log regex");
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| noisy_pid_log.is_match(line))
        .map(|(idx, line)| {
            format!(
                "{rel}:{}: noisy tracing log carries PID/TID data: {}",
                idx + 1,
                line.trim()
            )
        })
        .collect()
}

#[test]
fn metric_labels_and_noisy_logs_stay_low_cardinality() {
    let mut violations = Vec::new();
    for rel in git_tracked_files(&["packages"]) {
        if !rel.ends_with(".rs") || rel.contains("/tests/") || rel.contains("/generated/") {
            continue;
        }
        let Some(content) = read_tracked_text_file(&rel) else {
            continue;
        };
        violations.extend(metric_label_violations(&rel, &content));
        violations.extend(noisy_pid_log_violations(&rel, &content));
    }
    assert!(
        violations.is_empty(),
        "metric/log cardinality ratchet violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn metric_and_log_ratchet_negative_fixtures_fail_closed() {
    let metric_fixture = r#"MetricDescriptor { labels: &["trace_id", "pid"] }"#;
    assert!(
        metric_label_violations("fixture.rs", metric_fixture).len() >= 2,
        "metric-label negative fixture must reject trace_id and pid labels"
    );

    let log_fixture = r#"tracing::debug!(pid = child_pid, "polling child");"#;
    assert!(
        !noisy_pid_log_violations("fixture.rs", log_fixture).is_empty(),
        "noisy PID log negative fixture must fail closed"
    );
}

#[test]
fn unsafe_code_allowances_stay_narrow_and_local() {
    let mut violations = Vec::new();
    for rel in git_tracked_files(&["packages"]) {
        if !rel.ends_with(".rs") {
            continue;
        }
        let Some(content) = read_tracked_text_file(&rel) else {
            continue;
        };
        for (line_index, line) in content.lines().enumerate() {
            if line.trim() == "#![allow(unsafe_code)]" {
                violations.push(format!(
                    "{rel}:{}: file-wide unsafe_code allowance is forbidden; use a narrow item-level allowlist",
                    line_index + 1
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "unsafe-code allowance ratchet violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn unsafe_allowance_negative_fixture_fails_closed() {
    let fixture = "#![allow(unsafe_code)]\nfn main() {}\n";
    let violations: Vec<String> = fixture
        .lines()
        .enumerate()
        .filter(|(_, line)| line.trim() == "#![allow(unsafe_code)]")
        .map(|(idx, _)| format!("fixture.rs:{}", idx + 1))
        .collect();
    assert!(
        !violations.is_empty(),
        "file-wide unsafe allow negative fixture must fail closed"
    );
}
