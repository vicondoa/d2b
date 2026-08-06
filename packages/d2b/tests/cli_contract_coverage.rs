//! v3 CLI documentation/parser contract coverage.
//!
//! The former compatibility matrix read the retired `cli-contract.md` surface and
//! exercised commands that the v3 clean break deliberately removed. This
//! test keeps the coverage closed by pinning the published v3 replacement
//! reference, the complete ModernCli top-level registry, representative
//! modern argument/flag probes, and explicit rejection of the retired
//! namespaces. The host error-golden closure remains below.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use serde_json::Value;

const V3_TOP_LEVEL_COMMANDS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "delete",
    "status",
    "upgrade",
    "reconcile",
    "host",
    "guest",
    "process",
    "exec",
    "shell",
    "volume",
    "network",
    "device",
    "endpoint",
    "export",
    "import",
    "resource",
    "user",
    "credential",
    "provider",
    "zone",
    "quota",
    "emergency-policy",
    "activation",
    "audit",
    "op",
    "auth",
    "complete",
    "audio",
    "clipboard",
    "display",
];

const V3_GLOBAL_FLAGS: &[&str] = &[
    "--zone",
    "--json",
    "--human",
    "--deadline",
    "--no-deadline",
    "-V",
    "--version",
];

const V3_DOC_MARKERS: &[&str] = &[
    "# v3 replacement contracts for desktop clients",
    "**Contract version:** d2b 3.0 (v3) replacement surface.",
    "The client must use the `selectedVersion` and returned feature list",
    "d2b audio status --json",
    "d2b usb security-key status",
    "d2b usb security-key sessions",
    "d2b usb security-key cancel",
    "Persistent shell operations are admin-only.",
    "open a hidraw device",
    "It never contains",
    "private proxy paths",
    "must not open the private file directly",
    "must not read private d2b state",
    "normal `d2b vm stop --apply`",
    "Closed DTOs are decoded with the documented version",
];

/// Published examples use companion-facing nouns; these are the corresponding
/// ModernCli parser paths after the v3 clean break.
const V3_DOCUMENTED_REPLACEMENTS: &[(&str, &[&str])] = &[
    ("d2b audio status --json", &["audio", "status", "--json"]),
    (
        "d2b usb security-key status",
        &["device", "security-key", "status", "--json"],
    ),
    (
        "d2b usb security-key sessions",
        &["device", "security-key", "sessions", "--json"],
    ),
    (
        "d2b usb security-key cancel",
        &[
            "device",
            "security-key",
            "cancel",
            "--current",
            "--dry-run",
            "--json",
        ],
    ),
    (
        "normal `d2b vm stop --apply`",
        &["guest", "stop", "work", "--apply", "--json"],
    ),
];

