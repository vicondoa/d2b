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

use d2b_contract_tests::{read_repo_file, repo_root};
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
    let provenance_field = Regex::new(
        r"(?mi)^-\s+\*\*(AI|agent|assistant|tool|model|provider|run([ _-]?id)?)\b[^*]*\*\*",
    )
    .expect("valid PR provenance-field regex");
    if provenance_field.is_match(template) {
        violations.push("PR template must not request tool/model/run provenance fields".to_owned());
    }
    for (required, message) in [
        (
            "Do not include AI agent, assistant, or model metadata",
            "PR template must ban AI/agent/model metadata",
        ),
        (
            "Do not paste raw evidence",
            "PR template must keep evidence payloads external",
        ),
        (
            "Open or update the PR after focused preflight",
            "PR template must open the PR before final delivery lanes",
        ),
        (
            "panel lanes may be pending while the PR is open",
            "PR template must permit concurrent final lanes on an open PR",
        ),
        (
            "`make test-integration` passes in the final validator lane",
            "PR template must keep test-integration in the final validator lane",
        ),
        (
            "`make test-host-integration` passes in the final validator lane",
            "PR template must keep test-host-integration in the final validator lane",
        ),
        (
            "Delivery `candidate_id`",
            "PR template must summarize the delivery candidate_id",
        ),
        (
            "Delivery `content_id`",
            "PR template must summarize the delivery content_id",
        ),
        (
            "tree-bound wave seal passed before",
            "PR template must require a tree-bound seal before merge",
        ),
    ] {
        if !template.contains(required) {
            violations.push(message.to_owned());
        }
    }
    if template.matches(PURE_POLICY_NA).count() < 2 {
        violations.push(
            "PR template must provide N/A escape hatches for both final host gates".to_owned(),
        );
    }
    violations
}

#[test]
fn pr_template_carries_external_parallel_delivery_contract() {
    let template = read_repo_file(PR_TEMPLATE);
    let violations = pr_template_violations(&template);
    assert!(
        violations.is_empty(),
        "PR template ratchet violations:\n{}",
        violations.join("\n")
    );
}

fn valid_pr_template_fixture() -> String {
    format!(
        "Do not include AI agent, assistant, or model metadata\n\
         Do not paste raw evidence\n\
         Open or update the PR after focused preflight\n\
         Final CI, validator, and panel lanes may be pending while the PR is open\n\
         Delivery `candidate_id`\n\
         Delivery `content_id`\n\
         - [ ] **`make test-integration` passes in the final validator lane**\n\
         - [ ] **`make test-host-integration` passes in the final validator lane**\n\
         tree-bound wave seal passed before\n\
         {PURE_POLICY_NA}\n\
         {PURE_POLICY_NA}\n"
    )
}

