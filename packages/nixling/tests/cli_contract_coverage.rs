//! Rust successor for `tests/cli-contract-coverage.sh`.
//!
//! This is intentionally a documentation/parser contract test, not a runtime
//! CLI-output test. It keeps the retired shell gate's closed command/disposition
//! scope, compares the documented flag tables with clap-rendered help for the
//! selected parser surfaces, and preserves the host CLI error-golden closure.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use serde_json::Value;

const EXPECTED_DISPOSITION: &[(&str, &str)] = &[
    ("list", "rust-native"),
    ("vm start", "rust-native"),
    ("vm stop", "rust-native"),
    ("vm restart", "rust-native"),
    ("vm exec", "rust-native"),
    ("status", "rust-native"),
    ("status --check-bridges", "rust-native"),
    ("usb attach", "rust-native"),
    ("usb detach", "rust-native"),
    ("usb probe", "rust-native"),
    ("console", "rust-native shim"),
    ("audio status", "rust-native shim"),
    ("audio mic", "rust-native shim"),
    ("audio speaker", "rust-native shim"),
    ("audio off", "rust-native shim"),
    ("build", "rust-native"),
    ("switch", "rust-native"),
    ("boot", "rust-native"),
    ("test", "rust-native"),
    ("rollback", "rust-native"),
    ("generations", "rust-native"),
    ("gc", "rust-native"),
    ("store verify", "rust-native"),
    ("trust", "rust-native"),
    ("rotate-known-host", "rust-native"),
    ("keys list", "rust-native"),
    ("keys show", "rust-native"),
    ("keys rotate", "rust-native"),
    ("audit", "rust-native"),
    ("host check", "rust-native"),
    ("host prepare", "rust-native"),
    ("host destroy", "rust-native"),
    ("auth status", "rust-native"),
];

const HELP_GROUPS: &[(&str, &[&str])] = &[
    ("list", &["list"]),
    ("status", &["status", "status --check-bridges"]),
    ("vm exec", &["vm exec"]),
    ("keys list", &["keys list"]),
    ("keys show", &["keys show"]),
    ("usb attach", &["usb attach"]),
    ("usb detach", &["usb detach"]),
    ("usb probe", &["usb probe"]),
    ("store verify", &["store verify"]),
    ("audit", &["audit"]),
    ("host check", &["host check"]),
    ("auth status", &["auth status"]),
];