/// One parser probe per modern namespace, with representative nested flags.
const V3_PARSER_PROBES: &[(&str, &[&str])] = &[
    (
        "generic get and global flags",
        &["get", "Guest/work", "--zone", "dev", "--json"],
    ),
    (
        "generic list filters",
        &[
            "list",
            "Guest",
            "--execution-ref",
            "Host/launcher",
            "--domain",
            "user",
            "--phase",
            "Ready",
            "--label-selector",
            "owner=alice",
            "--updates",
            "--page-token",
            "next",
            "--limit",
            "10",
            "--human",
        ],
    ),
    (
        "generic watch and no-deadline",
        &[
            "watch",
            "Guest",
            "--since-revision",
            "7",
            "--phase",
            "Ready",
            "--label-selector",
            "owner=alice",
            "--no-deadline",
            "--json",
        ],
    ),
    (
        "generic create",
        &[
            "create",
            "Guest",
            "--spec-file",
            "spec.json",
            "--wait-for-reconcile",
            "--reconcile-deadline",
            "5s",
            "--deadline",
            "30s",
            "--json",
        ],
    ),
    (
        "generic update-spec",
        &[
            "update-spec",
            "Guest/work",
            "--revision",
            "7",
            "--spec-file",
            "spec.json",
            "--wait-for-reconcile",
            "--reconcile-deadline",
            "5s",
            "--json",
        ],
    ),
    (
        "generic delete",
        &[
            "delete",
            "Guest/work",
            "--revision",
            "7",
            "--wait-for-reconcile",
            "--reconcile-deadline",
            "5s",
            "--json",
        ],
    ),
    (
        "generic status",
        &["status", "Guest/work", "--watch", "--json"],
    ),
    (
        "generic upgrade",
        &[
            "upgrade",
            "Guest/work",
            "--recursive",
            "--apply",
            "--reconcile-deadline",
            "5s",
            "--json",
        ],
    ),
    (
        "generic reconcile",
        &[
            "reconcile",
            "Guest/work",
            "--reconcile-deadline",
            "5s",
            "--json",
        ],
    ),
    (
        "host check",
        &["host", "check", "--read-only", "--strict", "--json"],
    ),
    ("host prepare", &["host", "prepare", "--dry-run", "--json"]),
    ("host destroy", &["host", "destroy", "--apply", "--json"]),
    ("host doctor", &["host", "doctor", "--read-only", "--json"]),
    (
        "host install",
        &[
            "host", "install", "--apply", "--enable", "--start", "--json",
        ],
    ),
    (
        "host reconcile",
        &["host", "reconcile", "--network", "--dry-run", "--json"],
    ),
    (
        "host validate",
        &[
            "host",
            "validate",
            "--dry-run",
            "--wave",
            "wave-5",
            "--evidence-dir",
            "evidence",
            "--scripts-dir",
            "scripts",
            "--operator-signature",
            "signature",
            "--json",
        ],
    ),
    ("guest get", &["guest", "get", "work", "--json"]),
    (
        "guest list",
        &[
            "guest",
            "list",
            "--phase",
            "Ready",
            "--updates",
            "--limit",
            "10",
            "--json",
        ],
    ),
    (
        "guest status",
        &["guest", "status", "work", "--watch", "--json"],
    ),
    (
        "guest start",
        &[
            "guest",
            "start",
            "work",
            "--apply",
            "--no-wait-ready",
            "--force",
            "--json",
        ],
    ),
    (
        "guest stop",
        &["guest", "stop", "work", "--apply", "--force", "--json"],
    ),
    (
        "guest restart",
        &["guest", "restart", "work", "--dry-run", "--json"],
    ),
    (
        "guest create",
        &["guest", "create", "--spec-file", "spec.json", "--json"],
    ),
    (
        "guest update-spec",
        &[
            "guest",
            "update-spec",
            "work",
            "--revision",
            "7",
            "--spec-file",
            "spec.json",
            "--json",
        ],
    ),
    (
        "guest delete",
        &["guest", "delete", "work", "--revision", "7", "--json"],
    ),
    ("guest console", &["guest", "console", "work", "--json"]),
    (
        "process list",
        &[
            "process",
            "list",
            "--execution-ref",
            "Guest/work",
            "--domain",
            "system",
            "--phase",
            "Ready",
            "--limit",
            "10",
            "--json",
        ],
    ),
    (
        "exec run",
        &[
            "exec",
            "run",
            "Guest/work",
            "--name",
            "run-1",
            "--domain",
            "user",
            "--user",
            "User/alice",
            "--provider",
            "Provider/shell",
            "--env",
            "KEY=VALUE",
            "--cwd",
            "/home/alice",
            "--",
            "echo",
            "ok",
            "--json",
        ],
    ),
    (
        "exec attach",
        &[
            "exec",
            "attach",
            "EphemeralProcess/run-1",
            "--interactive",
            "--tty",
            "--json",
        ],
    ),
    (
        "exec logs",
        &[
            "exec",
            "logs",
            "EphemeralProcess/run-1",
            "--stdout-offset",
            "4",
            "--stderr-offset",
            "8",
            "--max-len",
            "4096",
            "--json",
        ],
    ),
    (
        "shell open",
        &[
            "shell",
            "open",
            "Guest/work",
            "--name",
            "main",
            "--force",
            "--json",
        ],
    ),
    (
        "shell status",
        &["shell", "status", "ShellSession/main", "--watch", "--json"],
    ),
    (
        "typed volume verify",
        &["volume", "verify", "state", "--repair", "--json"],
    ),
    (
        "typed network list",
        &[
            "network",
            "list",
            "--domain",
            "system",
            "--phase",
            "Ready",
            "--label-selector",
            "owner=alice",
            "--updates",
            "--page-token",
            "next",
            "--limit",
            "10",
            "--json",
        ],
    ),
    (
        "device USB attach",
        &[
            "device",
            "usb",
            "attach",
            "work",
            "1-2",
            "--dry-run",
            "--json",
        ],
    ),
    (
        "device security key",
        &["device", "security-key", "status", "--json"],
    ),
    (
        "endpoint list",
        &[
            "endpoint",
            "list",
            "--endpoint-class",
            "display",
            "--updates",
            "--json",
        ],
    ),
    (
        "export list",
        &["export", "list", "--exported-type", "Credential", "--json"],
    ),
    ("import graph", &["import", "graph", "microphone", "--json"]),
    (
        "resource authorities",
        &[
            "resource",
            "authorities",
            "--scope",
            "system",
            "holders",
            "Guest/work",
            "--json",
        ],
    ),
    ("user get", &["user", "get", "alice", "--json"]),
    ("credential get", &["credential", "get", "token", "--json"]),
    (
        "provider list",
        &["provider", "list", "--package-only", "--json"],
    ),
    (
        "zone status",
        &["zone", "status", "dev", "--watch", "--json"],
    ),
    ("quota list", &["quota", "list", "--json"]),
    (
        "emergency policy status",
        &["emergency-policy", "status", "lockdown", "--json"],
    ),
    (
        "activation keys list",
        &["activation", "keys", "list", "--json"],
    ),
    ("audit", &["audit", "--strict", "--json"]),
    (
        "operation inspect",
        &[
            "op",
            "inspect",
            "--operation-id",
            "op-1",
            "--trace-id",
            "trace-1",
            "--span-id",
            "span-1",
            "--watch",
            "--json",
        ],
    ),
    (
        "auth hidden fixture seam",
        &["auth", "--test-uid", "1000", "status", "--json"],
    ),
    (
        "completion command list",
        &["complete", "--list-commands", "--json"],
    ),
    ("audio projection", &["audio", "status", "--json"]),
    ("clipboard projection", &["clipboard", "arm", "--json"]),
    ("display projection", &["display", "list", "--json"]),
];