#[test]
fn pr_template_ratchet_negative_fixtures_fail_closed() {
    let valid = valid_pr_template_fixture();
    assert!(
        pr_template_violations(&valid).is_empty(),
        "complete parallel-delivery fixture must pass"
    );

    let missing_metadata_ban =
        valid.replace("Do not include AI agent, assistant, or model metadata", "");
    assert!(
        pr_template_violations(&missing_metadata_ban)
            .iter()
            .any(|violation| violation.contains("metadata")),
        "negative fixture without AI/model metadata ban must fail closed"
    );

    let missing_na = valid.replacen(PURE_POLICY_NA, "", 1);
    assert!(
        pr_template_violations(&missing_na)
            .iter()
            .any(|violation| violation.contains("N/A escape hatches")),
        "negative fixture without host-gate N/A hatches must fail closed"
    );

    let old_pre_pr = valid
        .replace(
            "`make test-integration` passes in the final validator lane",
            "`make test-integration` passes on the host before PR creation",
        )
        .replace(
            "`make test-host-integration` passes in the final validator lane",
            "`make test-host-integration` passes on the host before PR creation",
        );
    assert!(
        pr_template_violations(&old_pre_pr)
            .iter()
            .any(|violation| violation.contains("final validator lane")),
        "obsolete pre-PR host-gate wording must fail closed"
    );

    let requested_model = format!("- **Model:** gemini\n{valid}");
    assert!(
        pr_template_violations(&requested_model)
            .iter()
            .any(|violation| violation.contains("provenance fields")),
        "PR template model/provenance fields must fail closed"
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
    for captures in labels_re.captures_iter(content) {
        let matched = captures.get(0).expect("labels match");
        let line_number = content[..matched.start()].lines().count() + 1;
        let labels = captures.name("labels").expect("labels capture").as_str();
        for label in string_re
            .captures_iter(labels)
            .filter_map(|capture| capture.get(1).map(|matched| matched.as_str()))
        {
            if forbidden.contains(&label) || label.ends_with("_pid") || label.ends_with("_fd") {
                violations.push(format!(
                    "{rel}:{}: metric label `{label}` is high-cardinality or sensitive",
                    line_number
                ));
            }
        }
    }
    violations
}

fn noisy_pid_log_violations(rel: &str, content: &str) -> Vec<String> {
    let noisy_log_start =
        Regex::new(r#"tracing::(trace|debug|warn)!\("#).expect("valid noisy tracing macro regex");
    let pid_or_tid =
        Regex::new(r#"(?m)(\bpid\b|pid\s*=|\btid\b|tid\s*=)"#).expect("valid pid/tid regex");
    let quoted_string = Regex::new(r#"(?s)"([^"\\]|\\.)*""#).expect("valid string literal regex");
    let lifecycle_allowlist = [
        "startup adoption quarantined runner snapshot",
        "startup adoption could not reopen pidfd; leaving snapshot on disk",
        "startup adoption dropped runner snapshot after pidfd reopen race",
        "startup adoption quarantined runner snapshot with unparseable proc stat",
        "spawn registration failed; signaled unregistered runner by pidfd",
        "spawn registration failed; direct pidfd signal failed, falling back to broker",
        "spawn registration failed; broker signaled unregistered runner",
        "spawn registration failed; broker cleanup signal returned unexpected response",
        "spawn registration failed; broker cleanup signal failed",
        "spawn registration failed; could not duplicate pidfd for cleanup",
        "spawn registration failed; SIGTERM cleanup did not reap runner, escalating",
        "spawn registration failed; runner was not observed reaped after SIGKILL, leaving broker pidfd registered",
    ];

    let mut violations = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if !noisy_log_start.is_match(line) {
            index += 1;
            continue;
        }

        let start_index = index;
        let mut body = String::new();
        loop {
            body.push_str(lines[index]);
            body.push('\n');
            if lines[index].contains(");") || index + 1 == lines.len() {
                break;
            }
            index += 1;
        }

        let body_without_strings = quoted_string.replace_all(&body, "\"\"");
        if pid_or_tid.is_match(&body_without_strings)
            && !lifecycle_allowlist
                .iter()
                .any(|message| body.contains(message))
        {
            violations.push(format!(
                "{rel}:{}: noisy tracing log carries PID/TID data: {}",
                start_index + 1,
                line.trim()
            ));
        }
        index += 1;
    }
    violations
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
    let multiline_metric_fixture =
        "MetricDescriptor {\n    labels: &[\n        \"span_id\",\n        \"vm\",\n    ],\n}";
    assert!(
        metric_label_violations("fixture.rs", multiline_metric_fixture)
            .iter()
            .any(|violation| violation.contains("span_id")),
        "metric-label multiline negative fixture must reject span_id labels"
    );

    let log_fixture = r#"tracing::debug!(pid = child_pid, "polling child");"#;
    assert!(
        !noisy_pid_log_violations("fixture.rs", log_fixture).is_empty(),
        "noisy PID log negative fixture must fail closed"
    );
    let multiline_log_fixture = "tracing::warn!(\n    pid = child_pid,\n    \"polling child\"\n);";
    assert!(
        !noisy_pid_log_violations("fixture.rs", multiline_log_fixture).is_empty(),
        "noisy PID multiline log negative fixture must fail closed"
    );
    let lifecycle_log_fixture = r#"tracing::warn!(
        vm = %vm,
        role = %role_id,
        pid = response.pid,
        "spawn registration failed; signaled unregistered runner by pidfd"
    );"#;
    assert!(
        noisy_pid_log_violations("fixture.rs", lifecycle_log_fixture).is_empty(),
        "structured lifecycle PID diagnostic fixture must stay allowed"
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