/// `(flag label, argv tokens)` for a single flag invocation under a command.
type FlagInvocation = (&'static str, &'static [&'static str]);
/// `(command label, [flag invocations])` — a command's flag-coverage matrix.
type CommandFlagMatrix = (&'static str, &'static [FlagInvocation]);

const FLAG_INVOCATIONS: &[CommandFlagMatrix] = &[
    (
        "list",
        &[
            ("--json", &["list", "--json"]),
            ("--human", &["list", "--human"]),
        ],
    ),
    (
        "status",
        &[
            ("--json", &["status", "--vm", "corp-vm", "--json"]),
            ("--human", &["status", "--human"]),
            ("--vm", &["status", "--vm", "corp-vm", "--human"]),
        ],
    ),
    (
        "status --check-bridges",
        &[("--check-bridges", &["status", "--check-bridges"])],
    ),
    (
        "vm exec",
        &[
            ("-d", &["vm", "exec", "-d", "corp-vm", "--", "sleep", "60"]),
            (
                "--detach",
                &["vm", "exec", "--detach", "corp-vm", "--", "sleep", "60"],
            ),
            ("-i", &["vm", "exec", "-i", "-t", "corp-vm", "--", "bash"]),
            (
                "--interactive",
                &[
                    "vm",
                    "exec",
                    "--interactive",
                    "--tty",
                    "corp-vm",
                    "--",
                    "bash",
                ],
            ),
            ("-t", &["vm", "exec", "-t", "corp-vm", "--", "bash"]),
            ("--tty", &["vm", "exec", "--tty", "corp-vm", "--", "bash"]),
            (
                "--env",
                &["vm", "exec", "--env", "KEY=VALUE", "corp-vm", "--", "env"],
            ),
            (
                "--cwd",
                &["vm", "exec", "--cwd", "/home/alice", "corp-vm", "--", "pwd"],
            ),
            (
                "--json",
                &["vm", "exec", "corp-vm", "logs", "exec-1", "--json"],
            ),
            (
                "--human",
                &["vm", "exec", "corp-vm", "logs", "exec-1", "--human"],
            ),
            (
                "--stdout-offset",
                &[
                    "vm",
                    "exec",
                    "corp-vm",
                    "logs",
                    "exec-1",
                    "--stdout-offset=4",
                ],
            ),
            (
                "--stderr-offset",
                &[
                    "vm",
                    "exec",
                    "corp-vm",
                    "logs",
                    "exec-1",
                    "--stderr-offset=8",
                ],
            ),
            (
                "--max-len",
                &["vm", "exec", "corp-vm", "logs", "exec-1", "--max-len=4096"],
            ),
        ],
    ),
    (
        "keys list",
        &[
            ("--json", &["keys", "list", "--json"]),
            ("--human", &["keys", "list", "--human"]),
        ],
    ),
    (
        "keys show",
        &[
            ("--json", &["keys", "show", "corp-vm", "--json"]),
            ("--human", &["keys", "show", "corp-vm", "--human"]),
        ],
    ),
    (
        "usb attach",
        &[
            (
                "--dry-run",
                &["usb", "attach", "corp-vm", "1-2", "--dry-run"],
            ),
            ("--apply", &["usb", "attach", "corp-vm", "1-2", "--apply"]),
            (
                "--json",
                &["usb", "attach", "corp-vm", "1-2", "--dry-run", "--json"],
            ),
            (
                "--human",
                &["usb", "attach", "corp-vm", "1-2", "--dry-run", "--human"],
            ),
        ],
    ),
    (
        "usb detach",
        &[
            (
                "--dry-run",
                &["usb", "detach", "corp-vm", "1-2", "--dry-run"],
            ),
            ("--apply", &["usb", "detach", "corp-vm", "1-2", "--apply"]),
            (
                "--json",
                &["usb", "detach", "corp-vm", "1-2", "--dry-run", "--json"],
            ),
            (
                "--human",
                &["usb", "detach", "corp-vm", "1-2", "--dry-run", "--human"],
            ),
        ],
    ),
    (
        "usb probe",
        &[
            ("--json", &["usb", "probe", "--json"]),
            ("--human", &["usb", "probe", "--human"]),
        ],
    ),
    (
        "store verify",
        &[
            (
                "--repair",
                &["store", "verify", "corp-vm", "--repair", "--json"],
            ),
            ("--json", &["store", "verify", "corp-vm", "--json"]),
            ("--human", &["store", "verify", "corp-vm", "--human"]),
        ],
    ),
    (
        "audit",
        &[
            ("--strict", &["audit", "--strict", "--json"]),
            ("--json", &["audit", "--json"]),
            ("--human", &["audit", "--human"]),
        ],
    ),
    (
        "host check",
        &[
            ("--read-only", &["host", "check", "--read-only", "--human"]),
            (
                "--strict",
                &["host", "check", "--strict", "--read-only", "--human"],
            ),
            ("--json", &["host", "check", "--read-only", "--json"]),
            ("--human", &["host", "check", "--read-only", "--human"]),
        ],
    ),
    (
        "auth status",
        &[
            (
                "--json",
                &["auth", "status", "--test-uid", "1000", "--json"],
            ),
            (
                "--human",
                &["auth", "status", "--test-uid", "1000", "--human"],
            ),
        ],
    ),
];

const W3_ROWS: &[(&str, &str)] = &[
    ("host-check", "cgroup-delegation-refused"),
    ("host-check", "cgroup-v2-unified-not-present"),
    ("host-check", "cgroup-controllers-missing"),
    ("host-check", "cgroup-kill-on-ancestor-refused"),
    ("host-check", "ifname-too-long"),
    ("host-check", "ifname-collision"),
    ("host-check", "ipv6-sysctl-drift"),
    ("host-check", "nm-managed-foreign-conflict"),
    ("host-check", "nm-reload-failed"),
    ("host-check", "foreign-nft-rule-shadows-nixling"),
    ("host-check", "firewall-coexistence-mismatch"),
    ("host-check", "host-modules-locked"),
    ("host-check", "modprobe-denied-not-in-matrix"),
    ("host-check", "minijail-too-old"),
    ("host-check", "ch-net-handoff-not-supported"),
    ("host-check", "runner-shape-drift"),
    ("host-check", "single-writer-conflict"),
    ("host-check", "tier-0-legacy-uses-nixos-module"),
    ("host-check", "host-lan-cidr-ambiguous"),
    ("host-prepare", "cgroup-delegation-refused"),
    ("host-prepare", "route-preflight-no-default-route"),
    ("host-prepare", "route-preflight-foreign-default-route"),
    ("host-prepare", "dnsmasq-not-bound"),
    ("host-prepare", "path-safety-violation"),
    ("host-prepare", "nm-reload-failed"),
    ("host-prepare", "bridge-port-flag-drift"),
    ("host-prepare", "nft-foreign-rule-flush-attempted"),
    ("host-prepare", "firewall-coexistence-mismatch"),
    ("host-prepare", "tier-0-legacy-uses-nixos-module"),
    ("host-prepare", "single-writer-conflict"),
    ("host-prepare", "legacy-no-prepare-apply"),
    ("host-destroy", "vm-still-running-refused"),
    ("host-destroy", "tier-0-legacy-uses-nixos-module"),
    ("host-destroy", "legacy-no-destroy-apply"),
    ("host-install", "not-yet-implemented"),
    ("host-check", "daemon-down"),
    ("host-check", "socket-perms-wrong"),
    ("host-check", "missing-group"),
    ("host-check", "unsupported-kernel"),
    ("host-check", "no-kvm"),
    ("host-check", "no-cgroup-v2"),
    ("host-check", "nftables-conflict"),
    ("host-check", "hardlink-fs-mismatch"),
    ("host-check", "manifest-skew"),
    ("host-check", "profile-rejects-root"),
    ("host-check", "seccomp-denial"),
    ("host-check", "tap-creation-denied"),
    ("host-check", "stale-lock"),
];

#[test]
fn cli_contract_sections_and_help_flags_match_documented_surface() {
    let doc = read_cli_contract();
    let sections = parse_sections(&doc);
    let dispatch_rows = parse_dispatch_rows(&doc);
    let mut violations = Vec::new();

    for (command, disposition) in EXPECTED_DISPOSITION {
        let Some(section) = sections.get(*command) else {
            violations.push(format!("missing section: {command}"));
            continue;
        };

        for label in [
            "**Synopsis:**",
            "**Flags**",
            "**Arguments**",
            "**Exit codes**",
            "**Human example**",
        ] {
            if !section.contains(label) {
                violations.push(format!("{command}: missing {label}"));
            }
        }

        if let Some(actual) = disposition_value(section, "**W2 disposition:**") {
            if actual != *disposition {
                violations.push(format!("{command}: disposition mismatch"));
            }
        } else if !["**Status**", "**Native**", "**Bash**"]
            .iter()
            .all(|label| section.contains(label))
        {
            violations.push(format!("{command}: missing disposition taxonomy labels"));
        }

        match dispatch_rows.get(*command) {
            Some(actual) if actual == disposition => {}
            Some(actual) => violations.push(format!(
                "dispatch table disposition mismatch for {command}: {actual}"
            )),
            None => violations.push(format!("dispatch table missing row: {command}")),
        }

        let exit_block = extract_block(section, "**Exit codes**", Some("**Human example**"));
        if !has_numeric_code_row(exit_block) {
            violations.push(format!("{command}: missing exit-code rows"));
        }
        if !has_fenced_block_after(section, "**Human example**", "text") {
            violations.push(format!("{command}: missing human example code fence"));
        }
        for schema_name in json_schema_names(command) {
            if !section.contains(schema_name) {
                violations.push(format!("{command}: missing schema link {schema_name}"));
            }
            if !has_fenced_block_after(section, "**`--json` example**", "json") {
                violations.push(format!("{command}: missing --json example block"));
            }
        }
    }

    for (help_group, grouped_commands) in HELP_GROUPS {
        let mut documented_flags = BTreeSet::new();
        for command in *grouped_commands {
            let Some(section) = sections.get(*command) else {
                violations.push(format!(
                    "{command}: missing section for help group {help_group}"
                ));
                continue;
            };
            documented_flags.extend(parse_doc_flags(section));
        }

        match render_clap_help(help_group) {
            Ok(output) => {
                if !output.contains("Usage:") {
                    violations.push(format!("{help_group}: --help did not render usage text"));
                    continue;
                }
                let mut actual_flags = parse_help_flags(&output);
                if *help_group == "vm exec" {
                    for token in ["--stdout-offset", "--stderr-offset", "--max-len"] {
                        if output.contains(token) {
                            actual_flags.insert(token.to_owned());
                        }
                    }
                }
                if actual_flags != documented_flags {
                    violations.push(format!(
                        "{help_group}: help flags {:?} != documented {:?}",
                        actual_flags, documented_flags
                    ));
                }
            }
            Err(err) => violations.push(format!("{help_group}: {err}")),
        }
    }

    for (_, grouped_commands) in HELP_GROUPS {
        for section_name in *grouped_commands {
            let Some(section) = sections.get(*section_name) else {
                continue;
            };
            for flag in parse_doc_flags(section) {
                let Some(args) = invocation_for(section_name, &flag) else {
                    violations.push(format!(
                        "{section_name}: no acceptance probe configured for {flag}"
                    ));
                    continue;
                };
                if let Err(err) = clap_accepts(args) {
                    violations.push(format!(
                        "{section_name}: documented flag {flag} was rejected by clap: {err}"
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "cli-contract coverage drift:\n{}",
        violations.join("\n")
    );
}

#[test]
fn host_cli_error_golden_table_is_closed_and_complete() {
    let golden_dir = repo_root().join("tests/golden/cli-output");
    let required_fields = BTreeSet::from([
        "kind",
        "code",
        "exit_code",
        "what_was_checked",
        "observed_state",
        "remediation",
        "docs_anchor",
    ]);
    let known: BTreeSet<String> = W3_ROWS
        .iter()
        .map(|(verb, code)| format!("{verb}-{code}"))
        .collect();
    let mut violations = Vec::new();

    for (verb, code) in W3_ROWS {
        let stem = format!("{verb}-{code}");
        let txt = golden_dir.join(format!("{stem}.txt"));
        let json = golden_dir.join(format!("{stem}.json"));
        if !txt.exists() {
            violations.push(format!("missing human golden: {}", display_repo_path(&txt)));
        }
        if !json.exists() {
            violations.push(format!("missing JSON golden: {}", display_repo_path(&json)));
            continue;
        }

        let raw = match fs::read_to_string(&json) {
            Ok(raw) => raw,
            Err(err) => {
                violations.push(format!("{}: read failed ({err})", json.display()));
                continue;
            }
        };
        let envelope: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(err) => {
                violations.push(format!("{}: invalid JSON ({err})", json.display()));
                continue;
            }
        };
        let Some(object) = envelope.as_object() else {
            violations.push(format!(
                "{}: JSON envelope is not an object",
                json.display()
            ));
            continue;
        };
        let present: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        let missing: Vec<&str> = required_fields.difference(&present).copied().collect();
        if !missing.is_empty() {
            violations.push(format!(
                "{}: JSON envelope missing required field(s): {:?}",
                json.file_name().unwrap().to_string_lossy(),
                missing
            ));
        }
        if object.get("code").and_then(Value::as_str) != Some(*code) {
            violations.push(format!(
                "{}: envelope `code` is {:?}, expected {code:?}",
                json.file_name().unwrap().to_string_lossy(),
                object.get("code")
            ));
        }
        let anchor = object
            .get("docs_anchor")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !valid_docs_anchor(anchor) {
            violations.push(format!(
                "{}: docs_anchor {anchor:?} does not match docs/reference/error-codes.md#<code>",
                json.file_name().unwrap().to_string_lossy()
            ));
        }
        if !object.get("exit_code").is_some_and(Value::is_i64) {
            violations.push(format!(
                "{}: exit_code must be an integer",
                json.file_name().unwrap().to_string_lossy()
            ));
        }
    }

    for entry in fs::read_dir(&golden_dir).expect("read tests/golden/cli-output") {
        let entry = entry.expect("read golden dir entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = host_golden_stem(name) else {
            continue;
        };
        if !known.contains(stem) {
            violations.push(format!(
                "orphan golden: {name} has no row in the W3 closed CLI error-code table"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "host CLI error golden drift:\n{}",
        violations.join("\n")
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("packages/nixling is two levels below the repository root")
        .to_path_buf()
}

fn read_cli_contract() -> String {
    let path = repo_root().join("docs/reference/cli-contract.md");
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn parse_sections(doc: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut body = String::new();

    for line in doc.lines() {
        if let Some(name) = heading_command(line) {
            if let Some(previous) = current.replace(name) {
                sections.insert(previous, std::mem::take(&mut body));
            }
            continue;
        }
        if line == "## Dispatch capability table" {
            break;
        }
        if current.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(previous) = current {
        sections.insert(previous, body);
    }
    sections
}

fn heading_command(line: &str) -> Option<String> {
    let rest = line.strip_prefix("### `")?;
    let end = rest.find('`')?;
    Some(rest[..end].to_owned())
}

fn parse_dispatch_rows(doc: &str) -> BTreeMap<String, String> {
    let Some((_, table)) = doc.split_once("## Dispatch capability table") else {
        return BTreeMap::new();
    };
    let mut rows = BTreeMap::new();
    for line in table.lines() {
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 2 {
            continue;
        }
        let Some(command) = single_backtick_value(cells[0]) else {
            continue;
        };
        let Some(disposition) = single_backtick_value(cells[1]) else {
            continue;
        };
        rows.insert(command, disposition);
    }
    rows
}

fn single_backtick_value(cell: &str) -> Option<String> {
    let start = cell.find('`')? + 1;
    let end = cell[start..].find('`')? + start;
    Some(cell[start..end].to_owned())
}

fn disposition_value(section: &str, label: &str) -> Option<String> {
    let after = section.split_once(label)?.1;
    single_backtick_value(after)
}

fn extract_block<'a>(section: &'a str, start_label: &str, end_label: Option<&str>) -> &'a str {
    let Some((_, after)) = section.split_once(start_label) else {
        return "";
    };
    match end_label.and_then(|label| after.split_once(label).map(|(block, _)| block)) {
        Some(block) => block,
        None => after,
    }
}

fn parse_doc_flags(section: &str) -> BTreeSet<String> {
    let flags_block = extract_block(section, "**Flags**", Some("**Arguments**"));
    let mut documented = BTreeSet::new();
    for line in flags_block.lines() {
        let stripped = line.trim();
        if !stripped.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = stripped
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(first) = cells.first() else {
            continue;
        };
        if *first == "_(none)_" {
            continue;
        }
        for token in backtick_values(first) {
            if token.starts_with('-') {
                documented.insert(token);
            }
        }
    }
    documented
}

fn backtick_values(cell: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = cell;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        values.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    values
}

fn has_numeric_code_row(block: &str) -> bool {
    block.lines().any(|line| {
        if !line.trim().starts_with('|') {
            return false;
        }
        backtick_values(line)
            .iter()
            .any(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
    })
}

fn has_fenced_block_after(section: &str, label: &str, language: &str) -> bool {
    let Some((_, after)) = section.split_once(label) else {
        return false;
    };
    let fence = format!("```{language}\n");
    let Some((_, after_fence)) = after.split_once(&fence) else {
        return false;
    };
    after_fence.contains("\n```")
}

fn json_schema_names(command: &str) -> &'static [&'static str] {
    match command {
        "list" => &["list.schema.json"],
        "status" => &["status.schema.json"],
        "audit" => &["audit.schema.json"],
        "host check" => &["host-check.schema.json"],
        "auth status" => &["auth-status.schema.json"],
        "store verify" => &["store-verify.schema.json"],
        "vm exec" => &[
            "vm-exec-create.schema.json",
            "vm-exec-list.schema.json",
            "vm-exec-status.schema.json",
            "vm-exec-logs.schema.json",
            "vm-exec-kill.schema.json",
        ],
        _ => &[],
    }
}

fn render_clap_help(command_path: &str) -> Result<String, String> {
    let mut argv = vec!["nixling"];
    argv.extend(command_path.split_whitespace());
    argv.push("--help");

    let mut command = nixling::cli_command();
    match command.try_get_matches_from_mut(argv) {
        Ok(_) => Err("help invocation parsed instead of rendering help".to_owned()),
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) =>
        {
            Ok(err.to_string())
        }
        Err(err) => {
            let rendered = err.to_string();
            if rendered.contains("Usage:") {
                Ok(rendered)
            } else {
                Err(format!("--help did not render usage text: {rendered}"))
            }
        }
    }
}

fn parse_help_flags(output: &str) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let mut in_options = false;
    for line in output.lines() {
        let stripped = line.trim();
        if stripped == "Options:" {
            in_options = true;
            continue;
        }
        if !in_options {
            continue;
        }
        if stripped.ends_with(':') && stripped != "Options:" && !stripped.starts_with('-') {
            break;
        }
        for token in flag_tokens(line) {
            if token != "-h" && token != "--help" {
                flags.insert(token);
            }
        }
    }
    flags
}

fn flag_tokens(line: &str) -> Vec<String> {
    line.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '[' | ']' | '(' | ')' | '`' | '<' | '>' | '=')
    })
    .filter_map(|raw| {
        let token = raw.trim_matches(|c: char| matches!(c, '.' | ':' | ';' | '|'));
        if let Some(rest) = token.strip_prefix("--") {
            if !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Some(format!("--{rest}"));
            }
        }
        if token.len() == 2 {
            let mut chars = token.chars();
            if chars.next() == Some('-') && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            {
                return Some(token.to_owned());
            }
        }
        None
    })
    .collect()
}

fn invocation_for(section_name: &str, flag: &str) -> Option<&'static [&'static str]> {
    FLAG_INVOCATIONS
        .iter()
        .find(|(section, _)| *section == section_name)?
        .1
        .iter()
        .find(|(candidate, _)| *candidate == flag)
        .map(|(_, args)| *args)
}

fn clap_accepts(args: &[&str]) -> Result<(), String> {
    let mut argv = vec!["nixling"];
    argv.extend_from_slice(args);
    let mut command = nixling::cli_command();
    command
        .try_get_matches_from_mut(argv)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn valid_docs_anchor(anchor: &str) -> bool {
    let Some(slug) = anchor.strip_prefix("docs/reference/error-codes.md#") else {
        return false;
    };
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn host_golden_stem(name: &str) -> Option<&str> {
    let stem = name
        .strip_suffix(".txt")
        .or_else(|| name.strip_suffix(".json"))?;
    [
        "host-check-",
        "host-prepare-",
        "host-destroy-",
        "host-install-",
    ]
    .iter()
    .any(|prefix| stem.starts_with(prefix))
    .then_some(stem)
}

fn display_repo_path(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}