const RETIRED_PARSER_PROBES: &[(&str, &[&str])] = &[
    ("retired vm namespace", &["vm", "start", "work"]),
    ("retired keys namespace", &["keys", "list"]),
    ("retired usb namespace", &["usb", "probe"]),
    ("retired realm namespace", &["realm", "list"]),
    ("retired up alias", &["up", "work"]),
    ("retired status VM flag", &["status", "--vm", "work"]),
    ("retired bridge flag", &["status", "--check-bridges"]),
    (
        "retired auth test-uid placement",
        &["auth", "status", "--test-uid", "1000"],
    ),
    ("retired list shape", &["list", "--json"]),
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
    ("host-check", "foreign-nft-rule-shadows-d2b"),
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
    let doc = read_zone_cli_contract();
    let mut violations = Vec::new();

    for marker in V3_DOC_MARKERS {
        if !doc.contains(marker) {
            violations.push(format!("zone-cli-contract.md is missing marker: {marker}"));
        }
    }

    for (documented, parser_args) in V3_DOCUMENTED_REPLACEMENTS {
        if !doc.contains(documented) {
            violations.push(format!(
                "zone-cli-contract.md is missing documented surface: {documented}"
            ));
        }
        if let Err(error) = clap_accepts(parser_args) {
            violations.push(format!(
                "ModernCli replacement for {documented} was rejected: {error}"
            ));
        }
    }

    let actual_commands: BTreeSet<String> = d2b::cli_command()
        .get_subcommands()
        .map(|command| command.get_name().to_owned())
        .collect();
    let expected_commands: BTreeSet<&str> = V3_TOP_LEVEL_COMMANDS.iter().copied().collect();
    let actual_commands_as_refs: BTreeSet<&str> =
        actual_commands.iter().map(String::as_str).collect();
    if actual_commands_as_refs != expected_commands {
        violations.push(format!(
            "ModernCli top-level command set drifted: actual {actual_commands:?}, expected {expected_commands:?}"
        ));
    }

    match render_clap_help("") {
        Ok(help) => {
            let actual_flags = parse_help_flags(&help);
            let expected_flags: BTreeSet<String> = V3_GLOBAL_FLAGS
                .iter()
                .map(|flag| (*flag).to_owned())
                .collect();
            if actual_flags != expected_flags {
                violations.push(format!(
                    "ModernCli global flag set drifted: actual {actual_flags:?}, expected {expected_flags:?}"
                ));
            }
        }
        Err(error) => violations.push(format!("ModernCli top-level help failed: {error}")),
    }

    for (label, args) in V3_PARSER_PROBES {
        if let Err(error) = clap_accepts(args) {
            violations.push(format!("modern parser probe {label} was rejected: {error}"));
        }
    }
    for (label, args) in RETIRED_PARSER_PROBES {
        if clap_accepts(args).is_ok() {
            violations.push(format!(
                "retired parser probe {label} unexpectedly parsed: d2b {}",
                args.join(" ")
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "v3 zone-cli contract/parser drift:\n{}",
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
        .expect("packages/d2b is two levels below the repository root")
        .to_path_buf()
}

fn read_zone_cli_contract() -> String {
    let path = repo_root().join("docs/reference/zone-cli-contract.md");
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn render_clap_help(command_path: &str) -> Result<String, String> {
    let mut argv = vec!["d2b"];
    argv.extend(command_path.split_whitespace());
    argv.push("--help");

    let mut command = d2b::cli_command();
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
        if let Some(rest) = token.strip_prefix("--")
            && !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Some(format!("--{rest}"));
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

fn clap_accepts(args: &[&str]) -> Result<(), String> {
    let mut argv = vec!["d2b"];
    argv.extend_from_slice(args);
    let mut command = d2b::cli_command();
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
